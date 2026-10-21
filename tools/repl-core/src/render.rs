//! Diagnostic reasons in words, one function per reason enum; types
//! print through their `Display`, never `Debug`.

use alloc::{
    format,
    string::{String, ToString},
};
use core::fmt::Write as _;
use fipe::{
    decl, infer, ir, parse, resolve,
    token::{Source, Span},
    value::{structural, Reason, Value},
};

/// A value in the language's spelling (`6`, `Just 2`, `[1, 2]`, `"a"`,
/// `Point 1 2`, `(1, "a")`, `<function>`), never through a `show` member.
pub fn value(value: &Value, tables: &ir::Tables) -> String {
    structural::show(value, tables)
}

/// A Fault's reason in words; a refuted entry names its limit and the
/// value, as the checker's refutation does, the caret giving the place.
pub fn fault(reason: &Reason, tables: &ir::Tables) -> String {
    reason.describe(tables)
}

/// The text a reason's own spans index: the session's accepted lines,
/// each followed by a blank line, then the entry from `source.len()`.
pub struct Context<'a> {
    pub source: &'a str,
    pub line: &'a str,
}

impl Context<'_> {
    /// An accepted line by its number, or `line:col` in the entry.
    fn site(&self, span: Span) -> String {
        if span.start < self.source.len() {
            let before = self.source.get(..span.start).unwrap_or_default();
            format!("line {}", before.matches("\n\n").count() + 1)
        } else {
            let at = span.start - self.source.len();
            let (line, col) = Source::new(self.line).line_col(at);
            format!("{line}:{col}")
        }
    }
}

pub fn syntax(reason: parse::Reason) -> String {
    reason.to_string()
}

pub fn resolve(reason: &resolve::Reason, at: &Context) -> String {
    match reason {
        resolve::Reason::Duplicate { first } => {
            format!("{reason}: first defined at {}", at.site(*first))
        }
        other => other.to_string(),
    }
}

pub fn infer(reason: &infer::Reason) -> String {
    reason.to_string()
}

/// A host declaration file's reason; `text` is that file.
pub fn declaration(reason: &decl::Reason, text: &str) -> String {
    match reason {
        decl::Reason::Duplicate { first } => {
            let (line, col) = Source::new(text).line_col(first.start);
            format!("{reason}: first declared at {line}:{col}")
        }
        other => other.to_string(),
    }
}

pub fn target(target: &resolve::Target, at: &Context) -> String {
    use resolve::Target as T;
    match target {
        T::Local { site } => format!("binding at {}", at.site(*site)),
        T::Value { origin, name } => {
            format!("value {}.{name}", origin_name(origin))
        }
        T::Constructor { origin, ty, name } => {
            format!("constructor {}.{ty}.{name}", origin_name(origin))
        }
        T::Type { origin, name } => {
            format!("type {}.{name}", origin_name(origin))
        }
        T::Module(origin) => format!("module {}", origin_name(origin)),
        T::Variable(name) => format!("type variable {name}"),
        T::SelfType => "enclosing type".into(),
        T::Members { head, members } => format!(
            "members {} of {}",
            members.join("."),
            self::target(head, at)
        ),
        T::TypeMember { ty, member } => {
            format!("member {member} of {}", self::target(ty, at))
        }
        T::Word => "text argument".into(),
    }
}

fn origin_name(origin: &resolve::Origin) -> String {
    use resolve::Origin as O;
    match origin {
        O::This => "current module".into(),
        O::Prelude => "prelude".into(),
        O::Commands => "commands".into(),
        O::File(name) | O::Host(name) => name.clone(),
        O::Line(index) => format!("session unit {}", index + 1),
    }
}

/// Render a plan entry without exposing byte offsets to the reader.
pub fn check(
    check: &fipe::typed::Check,
    location: Option<&str>,
    test: Option<&str>,
) -> String {
    use fipe::typed::Place;
    let mut place = match &check.place_kind {
        Place::Binding(name) => format!("binding {name}"),
        Place::Parameter(Some(name)) => format!("parameter {name}"),
        Place::Parameter(None) => String::from("a parameter"),
        Place::Result => String::from("the result"),
        Place::Field { owner, name } => format!("field {owner}.{name}"),
        Place::Part(index) => format!("tuple part {}", index + 1),
        Place::Element => String::from("an element"),
        Place::Payload => String::from("a payload"),
    };
    if let Some(location) = location {
        let _ = write!(place, " at {location}");
    }
    for fact in check.known.to_string().lines() {
        place.push_str(", ");
        place.push_str(fact);
    }
    let limit = test.map_or_else(
        || check.name.clone(),
        |test| format!("{}: {}", check.name, test.trim()),
    );
    if check.when_integer {
        return format!(
            "{}, {}: {place}, {limit} when Integer ({})\n",
            check.phase, check.frequency, check.reason
        );
    }
    format!(
        "{}, {}: {place}, against {limit} ({})\n",
        check.phase, check.frequency, check.reason
    )
}

/// A proved entry: where and against what, then the value the compile
/// phase proved it on.
pub fn settled(settled: &fipe::check::Settled, test: Option<&str>) -> String {
    let line = check(&settled.check, None, test);
    let open = line.rfind(" (").unwrap_or(line.len());
    format!(
        "{}: proved at compile time on {}\n",
        &line[..open],
        settled.value
    )
}

/// A compile-time evaluation that refuted its test or Faulted.
pub fn compile(what: &fipe::check::Evaluated, reason: &Reason) -> String {
    what.describe(reason)
}
