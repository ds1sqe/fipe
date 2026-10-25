//! A session: a sequence of lines, each checked against the lines
//! accepted before it.
//!
//! An accepted line is frozen in the session's frame; a later definition
//! of a name shadows it for the lines after, never changing it.

use crate::{
    ast::{Entry, ItemKind, Module},
    check::{local, within},
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    frame::Frame,
    infer,
    lex::{lex, Lexed},
    parse::{self, parse},
    resolve::{self, Resolved},
    token::Span,
    typed::Typed,
    types::Subst,
};
use alloc::{string::String, vec::Vec};

type Lowered = (
    crate::ir::Unit,
    Vec<crate::check::Settled>,
    Option<crate::check::lower::Journal>,
);

/// Accepted lines are joined by a blank line in the session's source,
/// the text their spans are in.
mod docs;

const SEPARATOR: &str = "\n\n";

/// The prelude, the host's declarations, the accepted lines and the file
/// modules loaded through [`Request::File`].
#[derive(Debug)]
pub struct Session {
    declarations: Declarations,
    source: String,
    /// Each accepted line's text and where it starts in `source`.
    lines: Vec<(String, usize)>,
    frame: Frame,
    files: Vec<(String, Module)>,
    absent: Vec<String>,
    pending: Option<Pending>,
    strict: bool,
    compile_budget: u64,
    index: Option<crate::check::lower::Index>,
    /// The entries the compile phase proved, by accepted line.
    settled: Vec<Vec<crate::check::Settled>>,
    /// The declared expressions and values typed so far, shared by
    /// every later line: a declaration module, an accepted line or a
    /// loaded file never changes within a session.
    declared: infer::files::FileCache,
    /// How many lines the session has accepted, withdrawn ones included:
    /// the serial of the next [`Accepted`].
    accepted: u64,
    /// What undoes the last accepted line, until another is accepted or
    /// it is withdrawn.
    last: Option<Last>,
    /// The names withdrawn lines bound, kept only to word a later
    /// "unknown name".
    withdrawn: alloc::collections::BTreeSet<String>,
}

/// A line the session accepted, which the host withdraws with
/// [`Session::withdraw`] when its run Faults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    serial: u64,
}

/// What a withdrawn line had added to the tables: the values and module
/// inits a machine forgets with [`crate::vm::Machine::withdraw`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Withdrawn {
    pub(crate) definitions: usize,
    pub(crate) modules: usize,
    pub(crate) tests: usize,
}

/// The last accepted line's undo.
#[derive(Clone, Debug)]
struct Last {
    serial: u64,
    frame: crate::frame::Undo,
    /// The index's journal; `None` when the line had no unit.
    journal: Option<crate::check::lower::Journal>,
    /// The files the line loaded.
    loaded: Vec<String>,
    /// The absent stems before the line.
    absent: Vec<String>,
}

impl Clone for Session {
    fn clone(&self) -> Self {
        let index = self.index.as_ref().map(crate::check::lower::Index::fork);
        Self {
            declarations: self.declarations.clone(),
            source: self.source.clone(),
            lines: self.lines.clone(),
            frame: self.frame.clone(),
            files: self.files.clone(),
            absent: self.absent.clone(),
            pending: self.pending.clone(),
            strict: self.strict,
            compile_budget: self.compile_budget,
            index,
            settled: self.settled.clone(),
            declared: self.declared.inherit(),
            accepted: self.accepted,
            last: self.last.clone(),
            withdrawn: self.withdrawn.clone(),
        }
    }
}

/// A line waiting for a [`Reply`].
#[derive(Clone, Debug)]
struct Pending {
    text: String,
    accept: bool,
    /// The stem of the outstanding [`Request::File`].
    waiting: Option<String>,
    /// Stems answered as absent or unparsable, for this line only.
    absent: Vec<String>,
    /// Syntax errors of replied files.
    errors: Vec<Diagnostic>,
    /// Stems of the files this line loaded, checked once it resolves.
    loaded: Vec<String>,
    /// The session's absent stems when the line began.
    absent_before: Vec<String>,
}

/// What one line produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Non-fatal diagnostics in the submitted line's source coordinates.
    pub warnings: Vec<crate::typed::Warning>,
    /// The line, parsed; `None` when it does not parse. Its spans are in
    /// the session's source followed by the line.
    pub module: Option<Module>,
    /// Where the line starts in the combined text.
    pub offset: usize,
    /// The diagnostics of the files the line loaded, then the line's
    /// own; the line checks exactly when it has none of its own.
    pub diagnostics: Vec<Diagnostic>,
    /// Present when the combined module resolved.
    pub resolved: Option<Resolved>,
    /// Present when the combined module resolved and typed.
    pub typed: Option<Typed>,
    /// Open entries left by this line.
    pub checks: Vec<crate::typed::Check>,
    /// Entries of this line the compile phase proved, with their values.
    pub settled: Vec<crate::check::Settled>,
    /// Indexed backend input, present only for a diagnostic-free line.
    pub unit: Option<crate::ir::Unit>,
    /// Present when the line was accepted.
    pub accepted: Option<Accepted>,
}

impl Report {
    fn syntax(errors: Vec<parse::Error>, offset: usize) -> Self {
        let diagnostics = errors
            .into_iter()
            .map(|error| {
                Diagnostic::Syntax(parse::Error {
                    span: local(error.span, offset),
                    ..error
                })
            })
            .collect();
        Self {
            warnings: Vec::new(),
            module: None,
            offset,
            diagnostics,
            resolved: None,
            typed: None,
            checks: Vec::new(),
            settled: Vec::new(),
            unit: None,
            accepted: None,
        }
    }

    /// The line's own entries of [`Report::module`].
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        let Some(module) = &self.module else {
            return &[];
        };
        let first = module.entries.partition_point(|entry| {
            let start = match entry {
                Entry::Item(item) => item.span.start,
                Entry::Line(id) => module.expr(*id).map_or(0, |e| e.span.start),
            };
            start < self.offset
        });
        &module.entries[first..]
    }
}

pub use crate::check::Error as Diagnostic;

impl Session {
    #[must_use]
    pub const fn new(declarations: Declarations) -> Self {
        Self {
            declarations,
            source: String::new(),
            lines: Vec::new(),
            frame: Frame {
                lines: Vec::new(),
                values: alloc::collections::BTreeMap::new(),
                types: alloc::collections::BTreeMap::new(),
                locals: alloc::collections::BTreeMap::new(),
                facts: alloc::collections::BTreeMap::new(),
                impls: alloc::collections::BTreeMap::new(),
                members: alloc::collections::BTreeMap::new(),
                constructors: alloc::collections::BTreeMap::new(),
                weak: Subst::from_bindings(Vec::new()),
            },
            files: Vec::new(),
            absent: Vec::new(),
            pending: None,
            strict: false,
            compile_budget: crate::check::COMPILE_BUDGET,
            index: None,
            settled: Vec::new(),
            declared: infer::files::FileCache::new(),
            accepted: 0,
            last: None,
            withdrawn: alloc::collections::BTreeSet::new(),
        }
    }

    /// Reject open implicit entries in subsequent units; explicit gates
    /// remain allowed. Accepted lines retain their earlier checks.
    pub const fn strict(&mut self, enabled: bool) {
        self.strict = enabled;
    }

    /// The charge units each compile-time evaluation of subsequent units
    /// may spend; exhausted means open.
    pub const fn compile_budget(&mut self, units: u64) {
        self.compile_budget = units;
    }

    /// Open checks of accepted lines, or of the latest binding named `name`.
    #[must_use]
    pub fn checks(&self, name: Option<&str>) -> Vec<crate::typed::Check> {
        self.select(name, |line| &self.frame.lines[line].typed.checks, |c| c)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Entries the compile phase proved in accepted lines, or in the
    /// latest binding named `name`.
    #[must_use]
    pub fn settled(&self, name: Option<&str>) -> Vec<crate::check::Settled> {
        self.select(name, |line| &self.settled[line], |s| &s.check)
            .into_iter()
            .cloned()
            .collect()
    }

    /// The items of accepted lines whose check is of the latest binding
    /// named `name`, or every item, once per place and limit.
    fn select<'a, T>(
        &'a self,
        name: Option<&str>,
        items: impl Fn(usize) -> &'a [T],
        check_of: fn(&T) -> &crate::typed::Check,
    ) -> Vec<&'a T> {
        let local_site = name
            .and_then(|name| self.frame.values.get(name))
            .and_then(|target| {
                if let resolve::Target::Local { site } = target {
                    Some(*site)
                } else {
                    None
                }
            });
        let file_binding = name.and_then(|name| {
            let (head, member) = name.split_once('.')?;
            let resolve::Target::Module(origin @ resolve::Origin::File(stem)) =
                self.frame.values.get(head)?
            else {
                return None;
            };
            let (_, module) = self.files.iter().find(|(n, _)| n == stem)?;
            module.entries.iter().find_map(|entry| {
                let Entry::Line(id) = entry else { return None };
                let expr = module.expr(*id)?;
                let crate::ast::ExprKind::Bind { pattern, .. } = expr.kind
                else {
                    return None;
                };
                let crate::ast::PatternKind::Name(n) =
                    &module.pattern(pattern)?.kind
                else {
                    return None;
                };
                (n.segments.last().is_some_and(|n| n == member))
                    .then_some((origin, expr.span))
            })
        });
        let mut seen = alloc::collections::BTreeSet::new();
        self.frame
            .lines
            .iter()
            .enumerate()
            .flat_map(|(index, line)| {
                let span = local_site.and_then(|site| {
                    line.module.entries.iter().find_map(|entry| {
                        let Entry::Line(id) = entry else { return None };
                        let expr = line.module.expr(*id)?;
                        let crate::ast::ExprKind::Bind { pattern, .. } =
                            expr.kind
                        else {
                            return None;
                        };
                        (line.module.pattern(pattern)?.span == site)
                            .then_some(expr.span)
                    })
                });
                items(index).iter().filter(move |item| {
                    let check = check_of(item);
                    name.is_none()
                        || (check.place_origin == resolve::Origin::This
                            && span
                                .is_some_and(|span| within(check.place, span)))
                        || file_binding.is_some_and(|(origin, span)| {
                            &check.place_origin == origin
                                && within(check.place, span)
                        })
                })
            })
            .filter(|item| {
                let check = check_of(item);
                seen.insert((
                    check.place_origin.clone(),
                    check.place,
                    check.place_kind.clone(),
                    check.limit_origin.clone(),
                    check.limit,
                ))
            })
            .collect()
    }

    /// The accepted lines, each followed by a blank line: the text
    /// before a new line's [`Report::offset`].
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Accepted compilation units in order, preserving embedded blank lines.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|(text, _)| text.as_str())
    }

    /// Check a line and accept it when it has no diagnostic. A line still
    /// awaiting a reply is abandoned.
    pub fn line(&mut self, text: &str) -> Outcome<Report> {
        self.start(text, true)
    }

    /// Take back the line accepted last, after its run Faulted: the
    /// session is then as if the line had not been given. `None`, and no
    /// change, when another line was accepted after it or it was already
    /// withdrawn. What it returns tells a machine which values to forget.
    pub fn withdraw(&mut self, line: Accepted) -> Option<Withdrawn> {
        let last = self.last.take_if(|last| last.serial == line.serial)?;
        self.pending = None;
        self.withdrawn.extend(last.frame.names().map(String::from));
        self.frame.withdraw(last.frame);
        if let Some((_, start)) = self.lines.pop() {
            self.source.truncate(start);
        }
        self.settled.pop();
        self.files.retain(|(stem, _)| !last.loaded.contains(stem));
        self.absent = last.absent;
        let line = resolve::Origin::Line(self.lines.len());
        self.declared.forget(|origin| {
            *origin == line
                || matches!(origin, resolve::Origin::File(stem)
                    if last.loaded.contains(stem))
        });
        // A line with no unit was never emitted, so nothing ran.
        let (definitions, modules, tests) =
            match (last.journal, self.index.as_mut()) {
                (Some(journal), Some(index)) => {
                    let (definitions, modules) = journal.before();
                    let tests = journal.tests_before();
                    index.withdraw(journal);
                    (definitions, modules, tests)
                }
                _ => (usize::MAX, usize::MAX, usize::MAX),
            };
        Some(Withdrawn {
            definitions,
            modules,
            tests,
        })
    }

    /// Whether a withdrawn line bound `name`: a later line naming it
    /// gets the ordinary "unknown name" (or "unknown type"), which a
    /// host may word with this.
    #[must_use]
    pub fn withdrawn(&self, name: &str) -> bool {
        self.withdrawn.contains(name)
    }

    /// Check a line without accepting it. An expression line is
    /// generalized, not defaulted: its scheme is the line's
    /// [`Typed::binding`].
    pub fn query(&mut self, text: &str) -> Outcome<Report> {
        self.start(text, false)
    }

    /// Answer the last request and resume the line that made it.
    ///
    /// # Panics
    /// If no line awaits a reply.
    pub fn reply(&mut self, reply: Reply) -> Outcome<Report> {
        let mut pending = self.pending.take().expect("no line awaits a reply");
        let stem = pending.waiting.take().expect("no request is pending");
        match reply {
            Reply::File(Some(text)) => match parse(lex(&text)) {
                Ok(module) => {
                    self.absent.retain(|name| *name != stem);
                    self.files.push((stem.clone(), module));
                    pending.loaded.push(stem);
                }
                Err(errors) => {
                    pending.errors.extend(errors.into_iter().map(|error| {
                        Diagnostic::File {
                            stem: stem.clone(),
                            error,
                        }
                    }));
                    pending.absent.push(stem);
                }
            },
            Reply::File(None) => pending.absent.push(stem),
            Reply::Value(_) => panic!("a file request takes a file reply"),
        }
        for stem in &pending.absent {
            if !self.absent.contains(stem) {
                self.absent.push(stem.clone());
            }
        }
        self.step(pending)
    }

    fn start(&mut self, text: &str, accept: bool) -> Outcome<Report> {
        self.pending = None;
        self.step(Pending {
            text: String::from(text),
            accept,
            waiting: None,
            absent: Vec::new(),
            errors: Vec::new(),
            loaded: Vec::new(),
            absent_before: self.absent.clone(),
        })
    }

    fn step(&mut self, mut pending: Pending) -> Outcome<Report> {
        let offset = self.source.len();
        let module = match parse(shifted(lex(&pending.text), offset)) {
            Ok(module) => module,
            Err(errors) => {
                return Outcome::Done(Report::syntax(errors, offset))
            }
        };
        if let Some(stem) = self.missing(&module, &pending.absent) {
            pending.waiting = Some(stem.clone());
            self.pending = Some(pending);
            return Outcome::Request(Request::File { stem });
        }
        self.declared.end_unit();
        let cache = &self.declared;
        let mut diagnostics = core::mem::take(&mut pending.errors);
        for stem in &pending.loaded {
            diagnostics.extend(crate::check::file_errors(
                stem,
                &self.declarations,
                &self.files,
                cache,
            ));
        }
        let files = diagnostics.len();
        let checked = crate::check::stages(
            &module,
            &self.declarations,
            &self.files,
            Some(&self.frame),
            cache,
            crate::check::Options {
                strict: self.strict,
                compile_budget: self.compile_budget,
            },
            (if pending.accept { usize::MAX } else { offset }, offset),
        );
        diagnostics.extend(checked.diagnostics);
        let resolved = checked.resolved;
        let mut typed = checked.typed;
        let files_valid = cache.files.borrow().values().all(|file| {
            file.resolve_errors.is_empty() && file.type_errors.is_empty()
        });
        let mut settled = Vec::new();
        let mut journal = None;
        let unit = if diagnostics.is_empty() && files_valid {
            if let (Some(resolved), Some(typed)) = (&resolved, &mut typed) {
                match self.lower(
                    &module,
                    resolved,
                    typed,
                    offset,
                    pending.accept,
                ) {
                    Ok(Some((unit, proved, kept))) => {
                        settled = proved;
                        journal = kept;
                        Some(unit)
                    }
                    Ok(None) => None,
                    Err(errors) => {
                        diagnostics.extend(errors);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        let checks = typed
            .as_ref()
            .map_or_else(Vec::new, |typed| typed.checks.clone());
        let accepted = match (&resolved, &typed) {
            (Some(resolved), Some(typed))
                if pending.accept && diagnostics.len() == files =>
            {
                let checked = (&module, resolved, typed);
                Some(self.accept(pending, checked, settled.clone(), journal))
            }
            _ => None,
        };
        self.declared.end_unit();
        Outcome::Done(Report {
            warnings: report_warnings(typed.as_ref(), offset),
            module: Some(module),
            offset,
            diagnostics,
            resolved,
            typed,
            checks,
            settled,
            unit,
            accepted,
        })
    }

    /// Lower a checked line and run the compile phase over it.
    fn lower(
        &mut self,
        module: &Module,
        resolved: &Resolved,
        typed: &mut Typed,
        offset: usize,
        accept: bool,
    ) -> Result<Option<Lowered>, Vec<Diagnostic>> {
        // An accepted line is emitted into the session's index, journaled
        // so a refusal by the compile phase undoes it; a query owns a copy.
        let lines = self.frame.lines.len();
        let empty = crate::check::lower::Index::default();
        let reader = self.index.as_ref().unwrap_or(&empty);
        let Some(prepared) = reader.prepare(
            (module, resolve::Origin::Line(lines)),
            resolved.clone(),
            typed.clone(),
            &self.declarations,
            &self.files,
            &self.declared,
        ) else {
            return Ok(None);
        };
        let index = self.index.get_or_insert_with(Default::default);
        let mut query;
        let index = if accept {
            index.begin();
            index
        } else {
            query = index.fork();
            &mut query
        };
        index.session(lines, typed);
        let first = index.tables().nodes.len();
        let mut unit = index.emit_prepared(prepared);
        let judged =
            self.judge(&mut unit, first, (module, resolved), typed, offset);
        let mut journal = None;
        if accept {
            let index = self.index.as_mut().expect("the session's index");
            if judged.is_ok() {
                journal = Some(index.commit());
            } else {
                index.rollback();
            }
        }
        judged.map(|settled| Some((unit, settled, journal)))
    }

    /// Run the compile phase over an emitted line and apply what it
    /// decided to the line's typing; its check errors refuse the line.
    fn judge(
        &self,
        unit: &mut crate::ir::Unit,
        first: usize,
        (module, resolved): (&Module, &Resolved),
        typed: &mut Typed,
        offset: usize,
    ) -> Result<Vec<crate::check::Settled>, Vec<Diagnostic>> {
        let demands = crate::check::demands(module, typed);
        let phase = crate::check::evaluate::run(
            unit,
            first,
            self.compile_budget,
            &demands,
        )
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|error| relocated(error, offset))
                .collect::<Vec<_>>()
        })?;
        let settled = crate::check::settle(typed, module, &phase);
        let frame = &self.frame;
        let mut errors = crate::check::unsettled(
            typed,
            |origin, span| match origin {
                resolve::Origin::This => {
                    crate::check::timed_compile(module, resolved, span)
                }
                resolve::Origin::Line(line) => {
                    frame.lines.get(*line).is_some_and(|line| {
                        crate::check::timed_compile(
                            &line.module,
                            &line.resolved,
                            span,
                        )
                    })
                }
                _ => false,
            },
            offset,
        );
        errors.extend(crate::check::open_errors(
            &typed.checks,
            self.strict,
            offset,
            true,
        ));
        if errors.is_empty() {
            Ok(settled)
        } else {
            Err(errors)
        }
    }

    /// Keep a checked line, with what undoes it until the next one.
    fn accept(
        &mut self,
        pending: Pending,
        (module, resolved, typed): (&Module, &Resolved, &Typed),
        settled: Vec<crate::check::Settled>,
        journal: Option<crate::check::lower::Journal>,
    ) -> Accepted {
        let frame = self.frame.accept(module.clone(), resolved.clone(), typed);
        self.lines.push((pending.text.clone(), self.source.len()));
        self.source.push_str(&pending.text);
        self.source.push_str(SEPARATOR);
        self.settled.push(settled);
        let serial = self.accepted;
        self.accepted += 1;
        self.last = Some(Last {
            serial,
            frame,
            journal,
            loaded: pending.loaded,
            absent: pending.absent_before,
        });
        Accepted { serial }
    }

    /// The first `use`d module that is neither a host module, nor loaded,
    /// nor answered absent. A use written in this line retries its imports.
    fn missing(&self, module: &Module, absent: &[String]) -> Option<String> {
        let retry = self.imports(module);
        core::iter::once(module)
            .chain(self.files.iter().map(|(_, file)| file))
            .flat_map(|module| &module.entries)
            .filter_map(|entry| match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Use { names, .. } => Some(names),
                    _ => None,
                },
                Entry::Line(_) => None,
            })
            .flatten()
            .filter_map(|name| name.segments.first())
            .find(|stem| {
                self.declarations.get_module(stem).is_none()
                    && !self.files.iter().any(|(file, _)| file == *stem)
                    && !absent.contains(stem)
                    && (!self.absent.contains(stem) || retry.contains(*stem))
            })
            .cloned()
    }
    fn imports(&self, module: &Module) -> alloc::collections::BTreeSet<String> {
        let mut stems = alloc::collections::BTreeSet::new();
        let mut pending = alloc::vec![module];
        while let Some(module) = pending.pop() {
            for entry in &module.entries {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Use { names, .. } = &item.kind else {
                    continue;
                };
                for name in names {
                    let Some(stem) = name.segments.first() else {
                        continue;
                    };
                    if stems.insert(stem.clone()) {
                        if let Some((_, file)) =
                            self.files.iter().find(|(name, _)| name == stem)
                        {
                            pending.push(file);
                        }
                    }
                }
            }
        }
        stems
    }
}

/// A compile-phase diagnostic moved into the line's text.
fn relocated(error: crate::check::Error, offset: usize) -> Diagnostic {
    match error {
        Diagnostic::Compile { span, what, reason } => Diagnostic::Compile {
            span: local(span, offset),
            what,
            reason,
        },
        other => other,
    }
}

/// Tokens of a line placed at `offset` in the session's text, so every
/// site in the session is distinct.
fn shifted(mut lexed: Lexed, offset: usize) -> Lexed {
    let shift = |span: &mut Span| {
        span.start += offset;
        span.end += offset;
    };
    for comment in &mut lexed.comments {
        shift(comment);
    }
    for token in &mut lexed.tokens {
        shift(&mut token.span);
    }
    for error in &mut lexed.errors {
        shift(&mut error.span);
    }
    lexed
}

fn report_warnings(
    typed: Option<&Typed>,
    offset: usize,
) -> Vec<crate::typed::Warning> {
    typed.map_or_else(Vec::new, |typed| {
        typed
            .warnings
            .iter()
            .map(|warning| crate::typed::Warning {
                span: local(warning.span, offset),
                style: warning.style,
            })
            .collect()
    })
}

#[cfg(test)]
mod publication_materialization_proof {
    use super::*;
    use crate::{
        infer::publication_proof::{self, Checked, Failure, Refusal},
        value::{
            resource::{ResourceDomain, ResourceType},
            Value,
        },
        vm::Machine,
    };
    use alloc::{
        collections::{BTreeMap, BTreeSet},
        format,
        rc::Rc,
    };

    const STRING_VIEW: &str =
        "pub type resource View\nimpl View {\n name: (self: Self) -> String\n}";
    const INTEGER_VIEW: &str = "pub type resource View\nimpl View {\n name: (self: Self) -> Integer\n}";
    const UNMARKED: &str =
        "pub type View\nimpl View { name: (self: Self) -> String }";
    const GENERIC: &str = "pub type resource View<T>\nimpl View<T> { name: (self: Self) -> String }";
    const INVALID: &str = "pub type resource View\nimpl View { name: (self: Self) -> MissingType }";

    #[derive(Debug)]
    enum StageError {
        Duplicate,
        Declaration(Vec<crate::decl::Error>),
        Checked(Failure),
    }

    struct Candidate {
        declarations: Declarations,
        cache: infer::files::FileCache,
        checked: Checked,
    }

    struct Receipt {
        journal: crate::check::lower::Journal,
        declarations: Declarations,
        cache: infer::files::FileCache,
    }

    struct Fixture {
        session: Session,
        machine: Machine,
        tables: Rc<core::cell::RefCell<crate::ir::Tables>>,
        original: Value,
        witness: ResourceType,
    }

    type CachedExpressions =
        BTreeMap<(resolve::Origin, usize), crate::typed::DeclaredExpression>;

    struct Snapshot {
        source: String,
        lines: usize,
        frame: String,
        serial: u64,
        last: String,
        tables: crate::ir::Tables,
        globals: Vec<(usize, Value)>,
        origins: BTreeSet<resolve::Origin>,
        expressions: CachedExpressions,
        values: Vec<CacheEntry>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CacheValue {
        Checking,
        Failed,
        Typed(crate::types::Scheme),
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CacheEntry {
        origin: resolve::Origin,
        name: String,
        value: CacheValue,
    }

    fn cache_values(
        session: &Session,
        origins: &BTreeSet<resolve::Origin>,
    ) -> Vec<CacheEntry> {
        session
            .declared
            .values
            .borrow()
            .iter()
            .filter(|((origin, _), _)| origins.contains(origin))
            .map(|((origin, name), value)| {
                let value = match value {
                    infer::files::DeclaredValue::Checking => {
                        CacheValue::Checking
                    }
                    infer::files::DeclaredValue::Failed => CacheValue::Failed,
                    infer::files::DeclaredValue::Typed(scheme) => {
                        CacheValue::Typed(scheme.clone())
                    }
                };
                CacheEntry {
                    origin: origin.clone(),
                    name: name.clone(),
                    value,
                }
            })
            .collect()
    }

    fn cache_expressions(
        session: &Session,
        origins: &BTreeSet<resolve::Origin>,
    ) -> CachedExpressions {
        session
            .declared
            .expressions
            .borrow()
            .iter()
            .filter(|((origin, _), _)| origins.contains(origin))
            .map(|(key, expression)| (key.clone(), expression.clone()))
            .collect()
    }

    fn fixture() -> Fixture {
        let mut declarations = Declarations::new();
        declarations
            .module(
                "base",
                "pub type resource BaseJson\npub load: () -> BaseJson",
            )
            .unwrap();
        declarations
            .module("old", "pub identity: Integer -> Integer = value => value")
            .unwrap();
        let mut session = Session::new(declarations);
        let Outcome::Done(report) =
            session.line("use base old\nj = base.load ()\nold.identity 7")
        else {
            panic!("no file requests");
        };
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert!(report.accepted.is_some());
        let unit = report.unit.unwrap();
        let tables = Rc::clone(&unit.tables);
        assert!(tables.borrow().has_resources);
        let program = crate::compile::compile(&unit);
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("actual load");
        };
        assert_eq!(arguments, [Value::Unit]);
        {
            let tables = machine.program().unwrap().tables.borrow();
            let called = &tables.declarations[declaration];
            assert_eq!(called.label, "load");
            assert_eq!(tables.namespaces[called.namespace], "base");
        }
        let witness = machine.pending_resource_type().unwrap();
        let original = ResourceDomain::new()
            .issue(&witness, Rc::from(&b"publication-base-entry"[..]));
        assert_eq!(
            machine.reply(&program, Reply::Value(Ok(original.clone()))),
            Outcome::Done(Ok(Value::Int(7))),
        );
        assert!(machine.globals().any(|(_, value)| value == &original));
        let old = resolve::Origin::Host("old".into());
        assert!(session.declared.expressions.borrow().iter().any(
            |((origin, _), expression)| {
                origin == &old && expression.origin == old
            },
        ));
        assert!(matches!(
            session
                .declared
                .values
                .borrow()
                .get(&(old, "identity".into())),
            Some(infer::files::DeclaredValue::Typed(_)),
        ));
        Fixture {
            session,
            machine,
            tables,
            original,
            witness,
        }
    }

    fn snapshot(fixture: &Fixture) -> Snapshot {
        let session = &fixture.session;
        let origins = ["old", "base"]
            .into_iter()
            .map(|name| resolve::Origin::Host(name.into()))
            .collect();
        Snapshot {
            source: session.source.clone(),
            lines: session.lines.len(),
            frame: format!("{:?}", session.frame),
            serial: session.accepted,
            last: format!("{:?}", session.last),
            tables: fixture.tables.borrow().clone(),
            globals: fixture
                .machine
                .globals()
                .map(|(index, value)| (index, value.clone()))
                .collect(),
            expressions: cache_expressions(session, &origins),
            values: cache_values(session, &origins),
            origins,
        }
    }

    fn original_unchanged(fixture: &Fixture, before: &Snapshot) {
        let session = &fixture.session;
        assert_eq!(
            cache_expressions(session, &before.origins),
            before.expressions
        );
        assert_eq!(cache_values(session, &before.origins), before.values);
        assert_eq!(session.source, before.source);
        assert_eq!(session.lines.len(), before.lines);
        assert_eq!(format!("{:?}", session.frame), before.frame);
        assert_eq!(session.accepted, before.serial);
        assert_eq!(format!("{:?}", session.last), before.last);
        let index = session.index.as_ref().unwrap();
        assert!(core::ptr::eq(
            &raw const *index.tables(),
            &raw const *fixture.tables.borrow(),
        ));
        assert!(fixture.tables.borrow().has_resources);
        let globals: Vec<_> = fixture
            .machine
            .globals()
            .map(|(index, value)| (index, value.clone()))
            .collect();
        assert_eq!(globals, before.globals);
        let Value::Resource(original) = &fixture.original else {
            panic!("actual original resource");
        };
        let (_, stored) = fixture
            .machine
            .globals()
            .find(|(_, value)| *value == &fixture.original)
            .unwrap();
        let Value::Resource(stored) = stored else {
            panic!("resource global");
        };
        assert!(Rc::ptr_eq(original, stored));
        assert_eq!(original.resource_type(), &fixture.witness);
        assert_eq!(
            ResourceType::registered(
                &fixture.tables.borrow(),
                "base",
                "BaseJson",
            )
            .unwrap(),
            fixture.witness,
        );
        let tables = fixture.tables.borrow();
        let old = &before.tables;
        assert_eq!(tables.meanings[..old.meanings.len()], old.meanings);
        assert_eq!(
            tables.definitions[..old.definitions.len()],
            old.definitions,
        );
        assert_eq!(
            tables.declarations[..old.declarations.len()],
            old.declarations,
        );
    }

    fn prepare(
        session: &Session,
        name: &str,
        source: &str,
    ) -> Result<Candidate, StageError> {
        if session.declarations.get_module(name).is_some() {
            return Err(StageError::Duplicate);
        }
        let mut declarations = session.declarations.clone();
        declarations
            .module(name, source)
            .map_err(StageError::Declaration)?;
        let cache = session.declared.inherit();
        let checked = publication_proof::check(&declarations, name, &cache)
            .map_err(StageError::Checked)?;
        Ok(Candidate {
            declarations,
            cache,
            checked,
        })
    }

    fn materialize(session: &mut Session, checked: &Checked) -> ResourceType {
        let index = session.index.as_mut().unwrap();
        assert!(index.tables().has_resources);
        index.begin();
        index.shapes(&checked.origin, &checked.shapes);
        let ty = index.ty(&checked.origin, &checked.owner);
        let crate::ir::Type::Meaning {
            index: meaning,
            arguments,
        } = ty
        else {
            panic!("checked nominal publication");
        };
        assert!(arguments.is_empty());
        let tables = index.tables();
        let actual = &tables.meanings[meaning];
        assert!(actual.resource && actual.parameters.is_empty());
        assert_eq!(actual.label, "View");
        let resolve::Origin::Host(name) = &checked.origin else {
            panic!("immutable Host origin");
        };
        assert_eq!(tables.namespaces[actual.namespace], *name);
        ResourceType::registered(&tables, name, "View").unwrap()
    }

    fn contract(candidate: &Candidate, expected: &str) {
        let [(site, whole)] = candidate.checked.members.as_slice() else {
            panic!("one actual getter");
        };
        let resolve::Origin::Host(name) = &candidate.checked.origin else {
            panic!("Host publication");
        };
        let namespace = candidate.declarations.get_module(name).unwrap();
        assert_eq!(namespace.module.span.start, 0);
        assert!(site.start < site.end && site.end <= namespace.module.span.end);
        let crate::types::Type::Function { parameter, result } = whole else {
            panic!("ordinary checked getter");
        };
        assert_eq!(**parameter, candidate.checked.owner);
        assert!(matches!(&**result,
            crate::types::Type::Named { name, arguments }
            if name.origin == resolve::Origin::Prelude
                && name.name == expected && arguments.is_empty()));
    }

    fn commit(session: &mut Session, candidate: Candidate) -> Receipt {
        let journal = session.index.as_mut().unwrap().commit();
        Receipt {
            journal,
            declarations: core::mem::replace(
                &mut session.declarations,
                candidate.declarations,
            ),
            cache: core::mem::replace(&mut session.declared, candidate.cache),
        }
    }

    fn undo(session: &mut Session, receipt: Receipt) {
        session.index.as_mut().unwrap().withdraw(receipt.journal);
        session.declarations = receipt.declarations;
        session.declared = receipt.cache;
    }

    fn absent(fixture: &Fixture, name: &str) {
        assert!(matches!(
            ResourceType::registered(&fixture.tables.borrow(), name, "View",),
            Err(crate::value::resource::ResourceError::NotFound)
        ));
        assert!(fixture.session.declarations.get_module(name).is_none());
    }

    #[test]
    fn checked_publication_trial_rolls_back_without_changing_live_load() {
        let mut fixture = fixture();
        let before = snapshot(&fixture);
        let trial =
            prepare(&fixture.session, "publicationtrial", STRING_VIEW).unwrap();
        contract(&trial, "String");
        let _trial_witness = materialize(&mut fixture.session, &trial.checked);
        original_unchanged(&fixture, &before);
        fixture.session.index.as_mut().unwrap().rollback();
        drop(trial);
        absent(&fixture, "publicationtrial");
        assert_eq!(*fixture.tables.borrow(), before.tables);
        assert_eq!(
            cache_expressions(&fixture.session, &before.origins),
            before.expressions
        );
        assert_eq!(
            cache_values(&fixture.session, &before.origins),
            before.values
        );
        original_unchanged(&fixture, &before);
    }

    #[test]
    fn checked_publication_distinct_views_keep_string_and_integer() {
        let mut fixture = fixture();
        let before = snapshot(&fixture);
        let a = prepare(&fixture.session, "publicationa", STRING_VIEW).unwrap();
        contract(&a, "String");
        let witness_a = materialize(&mut fixture.session, &a.checked);
        let receipt_a = commit(&mut fixture.session, a);
        original_unchanged(&fixture, &before);
        let after_a = fixture.tables.borrow().clone();
        let b =
            prepare(&fixture.session, "publicationb", INTEGER_VIEW).unwrap();
        contract(&b, "Integer");
        let witness_b = materialize(&mut fixture.session, &b.checked);
        let receipt_b = commit(&mut fixture.session, b);
        assert_ne!(witness_a, witness_b);
        assert_eq!(
            fixture.tables.borrow().meanings[..after_a.meanings.len()],
            after_a.meanings
        );
        assert_eq!(
            ResourceType::registered(
                &fixture.tables.borrow(),
                "publicationa",
                "View",
            )
            .unwrap(),
            witness_a
        );
        original_unchanged(&fixture, &before);
        assert!(matches!(
            prepare(&fixture.session, "publicationa", INTEGER_VIEW),
            Err(StageError::Duplicate),
        ));
        assert_eq!(
            ResourceType::registered(
                &fixture.tables.borrow(),
                "publicationa",
                "View",
            )
            .unwrap(),
            witness_a
        );
        assert_eq!(
            fixture
                .session
                .declarations
                .get_module("publicationa")
                .unwrap()
                .module,
            receipt_b
                .declarations
                .get_module("publicationa")
                .unwrap()
                .module
        );
        original_unchanged(&fixture, &before);
        undo(&mut fixture.session, receipt_b);
        original_unchanged(&fixture, &before);
        absent(&fixture, "publicationb");
        undo(&mut fixture.session, receipt_a);
        original_unchanged(&fixture, &before);
        absent(&fixture, "publicationa");
        assert_eq!(*fixture.tables.borrow(), before.tables);
        original_unchanged(&fixture, &before);
    }

    fn refusal(
        fixture: &Fixture,
        name: &str,
        source: &str,
        expected: &Refusal,
    ) {
        match prepare(&fixture.session, name, source) {
            Err(StageError::Checked(Failure::Scope(actual))) => {
                assert_eq!(&actual, expected);
            }
            Err(StageError::Declaration(errors)) => {
                panic!("earlier declaration failure: {errors:?}");
            }
            Err(StageError::Checked(Failure::Resolve(errors))) => {
                panic!("ordinary resolve diagnostics retained: {errors:?}");
            }
            Err(StageError::Checked(Failure::Type(errors))) => {
                panic!("ordinary type diagnostics retained: {errors:?}");
            }
            _ => panic!("expected exact bounded publication refusal"),
        }
        absent(fixture, name);
    }

    #[test]
    fn checked_publication_refusals_and_late_abort_preserve_original_context() {
        let mut fixture = fixture();
        let original = snapshot(&fixture);
        let a = prepare(&fixture.session, "publicationa", STRING_VIEW).unwrap();
        let witness_a = materialize(&mut fixture.session, &a.checked);
        let receipt_a = commit(&mut fixture.session, a);
        original_unchanged(&fixture, &original);
        let b =
            prepare(&fixture.session, "publicationb", INTEGER_VIEW).unwrap();
        let witness_b = materialize(&mut fixture.session, &b.checked);
        let receipt_b = commit(&mut fixture.session, b);
        original_unchanged(&fixture, &original);
        let before = snapshot(&fixture);
        refusal(&fixture, "publicationunmarked", UNMARKED, &Refusal::Owner);
        refusal(&fixture, "publicationgeneric", GENERIC, &Refusal::Owner);
        let mut declarations = fixture.session.declarations.clone();
        declarations.module("publicationinvalid", INVALID).unwrap();
        let module = &declarations
            .get_module("publicationinvalid")
            .unwrap()
            .module;
        let site = module
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Item(crate::ast::Item {
                    kind: ItemKind::Impl { members, .. },
                    ..
                }) => members.iter().find_map(|member| match member {
                    crate::ast::ImplMember::Named { name, .. } => {
                        Some(name.span)
                    }
                    crate::ast::ImplMember::Constructor { .. } => None,
                }),
                _ => None,
            })
            .unwrap();
        refusal(
            &fixture,
            "publicationinvalid",
            INVALID,
            &Refusal::NonConcrete(site),
        );
        let candidate =
            prepare(&fixture.session, "publicationrollback", STRING_VIEW)
                .unwrap();
        let _private_witness =
            materialize(&mut fixture.session, &candidate.checked);
        // Abort at the real staged boundary, before any catalog/cache commit.
        fixture.session.index.as_mut().unwrap().rollback();
        drop(candidate);
        absent(&fixture, "publicationrollback");
        assert_eq!(*fixture.tables.borrow(), before.tables);
        assert_eq!(
            cache_expressions(&fixture.session, &before.origins),
            before.expressions
        );
        assert_eq!(
            cache_values(&fixture.session, &before.origins),
            before.values
        );
        original_unchanged(&fixture, &before);
        assert_eq!(
            ResourceType::registered(
                &fixture.tables.borrow(),
                "publicationa",
                "View",
            )
            .unwrap(),
            witness_a
        );
        assert_eq!(
            ResourceType::registered(
                &fixture.tables.borrow(),
                "publicationb",
                "View",
            )
            .unwrap(),
            witness_b
        );
        undo(&mut fixture.session, receipt_b);
        original_unchanged(&fixture, &original);
        undo(&mut fixture.session, receipt_a);
        original_unchanged(&fixture, &original);
    }
}
