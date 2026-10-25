//! Stable semantic codes and exact source placement, independent of message text.
use super::{Diagnostic, InputError, Related, Severity, Sources};
use crate::{
    ast::{Entry, ItemKind, Module},
    check, infer, parse, resolve,
    token::Span,
};
use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};

const fn lexical(reason: crate::lex::Reason) -> &'static str {
    match reason {
        crate::lex::Reason::UnterminatedQuote => "lex.unterminated-quote",
        crate::lex::Reason::BadEscape => "lex.bad-escape",
        crate::lex::Reason::InvalidUtf8 => "lex.invalid-utf8",
        crate::lex::Reason::Stray(_) => "lex.stray",
    }
}

const fn syntax(reason: parse::Reason) -> &'static str {
    match reason {
        parse::Reason::Lexical(inner) => lexical(inner),
        parse::Reason::InvalidTokenStream => "syntax.invalid-token-stream",
        parse::Reason::Expected(_) => "syntax.expected",
        parse::Reason::OperatorInArguments => "syntax.operator-in-arguments",
        parse::Reason::ApplicationInOperator => {
            "syntax.application-in-operator"
        }
        parse::Reason::ChainedComparison => "syntax.chained-comparison",
        parse::Reason::FlagValueMustTouch => "syntax.flag-value-must-touch",
        parse::Reason::ArmsNeedGroup => "syntax.arms-need-group",
        parse::Reason::MixedArmsAndLines => "syntax.mixed-arms-and-lines",
        parse::Reason::InvalidRestPattern => "syntax.invalid-rest-pattern",
        parse::Reason::CommaBeforeLineBreak => "syntax.comma-before-line-break",
        parse::Reason::NestingLimit => "syntax.nesting-limit",
        parse::Reason::DepthLimit => "syntax.depth-limit",
        parse::Reason::BindingValue => "syntax.binding-value",
        parse::Reason::EitherNot => "syntax.either-not",
        parse::Reason::UnattachedDoc => "syntax.unattached-doc",
        parse::Reason::InnerDocPosition => "syntax.inner-doc-position",
    }
}

const fn resolution(reason: &resolve::Reason) -> &'static str {
    match reason {
        resolve::Reason::DefaultScope => "resolve.default-scope",
        resolve::Reason::SelfDefault => "resolve.self-default",
        resolve::Reason::UnknownName => "resolve.unknown-name",
        resolve::Reason::UnknownNameIn { .. } => "resolve.unknown-name-in",
        resolve::Reason::NoGate { .. } => "resolve.no-gate",
        resolve::Reason::TimingForm => "resolve.timing-form",
        resolve::Reason::UnknownType => "resolve.unknown-type",
        resolve::Reason::CaseRule { .. } => "resolve.case-rule",
        resolve::Reason::Duplicate { .. } => "resolve.duplicate",
        resolve::Reason::DeclarationForm => "resolve.declaration-form",
        resolve::Reason::SelfOutsideBody => "resolve.self-outside-body",
        resolve::Reason::UnknownModule => "resolve.unknown-module",
        resolve::Reason::NotExported => "resolve.not-exported",
        resolve::Reason::UnknownMember => "resolve.unknown-member",
        resolve::Reason::ImplOutsideModule => "resolve.impl-outside-module",
        resolve::Reason::LimitedImpl => "resolve.limited-impl",
        resolve::Reason::UnknownField => "resolve.unknown-field",
        resolve::Reason::PrivateField { .. } => "resolve.private-field",
        resolve::Reason::PrivateMember { .. } => "resolve.private-member",
        resolve::Reason::ImportCycle => "resolve.import-cycle",
        resolve::Reason::NoMain => "resolve.no-main",
        resolve::Reason::NotYetChecked { .. } => "resolve.not-yet-checked",
        resolve::Reason::NumericName { .. } => "resolve.numeric-name",
    }
}

const fn typing(reason: &infer::Reason) -> &'static str {
    match reason {
        infer::Reason::Spots { .. } => "type.spots",
        infer::Reason::TypeDepth => "type.type-depth",
        infer::Reason::CallReading { .. } => "type.call-reading",
        infer::Reason::Mismatch { .. } => "type.mismatch",
        infer::Reason::Occurs => "type.occurs",
        infer::Reason::UnknownFamily => "type.unknown-family",
        infer::Reason::NothingPayload => "type.nothing-payload",
        infer::Reason::OutcomeText { .. } => "type.outcome-text",
        infer::Reason::NoField { .. } => "type.no-field",
        infer::Reason::ClaimBlock { .. } => "type.claim-block",
        infer::Reason::MissingClaim { .. } => "type.missing-claim",
        infer::Reason::MissingMember { .. } => "type.missing-member",
        infer::Reason::NoCommonMeaning { .. } => "type.no-common-meaning",
        infer::Reason::NotAFunction => "type.not-a-function",
        infer::Reason::NotRecord => "type.not-record",
        infer::Reason::ResourceInFile { .. } => "type.resource-in-file",
        infer::Reason::Conversion { .. } => "type.conversion",
        infer::Reason::ConflictingConversion => "type.conflicting-conversion",
        infer::Reason::ExtraArgument => "type.extra-argument",
        infer::Reason::IfWithoutElse => "type.if-without-else",
        infer::Reason::LiteralApplied => "type.literal-applied",
        infer::Reason::PositionalBool => "type.positional-bool",
        infer::Reason::NonBoolChaining => "type.non-bool-chaining",
        infer::Reason::Default { .. } => "type.default",
        infer::Reason::DeclarationValue { .. } => "type.declaration-value",
        infer::Reason::UnknownSpot { .. } => "type.unknown-spot",
        infer::Reason::FlagOperand { .. } => "type.flag-operand",
        infer::Reason::DuplicateSupply { .. } => "type.duplicate-supply",
        infer::Reason::FilledSpot => "type.filled-spot",
        infer::Reason::OpenSpot => "type.open-spot",
        infer::Reason::NeedsMaybeOrResult => "type.needs-maybe-or-result",
        infer::Reason::NeedsOutcome => "type.needs-outcome",
        infer::Reason::NotYetChecked { .. } => "type.not-yet-checked",
        infer::Reason::Discarded => "type.discarded",
        infer::Reason::MissingField { .. } => "type.missing-field",
        infer::Reason::OutOfRange => "type.out-of-range",
        infer::Reason::NotExact { .. } => "type.not-exact",
        infer::Reason::NoBracketForm { .. } => "type.no-bracket-form",
        infer::Reason::FamilyClash { .. } => "type.family-clash",
        infer::Reason::NoPayload { .. } => "type.no-payload",
        infer::Reason::MissingPayload { .. } => "type.missing-payload",
        infer::Reason::Limit { .. } => "type.limit",
        infer::Reason::Ambiguous { .. } => "type.ambiguous",
        infer::Reason::ImportCycle => "type.import-cycle",
        infer::Reason::PrivateMember { .. } => "type.private-member",
        infer::Reason::PrivateField { .. } => "type.private-field",
        infer::Reason::NoConstructor { .. } => "type.no-constructor",
    }
}

const fn compile(reason: &crate::value::Reason) -> &'static str {
    match reason {
        crate::value::Reason::Budget => "check.budget",
        crate::value::Reason::DivideByZero => "check.divide-by-zero",
        crate::value::Reason::NotFinite => "check.not-finite",
        crate::value::Reason::NotExact { .. } => "check.not-exact",
        crate::value::Reason::Failed { .. } => "check.refuted",
        crate::value::Reason::Cast => "check.cast",
        crate::value::Reason::Unreachable => "check.unreachable",
        crate::value::Reason::Host(_) => "check.host",
        crate::value::Reason::Message(_) => "check.message",
    }
}

fn related(
    diagnostic: &mut Diagnostic,
    sources: Sources<'_>,
    origin: &resolve::Origin,
    span: Span,
    message: &str,
) -> Result<(), InputError> {
    let item = Related {
        location: sources.location(origin, span)?,
        message: String::from(message),
    };
    if !diagnostic.related.contains(&item) {
        diagnostic.related.push(item);
    }
    Ok(())
}

pub(super) fn checked(
    error: &check::Error,
    sources: Sources<'_>,
) -> Result<Diagnostic, InputError> {
    let mut origin = resolve::Origin::This;
    let (span, code, message) = match error {
        check::Error::Syntax(error) => {
            (error.span, syntax(error.reason), error.reason.to_string())
        }
        check::Error::Resolve(error) => (
            error.span,
            resolution(&error.reason),
            error.reason.to_string(),
        ),
        check::Error::Type(error) => {
            (error.span, typing(&error.reason), error.reason.to_string())
        }
        check::Error::File { stem, error } => {
            origin = resolve::Origin::File(stem.clone());
            (error.span, syntax(error.reason), error.reason.to_string())
        }
        check::Error::FileResolve { stem, error } => {
            origin = resolve::Origin::File(stem.clone());
            (
                error.span,
                resolution(&error.reason),
                error.reason.to_string(),
            )
        }
        check::Error::FileType { stem, error } => {
            origin = resolve::Origin::File(stem.clone());
            (error.span, typing(&error.reason), error.reason.to_string())
        }
        check::Error::Compile { span, what, reason } => {
            (*span, compile(reason), what.describe(reason))
        }
    };
    let mut diagnostic = Diagnostic {
        location: sources.location(&origin, span)?,
        code,
        message,
        severity: Severity::Error,
        related: Vec::new(),
    };
    match error {
        check::Error::Resolve(error)
        | check::Error::FileResolve { error, .. } => {
            if let resolve::Reason::Duplicate { first } = &error.reason {
                related(
                    &mut diagnostic,
                    sources,
                    &origin,
                    *first,
                    "declared here",
                )?;
            }
        }
        check::Error::Type(error) | check::Error::FileType { error, .. } => {
            match &error.reason {
                infer::Reason::Mismatch { other, .. }
                | infer::Reason::NoField { other, .. } => {
                    related(
                        &mut diagnostic,
                        sources,
                        &origin,
                        *other,
                        "required here",
                    )?;
                }
                infer::Reason::DeclarationValue {
                    origin,
                    declaration,
                    ..
                } => {
                    related(
                        &mut diagnostic,
                        sources,
                        origin,
                        *declaration,
                        "declared here",
                    )?;
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(diagnostic)
}

fn uses(module: &Module) -> impl Iterator<Item = (&str, Span)> {
    module.entries.iter().flat_map(|entry| {
        let names = match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Use { names, .. } => names.as_slice(),
                _ => &[],
            },
            Entry::Line(_) => &[],
        };
        names.iter().filter_map(|name| {
            Some((name.segments.first()?.as_str(), name.span))
        })
    })
}

pub(super) fn imports(
    diagnostic: &mut Diagnostic,
    module: &Module,
    files: &[(String, Module)],
    sources: Sources<'_>,
) -> Result<(), InputError> {
    if diagnostic.location.source == sources.current {
        return Ok(());
    }
    for (stem, span) in uses(module) {
        let mut pending = alloc::vec![stem];
        let mut seen = BTreeSet::new();
        let mut reaches = false;
        while let Some(stem) = pending.pop() {
            if !seen.insert(stem) {
                continue;
            }
            let Some((_, file)) = files.iter().find(|(known, _)| known == stem)
            else {
                continue;
            };
            if sources.source(&resolve::Origin::File(String::from(stem)))?
                == diagnostic.location.source
            {
                reaches = true;
                break;
            }
            pending.extend(uses(file).map(|(stem, _)| stem));
        }
        if reaches {
            related(
                diagnostic,
                sources,
                &resolve::Origin::This,
                span,
                "imported here",
            )?;
        }
    }
    Ok(())
}

pub(super) fn warning(
    warning: &crate::typed::Warning,
    sources: Sources<'_>,
) -> Result<Diagnostic, InputError> {
    Ok(Diagnostic {
        location: sources.location(&resolve::Origin::This, warning.span)?,
        severity: Severity::Warning,
        code: "warning.ambiguous-call",
        message: String::from(match warning.style {
            crate::ast::CallStyle::Space => {
                "ambiguous call: one tuple argument"
            }
            crate::ast::CallStyle::Parenthesized => {
                "ambiguous call: an argument list"
            }
        }),
        related: Vec::new(),
    })
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::types::Type;

    #[test]
    fn lexical_catalog_covers_current_variants() {
        let cases = [
            (
                crate::lex::Reason::UnterminatedQuote,
                "lex.unterminated-quote",
            ),
            (crate::lex::Reason::BadEscape, "lex.bad-escape"),
            (crate::lex::Reason::InvalidUtf8, "lex.invalid-utf8"),
            (crate::lex::Reason::Stray('x'), "lex.stray"),
        ];
        for (reason, expected) in cases {
            assert_eq!(lexical(reason), expected);
        }
    }

    #[test]
    fn syntax_catalog_covers_current_variants() {
        let cases = [
            (
                parse::Reason::Lexical(crate::lex::Reason::BadEscape),
                "lex.bad-escape",
            ),
            (
                parse::Reason::InvalidTokenStream,
                "syntax.invalid-token-stream",
            ),
            (
                parse::Reason::Expected(parse::Expected::Expression),
                "syntax.expected",
            ),
            (
                parse::Reason::OperatorInArguments,
                "syntax.operator-in-arguments",
            ),
            (
                parse::Reason::ApplicationInOperator,
                "syntax.application-in-operator",
            ),
            (
                parse::Reason::ChainedComparison,
                "syntax.chained-comparison",
            ),
            (
                parse::Reason::FlagValueMustTouch,
                "syntax.flag-value-must-touch",
            ),
            (parse::Reason::ArmsNeedGroup, "syntax.arms-need-group"),
            (
                parse::Reason::MixedArmsAndLines,
                "syntax.mixed-arms-and-lines",
            ),
            (
                parse::Reason::InvalidRestPattern,
                "syntax.invalid-rest-pattern",
            ),
            (
                parse::Reason::CommaBeforeLineBreak,
                "syntax.comma-before-line-break",
            ),
            (parse::Reason::NestingLimit, "syntax.nesting-limit"),
            (parse::Reason::DepthLimit, "syntax.depth-limit"),
            (parse::Reason::BindingValue, "syntax.binding-value"),
            (parse::Reason::EitherNot, "syntax.either-not"),
            (parse::Reason::UnattachedDoc, "syntax.unattached-doc"),
            (parse::Reason::InnerDocPosition, "syntax.inner-doc-position"),
        ];
        for (reason, expected) in cases {
            assert_eq!(syntax(reason), expected);
        }
    }

    #[test]
    fn resolution_catalog_covers_current_variants() {
        let cases = [
            (resolve::Reason::DefaultScope, "resolve.default-scope"),
            (resolve::Reason::SelfDefault, "resolve.self-default"),
            (resolve::Reason::UnknownName, "resolve.unknown-name"),
            (
                resolve::Reason::UnknownNameIn {
                    module: String::new(),
                    name: String::new(),
                },
                "resolve.unknown-name-in",
            ),
            (
                resolve::Reason::NoGate {
                    name: String::new(),
                },
                "resolve.no-gate",
            ),
            (resolve::Reason::TimingForm, "resolve.timing-form"),
            (resolve::Reason::UnknownType, "resolve.unknown-type"),
            (
                resolve::Reason::CaseRule { uppercase: true },
                "resolve.case-rule",
            ),
            (
                resolve::Reason::Duplicate {
                    first: Span::default(),
                },
                "resolve.duplicate",
            ),
            (resolve::Reason::DeclarationForm, "resolve.declaration-form"),
            (
                resolve::Reason::SelfOutsideBody,
                "resolve.self-outside-body",
            ),
            (resolve::Reason::UnknownModule, "resolve.unknown-module"),
            (resolve::Reason::NotExported, "resolve.not-exported"),
            (resolve::Reason::UnknownMember, "resolve.unknown-member"),
            (
                resolve::Reason::ImplOutsideModule,
                "resolve.impl-outside-module",
            ),
            (resolve::Reason::LimitedImpl, "resolve.limited-impl"),
            (resolve::Reason::UnknownField, "resolve.unknown-field"),
            (
                resolve::Reason::PrivateField {
                    field: String::new(),
                },
                "resolve.private-field",
            ),
            (
                resolve::Reason::PrivateMember {
                    member: String::new(),
                },
                "resolve.private-member",
            ),
            (resolve::Reason::ImportCycle, "resolve.import-cycle"),
            (resolve::Reason::NoMain, "resolve.no-main"),
            (
                resolve::Reason::NotYetChecked {
                    form: resolve::Unchecked::Affix,
                },
                "resolve.not-yet-checked",
            ),
            (
                resolve::Reason::NumericName {
                    upper: String::new(),
                },
                "resolve.numeric-name",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(resolution(&reason), expected);
        }
    }

    #[test]
    fn typing_catalog_heads_and_constraints() {
        let cases = [
            (
                infer::Reason::Spots {
                    problem: crate::decl::SpotProblem::Duplicate(String::new()),
                },
                "type.spots",
            ),
            (infer::Reason::TypeDepth, "type.type-depth"),
            (
                infer::Reason::CallReading {
                    style: crate::ast::CallStyle::Space,
                },
                "type.call-reading",
            ),
            (
                infer::Reason::Mismatch {
                    expected: Type::Unit,
                    actual: Type::Unit,
                    other: Span::default(),
                },
                "type.mismatch",
            ),
            (infer::Reason::Occurs, "type.occurs"),
            (infer::Reason::UnknownFamily, "type.unknown-family"),
            (infer::Reason::NothingPayload, "type.nothing-payload"),
            (
                infer::Reason::OutcomeText { family: Type::Unit },
                "type.outcome-text",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(typing(&reason), expected);
        }
    }

    #[test]
    fn typing_catalog_calls_and_conversion() {
        let cases = [
            (
                infer::Reason::NoField {
                    ty: Type::Unit,
                    other: Span::default(),
                },
                "type.no-field",
            ),
            (
                infer::Reason::ClaimBlock {
                    claim: Type::Unit,
                    problem: String::new(),
                },
                "type.claim-block",
            ),
            (
                infer::Reason::MissingClaim {
                    ty: Type::Unit,
                    claim: Type::Unit,
                },
                "type.missing-claim",
            ),
            (
                infer::Reason::MissingMember {
                    ty: Type::Unit,
                    name: String::new(),
                },
                "type.missing-member",
            ),
            (
                infer::Reason::NoCommonMeaning { forms: Vec::new() },
                "type.no-common-meaning",
            ),
            (infer::Reason::NotAFunction, "type.not-a-function"),
            (infer::Reason::NotRecord, "type.not-record"),
            (
                infer::Reason::ResourceInFile {
                    module: String::new(),
                },
                "type.resource-in-file",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(typing(&reason), expected);
        }
    }

    #[test]
    fn typing_catalog_arguments_and_defaults() {
        let cases = [
            (
                infer::Reason::Conversion {
                    source: None,
                    target: Type::Unit,
                    candidates: Vec::new(),
                },
                "type.conversion",
            ),
            (
                infer::Reason::ConflictingConversion,
                "type.conflicting-conversion",
            ),
            (infer::Reason::ExtraArgument, "type.extra-argument"),
            (infer::Reason::IfWithoutElse, "type.if-without-else"),
            (infer::Reason::LiteralApplied, "type.literal-applied"),
            (infer::Reason::PositionalBool, "type.positional-bool"),
            (infer::Reason::NonBoolChaining, "type.non-bool-chaining"),
            (
                infer::Reason::Default {
                    name: String::new(),
                    cause: None,
                },
                "type.default",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(typing(&reason), expected);
        }
    }

    #[test]
    fn typing_catalog_outcomes_and_fields() {
        let cases = [
            (
                infer::Reason::DeclarationValue {
                    name: String::new(),
                    origin: resolve::Origin::This,
                    declaration: Span::default(),
                },
                "type.declaration-value",
            ),
            (
                infer::Reason::UnknownSpot {
                    name: String::new(),
                },
                "type.unknown-spot",
            ),
            (
                infer::Reason::FlagOperand {
                    name: String::new(),
                },
                "type.flag-operand",
            ),
            (
                infer::Reason::DuplicateSupply {
                    name: String::new(),
                },
                "type.duplicate-supply",
            ),
            (infer::Reason::FilledSpot, "type.filled-spot"),
            (infer::Reason::OpenSpot, "type.open-spot"),
            (
                infer::Reason::NeedsMaybeOrResult,
                "type.needs-maybe-or-result",
            ),
            (infer::Reason::NeedsOutcome, "type.needs-outcome"),
        ];
        for (reason, expected) in cases {
            assert_eq!(typing(&reason), expected);
        }
    }

    #[test]
    fn typing_catalog_literal_and_payload() {
        let cases = [
            (
                infer::Reason::NotYetChecked {
                    form: resolve::Unchecked::Affix,
                },
                "type.not-yet-checked",
            ),
            (infer::Reason::Discarded, "type.discarded"),
            (
                infer::Reason::MissingField {
                    name: String::new(),
                },
                "type.missing-field",
            ),
            (infer::Reason::OutOfRange, "type.out-of-range"),
            (infer::Reason::NotExact { ty: Type::Unit }, "type.not-exact"),
            (
                infer::Reason::NoBracketForm { ty: Type::Unit },
                "type.no-bracket-form",
            ),
            (
                infer::Reason::FamilyClash {
                    left: Type::Unit,
                    right: Type::Unit,
                },
                "type.family-clash",
            ),
            (
                infer::Reason::NoPayload {
                    name: String::new(),
                },
                "type.no-payload",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(typing(&reason), expected);
        }
    }

    #[test]
    fn typing_catalog_visibility() {
        let cases = [
            (
                infer::Reason::MissingPayload {
                    good: true,
                    ty: Type::Unit,
                },
                "type.missing-payload",
            ),
            (
                infer::Reason::Limit {
                    message: String::new(),
                },
                "type.limit",
            ),
            (
                infer::Reason::Ambiguous {
                    class: String::new(),
                },
                "type.ambiguous",
            ),
            (infer::Reason::ImportCycle, "type.import-cycle"),
            (
                infer::Reason::PrivateMember {
                    member: String::new(),
                },
                "type.private-member",
            ),
            (
                infer::Reason::PrivateField {
                    field: String::new(),
                },
                "type.private-field",
            ),
            (
                infer::Reason::NoConstructor {
                    field: String::new(),
                },
                "type.no-constructor",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(typing(&reason), expected);
        }
    }

    #[test]
    fn compile_catalog_covers_current_variants() {
        let cases = [
            (crate::value::Reason::Budget, "check.budget"),
            (crate::value::Reason::DivideByZero, "check.divide-by-zero"),
            (crate::value::Reason::NotFinite, "check.not-finite"),
            (
                crate::value::Reason::NotExact {
                    value: String::new(),
                    target: String::new(),
                },
                "check.not-exact",
            ),
            (
                crate::value::Reason::Failed {
                    entry: 0,
                    value: String::new(),
                },
                "check.refuted",
            ),
            (crate::value::Reason::Cast, "check.cast"),
            (crate::value::Reason::Unreachable, "check.unreachable"),
            (
                crate::value::Reason::Host(String::from("host detail")),
                "check.host",
            ),
            (
                crate::value::Reason::Message(String::from("detail")),
                "check.message",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(compile(&reason), expected);
        }
    }

    #[test]
    fn expected_and_spot_payloads_do_not_change_codes() {
        for expected in [
            parse::Expected::Expression,
            parse::Expected::Pattern,
            parse::Expected::Type,
            parse::Expected::Name,
            parse::Expected::UnqualifiedName,
            parse::Expected::NonnegativeInteger,
            parse::Expected::LineEnd,
            parse::Expected::Symbol(crate::token::Symbol::Plus),
        ] {
            assert_eq!(
                syntax(parse::Reason::Expected(expected)),
                "syntax.expected"
            );
        }
        for problem in [
            crate::decl::SpotProblem::Duplicate(String::from("x")),
            crate::decl::SpotProblem::AmbiguousFlag('x'),
            crate::decl::SpotProblem::SelfNotLast,
        ] {
            assert_eq!(typing(&infer::Reason::Spots { problem }), "type.spots");
        }
    }
    #[test]
    fn wrappers_and_compile_contexts_preserve_source_and_code() {
        let origins = [(resolve::Origin::File(String::from("file")), 1)];
        let sources = Sources {
            texts: &["", ""],
            current: 0,
            origins: &origins,
            current_file: None,
        };
        let syntax = parse::Error {
            span: Span::default(),
            reason: parse::Reason::Expected(parse::Expected::Expression),
        };
        let resolved = resolve::Error {
            span: Span::default(),
            reason: resolve::Reason::UnknownName,
        };
        let typed = infer::Error {
            span: Span::default(),
            reason: infer::Reason::NotAFunction,
        };
        for (error, source, code) in [
            (check::Error::Syntax(syntax.clone()), 0, "syntax.expected"),
            (
                check::Error::Resolve(resolved.clone()),
                0,
                "resolve.unknown-name",
            ),
            (check::Error::Type(typed.clone()), 0, "type.not-a-function"),
            (
                check::Error::File {
                    stem: String::from("file"),
                    error: syntax,
                },
                1,
                "syntax.expected",
            ),
            (
                check::Error::FileResolve {
                    stem: String::from("file"),
                    error: resolved,
                },
                1,
                "resolve.unknown-name",
            ),
            (
                check::Error::FileType {
                    stem: String::from("file"),
                    error: typed,
                },
                1,
                "type.not-a-function",
            ),
        ] {
            let diagnostic = checked(&error, sources).unwrap();
            assert_eq!(diagnostic.code, code);
            assert_eq!(diagnostic.location.source, source);
            assert_eq!(diagnostic.severity, Severity::Error);
        }
        for what in [
            check::Evaluated::Entry {
                limit: String::from("Positive"),
                value: Some(String::from("-1")),
            },
            check::Evaluated::Literal {
                meaning: String::from("Integer"),
            },
            check::Evaluated::Demand,
            check::Evaluated::Sequence {
                meaning: String::from("Vec"),
            },
        ] {
            let error = check::Error::Compile {
                span: Span::default(),
                what,
                reason: crate::value::Reason::Budget,
            };
            assert_eq!(checked(&error, sources).unwrap().code, "check.budget");
        }
    }

    #[test]
    fn related_locations_and_warning_payloads_keep_their_own_messages() {
        let origins = [(resolve::Origin::Host(String::from("host")), 1)];
        let sources = Sources {
            texts: &["value", "declaration"],
            current: 0,
            origins: &origins,
            current_file: None,
        };
        let error = check::Error::Type(infer::Error {
            span: Span { start: 0, end: 5 },
            reason: infer::Reason::DeclarationValue {
                name: String::from("bad"),
                origin: resolve::Origin::Host(String::from("host")),
                declaration: Span { start: 0, end: 11 },
            },
        });
        let diagnostic = checked(&error, sources).unwrap();
        assert_eq!(diagnostic.related[0].location.source, 1);
        assert_eq!(diagnostic.related[0].message, "declared here");
        for style in [
            crate::ast::CallStyle::Space,
            crate::ast::CallStyle::Parenthesized,
        ] {
            let diagnostic = warning(
                &crate::typed::Warning {
                    span: Span { start: 0, end: 5 },
                    style,
                },
                sources,
            )
            .unwrap();
            assert_eq!(diagnostic.severity, Severity::Warning);
            assert_eq!(diagnostic.code, "warning.ambiguous-call");
        }
    }
}
