//! Diagnostic reasons in words, one function per reason enum; types
//! print through their `Display`, never `Debug`.

use alloc::{
    format,
    string::{String, ToString},
};
use fipe::{
    decl, infer,
    parse::{self, Expected},
    resolve,
    token::{Source, Span},
};

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
        | R::RecordInHead
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
        R::UnknownName => "unknown name".into(),
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
        R::SelfOutsideBody => "`self` outside an impl member".into(),
        R::UnknownModule => "unknown module".into(),
        R::NotExported => "the module exports no such name".into(),
        R::UnknownMember => "no such member".into(),
        R::LimitedImpl => "a limited type has no impl".into(),
        R::ImplOutsideModule => "impl of a type from another module".into(),
        R::UnknownField => "the type declares no such field".into(),
        R::PrivateField { field } => format!("field `{field}` is private"),
        R::PrivateMember { member } => format!("member `{member}` is private"),
        R::NotYetChecked { form } => format!("{form} is not yet checked"),
        R::NumericName { upper } => {
            format!("numeric types are uppercase: write `{upper}`")
        }
    }
}

pub fn infer(reason: &infer::Reason) -> String {
    use infer::Reason as R;
    match reason {
        R::Mismatch {
            expected, actual, ..
        } => format!(
            "expected {}, found {}",
            expected.beside(actual),
            actual.beside(expected)
        ),
        R::Occurs => "a type that would contain itself".into(),
        R::UnknownFamily => "no outcome family is known here".into(),
        R::MissingMember { ty, name } => format!("missing member {ty}.{name}"),
        R::NoField { ty, .. } => format!("no such field or member on {ty}"),
        R::Conversion { target, candidates } => format!("conversion to {target} needs a known source meaning; candidates: {}", candidates.iter().map(ToString::to_string).collect::<alloc::vec::Vec<_>>().join(", ")),
        R::ConflictingConversion => "a conversion for this source meaning is already declared".into(),
        R::NoCommonMeaning { forms } => format!("no meaning reads all literal forms: {}", forms.join(", ")),
        R::NotAFunction => "not a function".into(),
        R::ExtraArgument => "one argument too many".into(),
        R::UnknownSpot { name } => format!("no spot named `{name}`"),
        R::FilledSpot => "no open spot to fill".into(),
        R::OpenSpot => "`|!` on Maybe needs a value, not an open spot".into(),
        R::NeedsMaybeOrResult => "`||` and `&&` need Maybe or Result".into(),
        R::NotYetChecked { form } => format!("not yet checked: {form}"),
        R::Discarded => "a line before the last must be ()".into(),
        R::MissingField { name } => format!("missing field `{name}`"),
        R::OutOfRange => "integer out of range for its type".into(),
        R::Ambiguous { class } => {
            format!("ambiguous: `{class}` on an unknown type")
        }
        R::ImportCycle => "import cycle".into(),
        R::PrivateMember { member } => format!("member `{member}` is private"),
    }
}

/// A host declaration file's reason; `text` is that file.
pub fn declaration(reason: &decl::Reason, text: &str) -> String {
    use decl::Reason as R;
    match reason {
        R::Syntax(reason) => syntax(*reason),
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
