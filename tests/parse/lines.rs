//! What ends a line, what carries it over, and the hazard cases from other
//! languages that the conformance chapter lists.

use super::{errors, module, name, node, only};
use fipe::{
    ast::{ArgumentKind, BinaryOp, Entry, ExprKind, PipeOp},
    parse::{Expected, Reason},
    token::Source,
};

fn pipeline_ops(input: &str) -> Vec<PipeOp> {
    let (parsed, id) = only(input);
    let mut ops = Vec::new();
    let mut id = id;
    while let ExprKind::Pipe { op, left, .. } = node(&parsed, id) {
        ops.push(*op);
        id = *left;
    }
    ops.reverse();
    ops
}

#[test]
fn leading_and_trailing_operators_continue_a_line() {
    let vertical = "find x\n    |: cat\n    |! log\n    || root";
    let trailing = "find x |:\n    cat |!\n    log ||\n    root";
    let expected = [PipeOp::Good, PipeOp::Bad, PipeOp::Or];
    assert_eq!(pipeline_ops(vertical), expected);
    assert_eq!(pipeline_ops(trailing), expected);
    assert_eq!(pipeline_ops("find x |: cat |! log || root"), expected);

    let (parsed, id) = only("total = price\n    * count\n    + shipping");
    let ExprKind::Bind { value, .. } = node(&parsed, id) else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, *value),
        ExprKind::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
    for spelling in [
        "+", "-", "*", "%", "==", "!=", "<", "<=", ">", ">=", "|", "|:", "|!",
        "||", "&&",
    ] {
        let leading = format!("a\n{spelling} b");
        let trailing = format!("a {spelling}\nb");
        assert_eq!(module(&leading).entries.len(), 1, "{leading:?}");
        assert_eq!(module(&trailing).entries.len(), 1, "{trailing:?}");
    }
    let (parsed, id) = only("8\n/ 2");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Binary {
            op: BinaryOp::Divide,
            ..
        }
    ));
    let parsed = module("[a\n+ b]");
    let [Entry::Line(list)] = parsed.entries.as_slice() else {
        panic!()
    };
    let ExprKind::List(elements) = node(&parsed, *list) else {
        panic!()
    };
    assert_eq!(elements.len(), 1);
}

#[test]
fn a_new_line_starts_at_flags_literals_not_and_root_paths() {
    for (input, second) in [
        ("a\n-b", "-b"),
        ("a\n-5", "-5"),
        ("a\n!b", "!b"),
        ("ls /\nnext", "next"),
        ("ls\n/", "/"),
        ("ls\n/ -l", "/ -l"),
    ] {
        let result = fipe::parse::parse(fipe::lex::lex(input));
        match result {
            Ok(parsed) => {
                assert_eq!(parsed.entries.len(), 2, "{input:?}");
                let Entry::Line(id) = parsed.entries[1] else {
                    panic!()
                };
                assert_eq!(
                    Source::new(input).slice(parsed.expr(id).unwrap().span),
                    Some(second),
                    "{input:?}"
                );
            }
            Err(errors) => {
                // A flag cannot begin a line; the error sits on the new
                // line, not on a continuation of the previous one.
                assert!(matches!(second, "-b"), "{input:?}: {errors:?}");
                assert_eq!(
                    Source::new(input).slice(errors[0].span),
                    Some(second.split(' ').next().unwrap()),
                    "{input:?}"
                );
                assert_eq!(errors[0].span.start, 2, "{input:?}");
            }
        }
    }
    let (parsed, id) = only("a\n- b");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Binary {
            op: BinaryOp::Subtract,
            ..
        }
    ));
    // A leading `=>` continues the head of an arm.
    let (parsed, id) = only("pub inc = n\n    => n + 1");
    let ExprKind::Bind {
        public: true,
        value,
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *value) else {
        panic!()
    };
    assert_eq!(arms[0].patterns.len(), 1);
    let (parsed, id) = only("a\n=> b");
    assert!(matches!(node(&parsed, id), ExprKind::Arms(_)));
}

#[test]
fn a_leading_assign_continues_a_signature() {
    // A signature above its `= value` is one binding.
    let text = "describe: C -> String\n  = x => repr x.r";
    let (parsed, id) = only(text);
    let ExprKind::Bind {
        annotation: Some(annotation),
        value,
        ..
    } = node(&parsed, id)
    else {
        panic!("{:?}", node(&parsed, id))
    };
    let source = Source::new(text);
    assert_eq!(
        source.slice(parsed.ty(*annotation).unwrap().span),
        Some("C -> String")
    );
    assert!(matches!(node(&parsed, *value), ExprKind::Arms(_)));
    // A second `=` is an error at that `=`, on one line or leading.
    for (text, at) in [("x = 1 = 2", 6), ("x = 1\n  = 2", 8)] {
        let [error] = errors(text).try_into().unwrap();
        assert_eq!(error.reason, Reason::BindingValue, "{text:?}");
        assert_eq!(error.span.start, at, "{text:?}");
    }
    // A binding as an arm body or a block line is still a binding.
    only("f = x => y = 1");
    only("f = x => {\n  [a] = x\n  a\n}");
    // A first-line `=` and one after a blank line continue nothing.
    let reasons = |text: &str| -> Vec<Reason> {
        errors(text).into_iter().map(|e| e.reason).collect()
    };
    assert_eq!(reasons("= 2"), [Reason::Expected(Expected::Pattern)]);
    assert_eq!(
        reasons("describe: C -> String\n\n  = x => x"),
        [Reason::Expected(Expected::Pattern)]
    );
}

#[test]
fn a_blank_or_comment_line_ends_continuation() {
    for input in [
        "a\n\n|: b",
        "a\n# note\n|: b",
        "a |\n\nb",
        "a +\n\nb",
        "|: b",
        "use fs\n|: b",
        "{\n| b }",
    ] {
        let result = errors(input);
        assert!(
            matches!(
                result[0].reason,
                Reason::Expected(Expected::Expression | Expected::LineEnd)
            ),
            "{input:?}: {result:?}"
        );
    }
    let (parsed, id) = only("a |: # note\nb");
    assert!(matches!(node(&parsed, id), ExprKind::Pipe { .. }));
}

#[test]
fn a_comma_lives_at_the_end_of_a_line() {
    let (parsed, id) = only("f(x,\n  y)");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(arguments.len(), 2);
    module("[\n  1,\n  2,\n]\nP {\n  x: 1,\n  y: 2\n}\nf(\n)\ntype T {\n  a: A,\n  b: B\n}\nt: Vec<\n  u8\n> = []");
    for input in [
        "f(x\n  , y)",
        "[1\n2]",
        "P { x: 1\n y: 2 }",
        "type T { a: A\n b: B }",
        "impl T { a = 1\n b = 2 }",
    ] {
        let result = errors(input);
        assert_eq!(result[0].reason, Reason::CommaBeforeLineBreak, "{input:?}");
        assert_eq!(Source::new(input).slice(result[0].span), Some("\n"));
    }
}

#[test]
fn brackets_hold_lines_and_a_call_paren_touches_its_callee() {
    let (parsed, id) = only("(a\nb)");
    let ExprKind::Group(lines) = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(lines.len(), 2);
    let (parsed, id) = only("(f x\n y)");
    let ExprKind::Group(lines) = node(&parsed, id) else {
        panic!()
    };
    assert!(matches!(node(&parsed, lines[0]), ExprKind::Call { .. }));
    name(&parsed, lines[1], "y");

    let (parsed, id) = only("f (\n  a\n  b\n)");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    let [argument] = arguments.as_slice() else {
        panic!()
    };
    let ArgumentKind::Positional(group) = &argument.kind else {
        panic!()
    };
    let ExprKind::Group(lines) = node(&parsed, *group) else {
        panic!()
    };
    assert_eq!(lines.len(), 2);

    let parsed = module("f\n(g)");
    assert_eq!(parsed.entries.len(), 2);
    let (parsed, id) = only("cat x \\\n    y");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(arguments.len(), 2);
    let parsed = module("cat x # note \\\n    y");
    assert_eq!(parsed.entries.len(), 2);
    let (parsed, id) = only("{ cat x }");
    let ExprKind::Block(lines) = node(&parsed, id) else {
        panic!()
    };
    assert!(matches!(node(&parsed, lines[0]), ExprKind::Call { .. }));
    let (parsed, id) = only("{ cat\nx }");
    let ExprKind::Block(lines) = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(lines.len(), 2);
    let (parsed, id) = only("(\n)");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Literal(fipe::ast::Literal::Unit)
    ));
}

#[test]
fn records_are_recognised_by_lookahead_and_kept_out_of_heads() {
    let (parsed, id) = only("Point { x: 1, y: 2 }");
    assert!(matches!(node(&parsed, id), ExprKind::Record { .. }));
    let (parsed, id) = only("Point {\n  x: 1,\n}");
    assert!(matches!(node(&parsed, id), ExprKind::Record { .. }));
    let (parsed, id) = only("Point {}");
    let ExprKind::Record { fields, .. } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(fields.len(), 0);
    // A record opens after an uppercase name; `{ x }` puns the field.
    let (parsed, id) = only("Self { x, y: 2 }");
    let ExprKind::Record { name: head, fields } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(head.segments, ["Self"]);
    assert_eq!(fields[0].name.segments, ["x"]);
    name(&parsed, fields[0].value, "x");
    assert_eq!(fields[1].name.segments, ["y"]);
    let (parsed, id) = only("map { x }");
    assert!(matches!(node(&parsed, id), ExprKind::Call { .. }));
    let (parsed, id) = only("map { [:e] => e.name, [!] => \"-\" }");
    let ExprKind::Call {
        callee, arguments, ..
    } = node(&parsed, id)
    else {
        panic!("{parsed:?}")
    };
    name(&parsed, *callee, "map");
    let ArgumentKind::Positional(block) = arguments[0].kind else {
        panic!()
    };
    assert!(matches!(node(&parsed, block), ExprKind::Arms(_)));
    let (parsed, id) = only("f { value }");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    let ArgumentKind::Positional(block) = arguments[0].kind else {
        panic!()
    };
    let ExprKind::Block(lines) = node(&parsed, block) else {
        panic!()
    };
    name(&parsed, lines[0], "value");
    let input = "if p == Point { x: 1 } { y }";
    let result = errors(input);
    assert_eq!(result[0].reason, Reason::RecordInHead);
    assert_eq!(Source::new(input).slice(result[0].span), Some("{"));
    assert_eq!(result[0].span.start, 14);
    module("if p == (Point { x: 1 }) { y }");
}
