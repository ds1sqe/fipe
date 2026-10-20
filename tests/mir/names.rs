//! A local that has a name in the source reaches it through its slot;
//! a temporary has none.
use super::{ir, program};

/// The text of each named local in every body, in body and local
/// order, and how many locals have no slot.
fn locals(source: &str) -> (Vec<String>, usize) {
    let (unit, program) = program(source);
    let tables = unit.tables();
    // Prelude bodies name their locals in the prelude's source.
    let home = tables.nodes[unit.main[0]].namespace;
    let mut names = Vec::new();
    let mut temporaries = 0;
    for body in &program.bodies {
        for local in &body.locals {
            match local.slot {
                Some(slot) => {
                    if let Some((namespace, span)) = tables.slots[slot].name {
                        if namespace == home {
                            names
                                .push(source[span.start..span.end].to_string());
                        }
                    }
                }
                None => temporaries += 1,
            }
        }
    }
    (names, temporaries)
}

#[test]
fn a_spot_is_named_by_its_parameter() {
    let (names, _) = locals("f = n => n + 1\nf 2");
    assert!(names.contains(&"n".to_string()), "{names:?}");
}

#[test]
fn a_local_binding_is_named_by_its_name() {
    let (names, _) = locals("{\n    x = 1\n    x\n}");
    assert_eq!(names, ["x"]);
}

#[test]
fn a_pattern_binding_is_named_by_its_name() {
    let (names, _) = locals("{\n    Just y = Just 1\n    y + 1\n}");
    assert_eq!(names, ["y"]);
}

#[test]
fn an_arm_binding_is_named_in_its_arm() {
    let (names, _) = locals("g = { 0 => 1, m => m }\ng 2");
    assert!(names.contains(&"m".to_string()), "{names:?}");
}

#[test]
fn a_capture_is_named_by_the_slot_it_captures() {
    let source = "outer = x => (y => (x, y))\nouter 1";
    let (unit, _) = program(source);
    let tables = unit.tables();
    let captured: Vec<_> = tables
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            ir::Kind::Function { captures, .. } if !captures.is_empty() => {
                Some(captures.clone())
            }
            _ => None,
        })
        .flatten()
        .map(|slot| {
            let (_, span) = tables.slots[slot].name.expect("a named capture");
            source[span.start..span.end].to_string()
        })
        .collect();
    assert_eq!(captured, ["x"]);
}

#[test]
fn every_name_is_a_name_and_temporaries_have_none() {
    let source = "f = n => {\n    x: U8 = n\n    (x, x + 1)\n}\nf 1";
    let (names, temporaries) = locals(source);
    assert!(temporaries > 0, "a body with arithmetic has temporaries");
    assert_ne!(names.len(), 0, "the body has named locals");
    for name in &names {
        assert!(
            name.chars().all(|c| c.is_alphanumeric() || c == '_'),
            "{name:?} is not a name"
        );
    }
}
