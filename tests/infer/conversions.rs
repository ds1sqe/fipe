use super::{errors, ty};
use fipe::infer::Reason;

#[test]
fn conversions_use_target_members_and_close_the_error_type() {
    let point = "type P: From<Integer> { n: Integer }\nimpl P: From<Integer> { from: Integer -> Self = n => Self n }\n";
    for call in ["2 | to<P>", "2 | P.from", "x: P = to 2"] {
        assert_eq!(ty(&format!("{point}{call}")), "P");
    }
    assert_eq!(ty(&format!("{point}2 | try<P>")), "Result<P, LimitError>");
    assert!(errors(&format!("{point}f = x => P.from x"))
        .iter()
        .any(|e| matches!(e.reason, Reason::Conversion { .. })));
}

#[test]
fn conversion_members_are_unique_per_source_meaning() {
    let text = "type P: From<Integer> & TryFrom<Integer, String> { n: Integer }\nimpl P: From<Integer> { from: Integer -> Self = n => Self n }\nimpl P: TryFrom<Integer, String> { try_from: Integer -> Result<Self, String> = n => Ok (Self n) }";
    assert!(errors(text)
        .iter()
        .any(|e| e.reason == Reason::ConflictingConversion));
    let text = "type P: From<Integer> & From<String> { n: Integer }\nimpl P: From<Integer> { from: Integer -> Self = n => Self n }\nimpl P: From<String> { from: String -> Self = s => Self 0 }\n";
    assert_eq!(ty(&format!("{text}2 | to<P>")), "P");
    assert_eq!(ty(&format!("{text}\"s\" | to<P>")), "P");
}

#[test]
fn imported_conversion_overloads_keep_every_source() {
    let declarations = fipe::decl::Declarations::new();
    let files = [("points".into(), super::module("pub type P: From<Integer> & From<String> { n: Integer }\nimpl P: From<Integer> { pub from: Integer -> Self = n => Self n }\nimpl P: From<String> { pub from: String -> Self = s => Self 0 }"))];
    for text in ["use points.P\n1 | to<P>", "use points.P\n\"s\" | to<P>"] {
        let module = super::module(text);
        let resolved =
            fipe::resolve::resolve(&module, &declarations, &files).unwrap();
        assert!(
            fipe::infer::infer(&module, &resolved, &declarations, &files)
                .is_ok()
        );
    }
}
