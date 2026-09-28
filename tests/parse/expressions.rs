use super::{errors, integer, module, name, node, only};
use fipe::{
    ast::{
        ArgumentKind, BinaryOp, CallStyle, Entry, ExprKind, Literal,
        PatternKind, PipeOp,
    },
    parse::Reason,
    token::Source,
};

#[test]
fn all_pipes_associate_left_at_one_level() {
    for (spelling, expected) in [
        ("|", PipeOp::Plain),
        ("|:", PipeOp::Good),
        ("|!", PipeOp::Bad),
        ("||", PipeOp::Or),
        ("&&", PipeOp::And),
    ] {
        let (parsed, id) = only(&format!("a || b {spelling} c"));
        let ExprKind::Pipe {
            op, left, right, ..
        } = node(&parsed, id)
        else {
            panic!("expected outer pipe");
        };
        assert_eq!(*op, expected);
        name(&parsed, *right, "c");
        let ExprKind::Pipe {
            op: PipeOp::Or,
            left,
            right,
            ..
        } = node(&parsed, *left)
        else {
            panic!("expected left-associated fallback");
        };
        name(&parsed, *left, "a");
        name(&parsed, *right, "b");
    }
}

#[test]
fn greater_compares_everywhere_and_writes_are_commands() {
    for input in ["x > y", "(x > y)", "[x > y]", "if x > y { a }"] {
        let parsed = module(input);
        assert!(
            format!("{parsed:?}").contains("Greater"),
            "{input}: {parsed:?}"
        );
    }
    let input = "cat x > y";
    let errors = errors(input);
    assert_eq!(errors[0].reason, Reason::OperatorInArguments);
    assert_eq!(Source::new(input).slice(errors[0].span), Some(">"));
    let (parsed, id) = only("cat x | put ./out");
    let ExprKind::Pipe {
        op: PipeOp::Plain,
        right,
        ..
    } = node(&parsed, id)
    else {
        panic!("expected a pipe into the host command");
    };
    let ExprKind::Call { callee, .. } = node(&parsed, *right) else {
        panic!()
    };
    name(&parsed, *callee, "put");
    let input = "a >> b";
    let result = super::errors(input);
    assert_eq!(
        result[0].reason,
        Reason::Expected(fipe::parse::Expected::Expression)
    );
    assert_eq!(Source::new(input).slice(result[0].span), Some(">"));
    assert_eq!(result[0].span.start, 3);
}

#[test]
fn call_styles_keep_argument_order_and_explicit_empty_calls() {
    for (input, expected) in [
        ("f(1,2)", CallStyle::Parenthesized),
        ("f 1 2", CallStyle::Space),
    ] {
        let (parsed, id) = only(input);
        let ExprKind::Call {
            callee,
            arguments,
            style,
        } = node(&parsed, id)
        else {
            panic!()
        };
        name(&parsed, *callee, "f");
        assert_eq!(*style, expected);
        let [first, second] = arguments.as_slice() else {
            panic!()
        };
        let ArgumentKind::Positional(first) = first.kind else {
            panic!()
        };
        let ArgumentKind::Positional(second) = second.kind else {
            panic!()
        };
        integer(&parsed, first, 1);
        integer(&parsed, second, 2);
    }
    let (parsed, id) = only("f()");
    let ExprKind::Call {
        callee,
        arguments,
        style,
    } = node(&parsed, id)
    else {
        panic!()
    };
    name(&parsed, *callee, "f");
    assert_eq!(*style, CallStyle::Parenthesized);
    assert_eq!(arguments, &[]);
    let (parsed, id) = only("f (1)");
    let ExprKind::Call {
        arguments,
        style: CallStyle::Space,
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    let ArgumentKind::Positional(id) = arguments[0].kind else {
        panic!()
    };
    let ExprKind::Group(lines) = node(&parsed, id) else {
        panic!()
    };
    integer(&parsed, lines[0], 1);
    assert_eq!(
        errors("f (1,2)")[0].reason,
        Reason::Expected(fipe::parse::Expected::LineEnd)
    );
}

#[test]
fn computed_callees_and_postfix_fields_remain_structural() {
    let (parsed, id) = only("f().name(2)");
    let ExprKind::Call {
        callee, arguments, ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    let ArgumentKind::Positional(argument) = arguments[0].kind else {
        panic!()
    };
    integer(&parsed, argument, 2);
    let ExprKind::Field { value, field } = node(&parsed, *callee) else {
        panic!()
    };
    assert_eq!(field.segments, ["name"]);
    let ExprKind::Call {
        callee, arguments, ..
    } = node(&parsed, *value)
    else {
        panic!()
    };
    name(&parsed, *callee, "f");
    assert_eq!(arguments.as_slice(), []);

    let (parsed, id) = only("f .name");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    let ArgumentKind::Positional(id) = arguments[0].kind else {
        panic!()
    };
    let ExprKind::Selector(field) = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(field.segments, ["name"]);
    let (parsed, id) = only("(x => x)(2)");
    let ExprKind::Call { callee, .. } = node(&parsed, id) else {
        panic!()
    };
    assert!(matches!(node(&parsed, *callee), ExprKind::Arms(_)));
    assert_eq!(
        errors("(x).0")[0].reason,
        Reason::Expected(fipe::parse::Expected::LineEnd)
    );
}

#[test]
fn flags_retain_attached_values_and_separate_operands() {
    let input = "cmd -c=80 -l -n -5 \"-x\" -t=[a,b]";
    let (parsed, id) = only(input);
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(
        arguments
            .iter()
            .map(|arg| Source::new(input).slice(arg.span).unwrap())
            .collect::<Vec<_>>(),
        ["-c=80", "-l", "-n", "-5", "\"-x\"", "-t=[a,b]"]
    );
    let ArgumentKind::Flag {
        name: flag,
        value: Some(value),
    } = &arguments[0].kind
    else {
        panic!()
    };
    assert_eq!(flag, "c");
    integer(&parsed, *value, 80);
    assert_eq!(
        arguments[1].kind,
        ArgumentKind::Flag {
            name: "l".into(),
            value: None
        }
    );
    assert_eq!(
        arguments[2].kind,
        ArgumentKind::Flag {
            name: "n".into(),
            value: None
        }
    );
    let ArgumentKind::Positional(value) = arguments[3].kind else {
        panic!()
    };
    assert_eq!(
        node(&parsed, value),
        &ExprKind::Literal(Literal::Int {
            negative: true,
            magnitude: 5
        })
    );
    let ArgumentKind::Positional(value) = arguments[4].kind else {
        panic!()
    };
    assert_eq!(
        node(&parsed, value),
        &ExprKind::Literal(Literal::Str {
            value: "-x".into(),
            raw: false
        })
    );
    let ArgumentKind::Flag {
        name: flag,
        value: Some(value),
    } = &arguments[5].kind
    else {
        panic!()
    };
    assert_eq!(flag, "t");
    let ExprKind::List(values) = node(&parsed, *value) else {
        panic!()
    };
    let [a, b] = values.as_slice() else { panic!() };
    name(&parsed, *a, "a");
    name(&parsed, *b, "b");
    for broken in ["cmd -c =80", "cmd -c= 80", "cmd -c = 80"] {
        assert_eq!(errors(broken)[0].reason, Reason::FlagValueMustTouch);
    }
}

#[test]
fn lists_wrap_by_comma_and_groups_hold_lines() {
    let input = "[\nf(\n1,\n2\n),\n(\n3 +\n4\n),\nP {\nx: 1\n}\n]";
    let (parsed, id) = only(input);
    let ExprKind::List(values) = node(&parsed, id) else {
        panic!()
    };
    let [call, sum, record] = values.as_slice() else {
        panic!()
    };
    assert!(matches!(node(&parsed, *call), ExprKind::Call { .. }));
    let ExprKind::Group(lines) = node(&parsed, *sum) else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, lines[0]),
        ExprKind::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
    let ExprKind::Record { name, fields } = node(&parsed, *record) else {
        panic!()
    };
    assert_eq!(name.segments, ["P"]);
    assert_eq!(fields[0].name.segments, ["x"]);
    integer(&parsed, fields[0].value, 1);

    for input in ["({\nx = 1\nx\n})", "(\nx = 1\nx\n)"] {
        let (parsed, id) = only(input);
        let mut id = id;
        if let ExprKind::Group(lines) = node(&parsed, id) {
            if lines.len() == 1 {
                id = lines[0];
            }
        }
        let (ExprKind::Block(lines) | ExprKind::Group(lines)) =
            node(&parsed, id)
        else {
            panic!("{input}: {parsed:?}")
        };
        let [binding, value] = lines.as_slice() else {
            panic!()
        };
        assert!(matches!(node(&parsed, *binding), ExprKind::Bind { .. }));
        super::name(&parsed, *value, "x");
    }
}

#[test]
fn conditionals_attach_else_and_compare_in_the_head() {
    let (parsed, id) = only("if x > y { a }\nelse { b }");
    let ExprKind::If {
        condition,
        then_body,
        else_body: Some(else_body),
    } = node(&parsed, id)
    else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, *condition),
        ExprKind::Binary {
            op: BinaryOp::Greater,
            ..
        }
    ));
    let ExprKind::Block(then_lines) = node(&parsed, *then_body) else {
        panic!()
    };
    name(&parsed, then_lines[0], "a");
    let ExprKind::Block(else_lines) = node(&parsed, *else_body) else {
        panic!()
    };
    name(&parsed, else_lines[0], "b");
    let parsed = module("if a { if b { x }\nelse { y } }\nnext");
    let [Entry::Line(outer), Entry::Line(next)] = parsed.entries.as_slice()
    else {
        panic!()
    };
    name(&parsed, *next, "next");
    let ExprKind::If {
        then_body,
        else_body: None,
        ..
    } = node(&parsed, *outer)
    else {
        panic!()
    };
    let ExprKind::Block(lines) = node(&parsed, *then_body) else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, lines[0]),
        ExprKind::If {
            else_body: Some(_),
            ..
        }
    ));
}

#[test]
fn root_path_division_and_operator_arguments_are_distinct() {
    let (parsed, id) = only("ls /");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    let ArgumentKind::Positional(root) = arguments[0].kind else {
        panic!()
    };
    assert_eq!(node(&parsed, root), &ExprKind::Path("/".into()));
    let (parsed, id) = only("8 / 2");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Binary {
            op: BinaryOp::Divide,
            ..
        }
    ));
    module("cat (/) other\nls / -l");
    assert_eq!(errors("cat 1 / 2")[0].reason, Reason::OperatorInArguments);
}

#[test]
fn explicit_instantiation_is_an_operand() {
    let input = "value | Rd<fs>";
    let (parsed, id) = only(input);
    let ExprKind::Pipe { right, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Instance {
        name: head,
        arguments,
    } = node(&parsed, *right)
    else {
        panic!("{parsed:?}")
    };
    assert_eq!(
        Source::new(input).slice(parsed.expr(*right).unwrap().span),
        Some("Rd<fs>")
    );
    assert_eq!(head.segments, ["Rd"]);
    assert_eq!(arguments.len(), 1);
    let (parsed, id) = only("pl.into<Point>");
    let ExprKind::Instance { name: head, .. } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(head.segments, ["pl", "into"]);
    let (parsed, id) = only("\"42\" | read<i64> |: add 1");
    assert!(matches!(node(&parsed, id), ExprKind::Pipe { .. }));
    let (parsed, id) = only("(round x) < y");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Binary {
            op: BinaryOp::Less,
            ..
        }
    ));
    let (parsed, id) = only("a < b");
    assert!(matches!(node(&parsed, id), ExprKind::Binary { .. }));
    let (parsed, id) = only("f read<Vec<u8>> x");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(arguments.len(), 2);
    let input = "a<b";
    let errors = errors(input);
    assert_eq!(
        errors[0].reason,
        Reason::Expected(fipe::parse::Expected::Symbol(
            fipe::token::Symbol::Greater
        ))
    );
    assert_eq!(Source::new(input).slice(errors[0].span), Some(""));
    assert_eq!(module("!x").entries.len(), 1);
    let (parsed, id) = only("x as Bool");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!("`as` is an ordinary name: {parsed:?}")
    };
    assert_eq!(arguments.len(), 2);
}

#[test]
fn arms_keep_nested_patterns_guards_and_pipeline_bodies() {
    let input = "find x | {\nJust Entry { name: n, .. } if n != \"\" => n | show,\n_ => \"-\",\n}";
    let (parsed, id) = only(input);
    let ExprKind::Pipe { right, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *right) else {
        panic!()
    };
    let [first, fallback] = arms.as_slice() else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, first.guard.unwrap()),
        ExprKind::Binary {
            op: BinaryOp::NotEqual,
            ..
        }
    ));
    assert!(matches!(
        node(&parsed, first.body),
        ExprKind::Pipe {
            op: PipeOp::Plain,
            ..
        }
    ));
    let [head] = first.patterns.as_slice() else {
        panic!()
    };
    let PatternKind::Constructor { name, payload } =
        &parsed.pattern(*head).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(name.segments, ["Just"]);
    let PatternKind::Record {
        name,
        fields,
        rest: Some(rest),
    } = &parsed.pattern(*payload).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(name.segments, ["Entry"]);
    assert_eq!(fields[0].name.segments, ["name"]);
    assert_eq!(Source::new(input).slice(*rest), Some(".."));
    assert!(matches!(
        parsed.pattern(fallback.patterns[0]).unwrap().kind,
        PatternKind::Wildcard
    ));
    assert!(
        matches!(node(&parsed, fallback.body), ExprKind::Literal(Literal::Str { value, .. }) if value == "-")
    );
}

#[test]
fn arm_lists_are_comma_lists_in_either_bracket() {
    for input in [
        "find x | ([:e] => e.name, [!] => \"-\")",
        "find x | { [:e] => e.name, [!] => \"-\" }",
        "find x | {\n    [:e] =>\n        e.name,\n    [!] => \"-\",\n}",
        "find x | { [:e] => {\n    n = e.name\n    cat n\n},\n[!] => \"-\" }",
        "find x | [:e] => e.name,\n[!] => \"-\"",
    ] {
        let (parsed, id) = only(input);
        let ExprKind::Pipe { right, .. } = node(&parsed, id) else {
            panic!("{input}")
        };
        let ExprKind::Arms(arms) = node(&parsed, *right) else {
            panic!("{input}: {:?}", node(&parsed, *right))
        };
        assert_eq!(arms.len(), 2, "{input}");
    }
    let (parsed, id) = only("classify = { [:e] => 1, [!] => 0 }");
    let ExprKind::Bind { value, .. } = node(&parsed, id) else {
        panic!()
    };
    assert!(matches!(node(&parsed, *value), ExprKind::Arms(_)));
    for (input, offending, reason) in [
        (
            "find x | {\n    [:e] => e.name\n    [!] => \"-\"\n}",
            "\n",
            Reason::CommaBeforeLineBreak,
        ),
        (
            "find x | ([:e] => e.name [!] => \"-\")",
            "=>",
            Reason::ArmsNeedGroup,
        ),
        (
            "find x | { [:e] => e.name, ls }",
            "ls",
            Reason::MixedArmsAndLines,
        ),
        ("[x => x]", "=>", Reason::ArmsNeedGroup),
    ] {
        let result = errors(input);
        assert_eq!(result[0].reason, reason, "{input}");
        assert_eq!(
            Source::new(input).slice(result[0].span),
            Some(offending),
            "{input}"
        );
    }
}

#[test]
fn outcome_brackets_name_a_side_in_patterns_and_values() {
    let input = "m | { [:e] => [:e], [:] => [:], [!e] => [!e], [!] => [!] }";
    let (parsed, id) = only(input);
    let ExprKind::Pipe { right, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *right) else {
        panic!()
    };
    let expected = [(true, true), (true, false), (false, true), (false, false)];
    for (arm, (side, bound)) in arms.iter().zip(expected) {
        let PatternKind::Outcome { good, payload } =
            &parsed.pattern(arm.patterns[0]).unwrap().kind
        else {
            panic!()
        };
        assert_eq!((*good, payload.is_some()), (side, bound));
        let ExprKind::Outcome { good, payload } = node(&parsed, arm.body)
        else {
            panic!()
        };
        assert_eq!((*good, payload.is_some()), (side, bound));
    }
    let (parsed, id) = only("[!x]");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Outcome {
            good: false,
            payload: Some(_)
        }
    ));
    let (parsed, id) = only("[(!x)]");
    let ExprKind::List(elements) = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(elements.len(), 1);
    let (parsed, id) = only("[:Entry { name, .. }] = find x");
    let ExprKind::Bind { pattern, .. } = node(&parsed, id) else {
        panic!()
    };
    let PatternKind::Outcome {
        good: true,
        payload: Some(payload),
    } = &parsed.pattern(*pattern).unwrap().kind
    else {
        panic!()
    };
    assert!(matches!(
        parsed.pattern(*payload).unwrap().kind,
        PatternKind::Record { .. }
    ));
    module("[:[:e]]\n[:[!]] = x\n[:1 + 2]");
    assert_eq!(
        errors("[:1, 2]")[0].reason,
        Reason::Expected(fipe::parse::Expected::Symbol(
            fipe::token::Symbol::RBracket
        ))
    );
}

#[test]
fn pattern_bindings_and_scoped_values_preserve_structure() {
    let (parsed, id) = only("[x, [y], 0] = values");
    let ExprKind::Bind {
        pattern,
        annotation: None,
        value,
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    name(&parsed, *value, "values");
    let PatternKind::List(elements) = &parsed.pattern(*pattern).unwrap().kind
    else {
        panic!()
    };
    let [x, nested, zero] = elements.as_slice() else {
        panic!()
    };
    assert!(
        matches!(&parsed.pattern(*x).unwrap().kind, PatternKind::Name(name) if name.segments == ["x"])
    );
    assert!(matches!(
        parsed.pattern(*nested).unwrap().kind,
        PatternKind::List(_)
    ));
    assert_eq!(
        parsed.pattern(*zero).unwrap().kind,
        PatternKind::Literal(Literal::Int {
            negative: false,
            magnitude: 0
        })
    );
    let (parsed, id) = only("with cwd /tmp { ls }");
    let ExprKind::With { key, value, body } = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(key.segments, ["cwd"]);
    assert_eq!(node(&parsed, *value), &ExprKind::Path("/tmp".into()));
    let ExprKind::Block(lines) = node(&parsed, *body) else {
        panic!()
    };
    name(&parsed, lines[0], "ls");
}

#[test]
fn literals_retain_payloads_without_type_checking() {
    let (parsed, id) =
        only(r#"[(), -5, 1e999, "é", 'raw', b"\xFF", héllo, ./x]"#);
    let ExprKind::List(values) = node(&parsed, id) else {
        panic!()
    };
    assert_eq!(
        values
            .iter()
            .map(|id| node(&parsed, *id).clone())
            .collect::<Vec<_>>(),
        [
            ExprKind::Literal(Literal::Unit),
            ExprKind::Literal(Literal::Int {
                negative: true,
                magnitude: 5
            }),
            ExprKind::Literal(Literal::Float("1e999".into())),
            ExprKind::Literal(Literal::Str {
                value: "é".into(),
                raw: false
            }),
            ExprKind::Literal(Literal::Str {
                value: "raw".into(),
                raw: true
            }),
            ExprKind::Literal(Literal::Bytes(vec![255])),
            ExprKind::Word("héllo".into()),
            ExprKind::Path("./x".into()),
        ]
    );
    let (parsed, id) = only("Rd 4");
    let ExprKind::Call { callee, .. } = node(&parsed, id) else {
        panic!()
    };
    name(&parsed, *callee, "Rd");
}

#[test]
fn every_binary_operator_retains_its_kind_and_span() {
    for (spelling, expected) in [
        ("+", BinaryOp::Add),
        ("-", BinaryOp::Subtract),
        ("*", BinaryOp::Multiply),
        ("/", BinaryOp::Divide),
        ("%", BinaryOp::Remainder),
        ("==", BinaryOp::Equal),
        ("!=", BinaryOp::NotEqual),
        ("<", BinaryOp::Less),
        ("<=", BinaryOp::LessEqual),
        (">", BinaryOp::Greater),
        (">=", BinaryOp::GreaterEqual),
    ] {
        let input = format!("(a {spelling} b)");
        let (parsed, id) = only(&input);
        let ExprKind::Group(lines) = node(&parsed, id) else {
            panic!()
        };
        let ExprKind::Binary {
            op,
            operator,
            left,
            right,
        } = node(&parsed, lines[0])
        else {
            panic!()
        };
        assert_eq!(*op, expected);
        assert_eq!(Source::new(&input).slice(*operator), Some(spelling));
        name(&parsed, *left, "a");
        name(&parsed, *right, "b");
    }
}

#[test]
fn migrated_function_body_returns_its_last_line() {
    let parsed = module("call = foo => {\nflag = True\nfoo\n}\nhey = () => {\nhello = True\nhello\n}");
    let [Entry::Line(call), Entry::Line(hey)] = parsed.entries.as_slice()
    else {
        panic!()
    };
    for (binding, last) in [(call, "foo"), (hey, "hello")] {
        let ExprKind::Bind { value, .. } = node(&parsed, *binding) else {
            panic!()
        };
        let ExprKind::Arms(arms) = node(&parsed, *value) else {
            panic!()
        };
        let ExprKind::Block(lines) = node(&parsed, arms[0].body) else {
            panic!()
        };
        let [bound, value] = lines.as_slice() else {
            panic!()
        };
        assert!(matches!(node(&parsed, *bound), ExprKind::Bind { .. }));
        name(&parsed, *value, last);
    }
    let (parsed, id) = only("hey = () => 1");
    let ExprKind::Bind { value, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *value) else {
        panic!()
    };
    assert_eq!(
        parsed.pattern(arms[0].patterns[0]).unwrap().kind,
        PatternKind::Literal(Literal::Unit)
    );
}
