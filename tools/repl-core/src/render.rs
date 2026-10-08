//! Diagnostic reasons in words, one function per reason enum; types
//! print through their `Display`, never `Debug`.

use alloc::{
    format,
    string::{String, ToString},
};
use fipe::{
    decl, infer, ir,
    parse::{self, Expected},
    resolve,
    token::{Source, Span},
    value::{structural, Value},
};

/// A value in the language's spelling (`6`, `Just 2`, `[1, 2]`, `"a"`,
/// `Point 1 2`, `(1, "a")`, `<function>`), never through a `show` member.
pub fn value(value: &Value, tables: &ir::Tables) -> String {
    structural::show(value, tables)
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
    use parse::Reason as R;
    match reason {
        R::Expected(expected) => format!("expected {}", thing(expected)),
        R::Lexical(_)
        | R::InvalidTokenStream
        | R::OperatorInArguments
        | R::ChainedComparison
        | R::FlagValueMustTouch
        | R::ArmsNeedGroup
        | R::MixedArmsAndLines
        | R::InvalidRestPattern
        | R::CommaBeforeLineBreak
        | R::NestingLimit
        | R::BindingValue
        | R::UnattachedDoc
        | R::InnerDocPosition
        | R::EitherNot => reason.to_string(),
    }
}

fn thing(expected: Expected) -> String {
    match expected {
        Expected::Expression => "an expression".into(),
        Expected::Pattern => "a pattern".into(),
        Expected::Type => "a type".into(),
        Expected::Name => "a name".into(),
        Expected::UnqualifiedName => "a name without dots".into(),
        Expected::NonnegativeInteger => "a non-negative integer".into(),
        Expected::LineEnd => "the end of the line".into(),
        Expected::Symbol(symbol) => symbol.to_string(),
    }
}

pub fn resolve(reason: &resolve::Reason, at: &Context) -> String {
    use resolve::Reason as R;
    match reason {
        R::DefaultScope => "a default may name earlier spots and module-level names, not an enclosing function’s locals".into(),
        R::SelfDefault => "a default may not name the binding it belongs to".into(),
        R::UnknownName => "unknown name".into(),
        R::NoGate { name } => format!("`{name}` has no test, so it is no gate: annotate the place, `x: {name} = …`, or write `to<{name}>`"),
        R::UnknownType => "unknown type".into(),
        R::ImportCycle => "import cycle".into(),
        R::NoMain => "a used module has no main".into(),
        R::CaseRule { uppercase: true } => {
            "a type or constructor name starts uppercase".into()
        }
        R::CaseRule { uppercase: false } => {
            "a binding, field or parameter name starts lowercase".into()
        }
        R::Duplicate { first } => {
            format!("already defined: first defined at {}", at.site(*first))
        }
        R::DeclarationForm => {
            "a declaration form, allowed only in host files".into()
        }
        R::SelfOutsideBody => "`Self` outside a type or impl body".into(),
        R::UnknownModule => "unknown module".into(),
        R::NotExported => "the module exports no such name".into(),
        R::UnknownMember => "no such member".into(),
        R::LimitedImpl => "a limited type has no impl".into(),
        R::ImplOutsideModule => "impl of a type from another module".into(),
        R::UnknownField => "the type declares no such field".into(),
        R::PrivateField { field } => format!("field `{field}` is private"),
        R::PrivateMember { member } => format!("member `{member}` is private"),
        R::NotYetChecked { form } => format!("{form} is not yet checked"),
        R::TimingForm => String::from("a timing form takes exactly one operand; its name is not a value or pattern"),
        R::NumericName { upper } => {
            format!("numeric types are uppercase: write `{upper}`")
        }
    }
}

pub fn infer(reason: &infer::Reason) -> String {
    use infer::Reason as R;
    match reason {
        R::Mismatch {
            expected,
            actual: fipe::types::Type::Array { .. },
            ..
        } if matches!(expected, fipe::types::Type::Named { name, .. } if name.origin == fipe::resolve::Origin::Prelude && name.name == "Vec") =>
        {
            format!("a bound array cannot enter {expected}; use to<{expected}>")
        }
        R::Mismatch {
            expected, actual, ..
        } => format!(
            "expected {}, found {}",
            expected.beside(actual),
            actual.beside(expected)
        ),
        R::CallReading { style } => match style {
            fipe::ast::CallStyle::Space => {
                "written as one tuple argument; the argument-list reading binds"
                    .into()
            }
            fipe::ast::CallStyle::Parenthesized => {
                "written as an argument list; the tuple reading binds".into()
            }
        },
        R::TypeDepth => "inferred type nesting limit exceeded".into(),
        R::Occurs => "a type that would contain itself".into(),
        R::NothingPayload => "Nothing has no payload".into(),
        R::OutcomeText { family } => {
            format!("{family} cannot enter a text place; use || or |:")
        }
        R::UnknownFamily => "no outcome family is known here".into(),
        R::ClaimBlock { claim, problem } => format!("claim {claim} {problem}"),
        R::MissingClaim { ty, claim } => {
            format!("{ty} does not have a ready claim for {claim}")
        }
        R::MissingMember { ty, name } => format!("missing member {ty}.{name}"),
        R::NoField { ty, .. } => format!("no such field or member on {ty}"),
        R::Conversion {
            source,
            target,
            candidates,
        } => {
            if matches!(
                target,
                fipe::types::Type::Var(_) | fipe::types::Type::Rigid(_)
            ) {
                return "conversion needs a known target meaning".into();
            }
            let choices = if candidates.is_empty() {
                String::new()
            } else {
                let choices = candidates
                    .iter()
                    .map(ToString::to_string)
                    .collect::<alloc::vec::Vec<_>>()
                    .join(", ");
                format!("; candidates: {choices}")
            };
            match source {
                Some(source) if !matches!(source, fipe::types::Type::Var(_) | fipe::types::Type::Rigid(_)) => format!("no conversion or member of {target} accepts {source}{choices}"),
                _ => format!("{target} needs a known argument meaning{choices}"),
            }
        }
        R::ConflictingConversion => {
            "a conversion for this source meaning is already declared".into()
        }
        R::NoCommonMeaning { forms } => {
            format!("no meaning reads all literal forms: {}", forms.join(", "))
        }
        R::NotAFunction => "not a function".into(),
        R::ExtraArgument => "one argument too many".into(),
        R::UnknownSpot { name } => format!("no spot named `{name}`"),
        R::FlagOperand { name } => {
            format!("flag `{name}` needs an operand: `-{name} value`")
        }
        R::DuplicateSupply { name } => {
            format!("duplicate supply: spot `{name}` is already filled")
        }
        R::Spots { problem } => spot_problem(problem),
        R::DeclarationValue { name, .. } => {
            format!("declared value `{name}` does not type check")
        }
        R::Default { name, cause } => match cause {
            Some(cause) => format!(
                "default for spot `{name}` does not meet its type: {}",
                infer(cause)
            ),
            None => format!(
                "default for spot `{name}` does not meet its type: a name does not resolve"
            ),
        },
        R::NonBoolChaining => {
            "a chained flag needs only Bool spots and no value".into()
        }
        R::FilledSpot => "no open spot to fill".into(),
        R::OpenSpot => "`|!` on Maybe needs a value, not an open spot".into(),
        R::NeedsMaybeOrResult => "`||` and `&&` need Maybe or Result".into(),
        R::NotYetChecked { form } => format!("not yet checked: {form}"),
        R::Discarded => "a line before the last must be ()".into(),
        R::MissingField { name } => format!("missing field `{name}`"),
        R::Limit { message } => message.clone(),
        R::OutOfRange => "integer out of range for its type".into(),
        R::NotExact { ty } if integer_meaning(ty) => {
            "not an integer: a literal at an integer place must be exact".into()
        }
        R::NotExact { ty } => {
            format!("not exact: {ty} cannot hold this literal exactly")
        }
        R::NoBracketForm { ty } => format!(
            "`[:…]` and `[!…]` need a Maybe or Result place; {ty} has no bracket form"
        ),
        R::FamilyClash { left, right } => {
            format!("`||` mixes two outcome families: {left} and {right}")
        }
        R::NoPayload { name } => format!("`{name}` takes no payload"),
        R::MissingPayload { good, ty } => format!(
            "`[{}]` needs a payload of {ty}",
            if *good { ':' } else { '!' }
        ),
        R::Ambiguous { class } => {
            format!("ambiguous: `{class}` on an unknown type")
        }
        R::ImportCycle => "import cycle".into(),
        R::PrivateMember { member } => format!("member `{member}` is private"),
        R::PrivateField { field } => format!("field `{field}` is private"),
        R::NoConstructor { field } => format!(
            "no constructor outside the module: private field `{field}` has no default"
        ),
    }
}

/// Whether a type's meaning is `Integer`, through its limits.
fn integer_meaning(ty: &fipe::types::Type) -> bool {
    match ty {
        fipe::types::Type::Limited { meaning, .. } => integer_meaning(meaning),
        fipe::types::Type::Named { name, .. } => {
            name.origin == fipe::resolve::Origin::Prelude
                && name.name == "Integer"
        }
        _ => false,
    }
}

/// A host declaration file's reason; `text` is that file.
pub fn declaration(reason: &decl::Reason, text: &str) -> String {
    use decl::Reason as R;
    match reason {
        R::Syntax(reason) => syntax(*reason),
        R::ClaimBlock { problem } => format!("invalid claim block: {problem}"),
        R::Spots { problem } => spot_problem(problem),
        R::UnknownDocSpot { name } => {
            format!("doc names unknown spot `{name}`")
        }
        R::NotDeclaration => "not a declaration or `name = value`".into(),
        R::Duplicate { first } => {
            let (line, col) = Source::new(text).line_col(first.start);
            format!("already declared: first declared at {line}:{col}")
        }
        R::NotYetChecked { form } => format!("{form} is not yet checked"),
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
        place.push_str(&format!(" at {location}"));
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

fn spot_problem(problem: &fipe::decl::SpotProblem) -> String {
    match problem {
        fipe::decl::SpotProblem::Duplicate(name) => {
            format!("spot `{name}` is declared twice")
        }
        fipe::decl::SpotProblem::AmbiguousFlag(letter) => {
            format!("more than one Bool spot starts with `{letter}`")
        }
        fipe::decl::SpotProblem::SelfNotLast => {
            "a supplied member must take `self` last".into()
        }
    }
}
