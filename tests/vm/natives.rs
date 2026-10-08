use super::run::shown;
use super::{check, lex, parse, Declarations};
use fipe::vm::Natives;

/// The labels the prelude declares: its words and its members.
fn prelude_labels() -> Vec<String> {
    let mut labels = Vec::new();
    let mut in_impl = false;
    let mut in_type = false;
    for line in include_str!("../../prelude/prelude.fipe").lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if in_type {
            in_type = trimmed != "}";
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("pub ") {
            if rest.starts_with("type ") {
                in_type = rest.ends_with('{');
                continue;
            }
            if let Some((label, _)) = rest.split_once(':') {
                labels.push(label.trim().to_string());
            }
            continue;
        }
        if trimmed.starts_with("impl ") {
            if let Some((_, members)) = trimmed.split_once('{') {
                for member in members.trim_end_matches('}').split(',') {
                    if let Some((label, _)) = member.split_once(':') {
                        let label = label.trim();
                        if !label.is_empty() {
                            labels.push(label.to_string());
                        }
                    }
                }
                in_impl = !trimmed.ends_with('}');
            }
            continue;
        }
        if in_impl {
            if trimmed == "}" {
                in_impl = false;
                continue;
            }
            if let Some((label, _)) = trimmed.split_once(':') {
                labels.push(label.trim().to_string());
            }
        }
    }
    labels.sort();
    labels.dedup();
    labels
}

#[test]
fn every_prelude_declaration_has_a_native_or_a_named_pending_one() {
    let labels = prelude_labels();
    assert!(labels.len() > 50, "{labels:?}");
    for label in &labels {
        assert!(Natives::known(label), "the prelude's {label} has no native");
    }
    let module = parse(lex("1 + 2")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    let natives = Natives::new(&tables);
    assert!(natives.covers(&tables));
    let module = parse(lex("\"a b\" | words")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    assert_eq!(Natives::new(&tables).pending(), ["words"]);
}

#[test]
fn maybe_and_result_members_run() {
    assert_eq!(shown("Just 1 | map (n => n + 1)"), "Just 2");
    assert_eq!(shown("Nothing | map (n => n + 1)"), "Nothing");
    assert_eq!(shown("Just 1 |: (n => Just (n + 1))"), "Just 2");
    assert_eq!(shown("Just 1 || Nothing"), "Just 1");
    assert_eq!(shown("Nothing || Just 2"), "Just 2");
}

#[test]
fn sequence_natives_fold_filter_and_sum() {
    assert_eq!(shown("[1, 2] | map (n => n + 1)"), "[2, 3]");
    assert_eq!(shown("[3, 1, 2] | fold 0 (a b => a + b)"), "6");
    assert_eq!(shown("[3, 1, 2] | filter (n => n > 1)"), "[3, 2]");
    assert_eq!(shown("[3, 1, 2] | sum"), "6");
    assert_eq!(shown("[1.5, 2.5] | sum"), "4.0");
    assert_eq!(shown("[1, 2, 3] | take 2"), "[1, 2]");
    assert_eq!(shown("[1, 2, 3] | first"), "Just 1");
    assert_eq!(shown("[] | first"), "Nothing");
    assert_eq!(shown("[1, 2, 3] | head"), "1");
    assert_eq!(shown("len [1, 2, 3]"), "3");
    assert_eq!(shown("len \"abc\""), "3");
    assert_eq!(shown("[1] ++ [2]"), "[1, 2]");
}

#[test]
fn text_numbers_and_words() {
    assert_eq!(shown("\"3\" | read<I64>"), "Ok 3");
    assert_eq!(
        shown("\"x\" | read<I64>"),
        "Err (ParseError \"not an integer: \\\"x\\\"\")"
    );
    assert_eq!(shown("!True"), "False");
    assert_eq!(shown("fault \"stop\""), "fault: stop");
    assert_eq!(shown("log 1"), "1");
    assert_eq!(shown("sqrt 4.0"), "2.0");
}
