//! Rust callbacks exercise the scalar C ABI; external C/backend proof is separate.
use fipe::{
    check, compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    ir::Type,
    lex::lex,
    mir::Program,
    parse::parse,
    token::Magnitude,
    value::{Reason, Value},
    vm::{Fault, Machine},
};
use fipe_ffi::{abi, Error, Provider};
use std::{cell::Cell, ffi::c_void, mem::size_of};

thread_local! {
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn descriptor(source: &str) -> abi::Provider {
    abi::Provider {
        version: abi::VERSION,
        size: size_of::<abi::Provider>(),
        module: abi::Bytes::new(b"probe"),
        declarations: abi::Bytes::new(source.as_bytes()),
        call: Some(dispatch),
    }
}

fn provider(descriptor: &abi::Provider) -> Provider<'_> {
    // Fixtures keep descriptor/buffers/code alive and use stateless synchronous callbacks.
    unsafe { Provider::new(descriptor) }.unwrap()
}

fn program(provider: &Provider<'_>, source: &str) -> Program {
    let mut declarations = Declarations::new();
    provider.register(&mut declarations).unwrap();
    let text = format!("use probe\n{source}");
    let module = parse(lex(&text)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    compile::compile(&unit)
}

fn request(program: &Program, preceding: &[Value]) -> Request {
    let mut machine = Machine::new(Some(10_000));
    let mut outcome = machine.run(program);
    for value in preceding {
        assert!(matches!(outcome, Outcome::Request(_)));
        outcome = machine.reply(program, Reply::Value(Ok(value.clone())));
    }
    match outcome {
        Outcome::Request(request) => request,
        other @ Outcome::Done(_) => panic!("expected request: {other:?}"),
    }
}

fn run(
    provider: &mut Provider<'_>,
    source: &str,
    authorized: bool,
) -> Result<Value, Fault> {
    let program = program(provider, source);
    let mut machine = Machine::new(Some(10_000));
    let mut outcome = machine.run(&program);
    loop {
        match outcome {
            Outcome::Done(result) => return result,
            Outcome::Request(request) => {
                let answer = if authorized {
                    provider
                        .call(&request, &program.tables.borrow())
                        .map_err(|error| error.to_string())
                } else {
                    Err(String::from("denied by driver"))
                };
                outcome = machine.reply(&program, Reply::Value(answer));
            }
        }
    }
}

fn contract<T>(result: Result<T, Error>) -> &'static str {
    match result {
        Err(Error::Contract(message)) => message,
        Err(other) => panic!("expected contract failure: {other}"),
        Ok(_) => panic!("expected contract failure"),
    }
}

fn tagged(tag: u32) -> abi::Value {
    abi::Value {
        tag,
        ..abi::Value::default()
    }
}

unsafe extern "C" fn dispatch(
    name: abi::Bytes,
    arguments: *const abi::Value,
    len: usize,
    emit: abi::Emit,
    context: *mut c_void,
) -> u32 {
    CALLS.with(|calls| calls.set(calls.get() + 1));
    // Adapter-owned label/array remain live throughout this synchronous call.
    let name = unsafe { std::slice::from_raw_parts(name.data, name.len) };
    let arguments = unsafe { std::slice::from_raw_parts(arguments, len) };
    let mut status = 0;
    let invalid_utf8 = [0xff];
    let value = match name {
        b"echo" if arguments.len() == 1 && arguments[0].tag <= abi::TEXT => {
            arguments[0]
        }
        b"sum"
            if arguments.len() == 2
                && arguments.iter().all(|v| v.tag == abi::INTEGER) =>
        {
            arguments[0]
                .integer
                .checked_add(arguments[1].integer)
                .map_or_else(
                    || {
                        status = 1;
                        abi::Value::text("sum overflow")
                    },
                    abi::Value::integer,
                )
        }
        b"zero" => abi::Value::integer(0),
        b"none" => return 0,
        b"fault" => {
            status = 1;
            abi::Value::text("provider refused")
        }
        b"fault_number" => {
            status = 1;
            abi::Value::integer(1)
        }
        b"unknown_status" => {
            status = 8;
            abi::Value::default()
        }
        b"wrong" => abi::Value::text("not an integer"),
        b"nan" => abi::Value::float(f64::NAN),
        b"infinite" => abi::Value::float(f64::INFINITY),
        b"invalid_utf8" => abi::Value {
            tag: abi::TEXT,
            bytes: abi::Bytes::new(&invalid_utf8),
            ..abi::Value::default()
        },
        b"null_text" | b"long_text" => abi::Value {
            tag: abi::TEXT,
            bytes: abi::Bytes {
                data: if name == b"null_text" {
                    std::ptr::null()
                } else {
                    invalid_utf8.as_ptr()
                },
                len: if name == b"null_text" { 1 } else { usize::MAX },
            },
            ..abi::Value::default()
        },
        b"empty_text" => tagged(abi::TEXT),
        b"old_bytes" => tagged(4),
        b"old_sequence" => tagged(5),
        b"old_tuple" => tagged(6),
        b"old_record" => tagged(7),
        b"unknown_tag" => tagged(99),
        b"null_value" => {
            // Rejected before dereference; no arbitrary dangling pointer is tested.
            unsafe { emit(context, std::ptr::null()) };
            return 0;
        }
        b"misaligned_value" => {
            let aligned = [abi::Value::default(); 2];
            let misaligned =
                aligned.as_ptr().cast::<u8>().wrapping_add(1).cast();
            // Within allocated storage, but rejected for alignment before dereference.
            unsafe { emit(context, misaligned) };
            return 0;
        }
        b"owned_text" => {
            let text = String::from("copied while the provider owns it");
            let value = abi::Value::text(&text);
            unsafe { emit(context, &raw const value) };
            return 0;
        }
        b"duplicate" => {
            let value = abi::Value::default();
            unsafe { emit(context, &raw const value) };
            value
        }
        _ => {
            status = 1;
            abi::Value::text("unknown function or arguments")
        }
    };
    // All reply data is still live; malformed cases exercise defined metadata errors.
    unsafe { emit(context, &raw const value) };
    status
}

#[test]
fn checked_scalars_round_trip_without_coercion() {
    for (ty, expression, expected) in [
        ("()", "()", Value::Unit),
        ("Integer", "-9223372036854775808", Value::Int(i64::MIN)),
        ("Integer", "9223372036854775807", Value::Int(i64::MAX)),
        ("I64", "7", Value::Int(7)),
        ("F64", "2.5", Value::F64(2.5)),
        ("String", "\"한\\0é\"", Value::text("한\0é")),
    ] {
        let source = format!("pub echo: {ty} -> {ty}");
        let descriptor = descriptor(&source);
        let mut provider = provider(&descriptor);
        assert_eq!(provider.module(), "probe");
        assert_eq!(provider.declarations(), source);
        assert_eq!(
            run(&mut provider, &format!("probe.echo {expression}"), true),
            Ok(expected)
        );
    }
    let descriptor = descriptor("pub echo: F64 -> F64");
    let Value::F64(zero) =
        run(&mut provider(&descriptor), "probe.echo -0.0", true).unwrap()
    else {
        panic!("F64 reply");
    };
    assert_eq!(zero.to_bits(), (-0.0_f64).to_bits());
    let descriptor = self::descriptor(
        "pub zero: Integer\npub sum: Integer -> Integer -> Integer",
    );
    let mut provider = provider(&descriptor);
    assert_eq!(run(&mut provider, "probe.zero", true), Ok(Value::Int(0)));
    assert_eq!(
        run(&mut provider, "probe.sum 7 8", true),
        Ok(Value::Int(15))
    );
}

#[test]
fn integer_range_and_driver_denial_prevent_native_side_effects() {
    let descriptor = descriptor("pub echo: Integer -> Integer");
    let mut provider = provider(&descriptor);
    for number in ["9223372036854775808", "-9223372036854775809"] {
        CALLS.set(0);
        let fault = run(&mut provider, &format!("probe.echo {number}"), true)
            .unwrap_err();
        assert!(
            matches!(fault.reason, Reason::Host(message) if message.contains("i64 range"))
        );
        assert_eq!(CALLS.get(), 0);
    }
    CALLS.set(0);
    assert!(run(&mut provider, "probe.echo 1", false).is_err());
    assert_eq!(CALLS.get(), 0);
    assert_eq!(run(&mut provider, "probe.echo 1", true), Ok(Value::Int(1)));
    assert_eq!(CALLS.get(), 1);
}

#[test]
fn opaque_inputs_outputs_and_shadowed_primitive_names_never_reach_native_code()
{
    let descriptor = descriptor("pub type Handle\npub handle: Handle\npub id: Handle -> Integer\npub make: Integer -> Handle");
    let mut provider = provider(&descriptor);
    for backing in [Value::text("opaque token"), Value::Int(42)] {
        let program = program(&provider, "probe.id probe.handle");
        let request = request(&program, &[backing]);
        CALLS.set(0);
        assert!(contract(provider.call(&request, &program.tables.borrow()))
            .contains("declaration type"));
        assert_eq!(CALLS.get(), 0);
    }
    for source in ["probe.handle", "probe.make 1"] {
        CALLS.set(0);
        assert!(run(&mut provider, source, true).is_err());
        assert_eq!(CALLS.get(), 0);
    }
    for name in ["Integer", "F64", "String"] {
        let source = format!(
            "pub type {name}\npub input: {name}\npub echo: {name} -> {name}"
        );
        let descriptor = self::descriptor(&source);
        let mut provider = self::provider(&descriptor);
        let program = program(&provider, "probe.echo probe.input");
        let request = request(&program, &[Value::text("host value")]);
        CALLS.set(0);
        assert!(provider.call(&request, &program.tables.borrow()).is_err());
        assert_eq!(CALLS.get(), 0);
    }
}

#[test]
fn checked_structured_f32_and_higher_order_calls_are_refused() {
    for (declarations, source) in [
        (
            "pub echo: (Integer * Integer) -> Integer",
            "probe.echo (1, 2)",
        ),
        ("pub echo: Vec<Integer> -> Integer", "probe.echo [1, 2]"),
        (
            "pub type Point { pub x: Integer }\npub echo: Point -> Integer",
            "probe.echo (probe.Point 1)",
        ),
        ("pub echo: F32 -> Integer", "x: F32 = 1.0\nprobe.echo x"),
        (
            "pub echo: (Integer -> Integer) -> Integer",
            "probe.echo (x => x)",
        ),
        ("pub zero: F32", "probe.zero"),
        ("pub echo: Integer -> (Integer * Integer)", "probe.echo 1"),
    ] {
        let descriptor = descriptor(declarations);
        let mut provider = provider(&descriptor);
        CALLS.set(0);
        assert!(run(&mut provider, source, true).is_err(), "{source}");
        assert_eq!(CALLS.get(), 0);
    }
}

#[test]
fn malformed_requests_and_unresolved_types_are_refused_before_invocation() {
    let descriptor = descriptor("pub echo: Integer -> Integer");
    let mut provider = provider(&descriptor);
    let program = program(&provider, "probe.echo 1");
    let original = request(&program, &[]);
    let tables = program.tables.borrow();
    let Request::Call { declaration, .. } = &original else {
        panic!("call")
    };
    let original_declaration = *declaration;
    for ty in [
        Type::Parameter(0),
        Type::Session(0),
        Type::Number(Magnitude::from(1_u64)),
        Type::Tuple(vec![]),
        Type::Array {
            element: Box::new(Type::Unit),
            length: Box::new(Type::Number(Magnitude::from(0_u64))),
        },
        Type::Packed(vec![]),
        Type::All(vec![]),
        Type::Not(Box::new(Type::Unit)),
        Type::Sum(vec![]),
    ] {
        let mut changed = tables.clone();
        changed.declarations[*declaration].ty = ty;
        CALLS.set(0);
        assert!(provider.call(&original, &changed).is_err());
        assert_eq!(CALLS.get(), 0);
    }
    let mut changed = tables.clone();
    let Type::Function { parameter, .. } =
        &mut changed.declarations[original_declaration].ty
    else {
        panic!("function signature");
    };
    let Type::Meaning { arguments, .. } = &mut **parameter else {
        panic!("Integer meaning");
    };
    arguments.push(Type::Unit);
    assert!(provider.call(&original, &changed).is_err());
    let mut changed = tables.clone();
    changed.meanings.clear();
    assert!(provider.call(&original, &changed).is_err());
    for arguments in [
        vec![],
        vec![Value::Int(1), Value::Int(2)],
        vec![Value::text("wrong")],
        vec![Value::F64(f64::NAN)],
    ] {
        let mut changed = original.clone();
        let Request::Call {
            arguments: actual, ..
        } = &mut changed
        else {
            panic!("call")
        };
        *actual = arguments;
        CALLS.set(0);
        assert!(provider.call(&changed, &tables).is_err());
        assert_eq!(CALLS.get(), 0);
    }
    let mut wrong = original.clone();
    let Request::Call { declaration, .. } = &mut wrong else {
        panic!("call")
    };
    *declaration = usize::MAX;
    assert_eq!(
        contract(provider.call(&wrong, &tables)),
        "unknown host declaration"
    );
    let mut changed = tables.clone();
    let namespace = changed.declarations[original_declaration].namespace;
    changed.namespaces[namespace] = String::from("other");
    assert!(contract(provider.call(&original, &changed))
        .contains("different provider"));
    assert!(provider
        .call(
            &Request::File {
                stem: String::from("probe")
            },
            &tables
        )
        .is_err());
}

#[test]
fn descriptor_validation_and_duplicate_registration_precede_calls() {
    let source = "pub zero: Integer";
    let mut descriptor = descriptor(source);
    CALLS.set(0);
    descriptor.version = 1;
    assert!(contract(unsafe { Provider::new(&descriptor) })
        .contains("version or size"));
    descriptor.version = abi::VERSION;
    descriptor.size -= 1;
    assert!(contract(unsafe { Provider::new(&descriptor) })
        .contains("version or size"));
    descriptor.size = size_of::<abi::Provider>();
    descriptor.call = None;
    assert!(contract(unsafe { Provider::new(&descriptor) })
        .contains("no call function"));
    descriptor.call = Some(dispatch);
    let invalid_utf8 = [0xff];
    for bytes in [
        abi::Bytes::new(&invalid_utf8),
        abi::Bytes {
            data: std::ptr::null(),
            len: 1,
        },
        abi::Bytes {
            data: invalid_utf8.as_ptr(),
            len: usize::MAX,
        },
    ] {
        descriptor.module = bytes;
        assert!(unsafe { Provider::new(&descriptor) }.is_err());
        descriptor.module = abi::Bytes::new(b"probe");
        descriptor.declarations = bytes;
        assert!(unsafe { Provider::new(&descriptor) }.is_err());
        descriptor.declarations = abi::Bytes::new(source.as_bytes());
    }
    descriptor.declarations = abi::Bytes::default();
    assert!(unsafe { Provider::new(&descriptor) }.is_ok());
    descriptor.declarations = abi::Bytes::new(source.as_bytes());
    let provider = provider(&descriptor);
    let mut declarations = Declarations::new();
    provider.register(&mut declarations).unwrap();
    assert!(contract(provider.register(&mut declarations))
        .contains("already registered"));
    assert_eq!(CALLS.get(), 0);
}

#[test]
fn malformed_scalar_replies_have_explicit_contract_failures() {
    let invalid_tag = "unknown native tag or nonfinite float";
    for (name, expected) in [
        ("nan", invalid_tag),
        ("infinite", invalid_tag),
        ("invalid_utf8", "native text is not UTF-8"),
        ("null_text", "invalid native slice"),
        ("long_text", "invalid native slice"),
        ("null_value", "invalid native slice"),
        ("misaligned_value", "invalid native slice"),
        ("none", "provider emitted no reply"),
        ("duplicate", "provider emitted more than one reply"),
        ("unknown_status", "unknown provider call status"),
        ("fault_number", "a host Fault must be UTF-8 text"),
        ("old_bytes", invalid_tag),
        ("old_sequence", invalid_tag),
        ("old_tuple", invalid_tag),
        ("old_record", invalid_tag),
        ("unknown_tag", invalid_tag),
    ] {
        let source = format!("pub {name}: Integer");
        let descriptor = descriptor(&source);
        let mut provider = provider(&descriptor);
        let program = program(&provider, &format!("probe.{name}"));
        let request = request(&program, &[]);
        CALLS.set(0);
        assert_eq!(
            contract(provider.call(&request, &program.tables.borrow())),
            expected,
            "{name}"
        );
        assert_eq!(CALLS.get(), 1, "{name}");
    }
    let descriptor =
        descriptor("pub empty_text: String\npub owned_text: String");
    let mut provider = provider(&descriptor);
    assert_eq!(
        run(&mut provider, "probe.empty_text", true),
        Ok(Value::text(""))
    );
    assert_eq!(
        run(&mut provider, "probe.owned_text", true),
        Ok(Value::text("copied while the provider owns it"))
    );
}

#[test]
fn wrong_scalar_and_provider_faults_keep_the_original_call_location() {
    for (name, expected) in [
        ("wrong", "the reply is not an Integer"),
        ("fault", "provider refused"),
    ] {
        let source = format!("pub {name}: Integer");
        let descriptor = descriptor(&source);
        let fault =
            run(&mut provider(&descriptor), &format!("probe.{name}"), true)
                .unwrap_err();
        assert_eq!(fault.reason, Reason::Host(String::from(expected)));
        assert_eq!(fault.span.start, "use probe\n".len());
        assert_eq!(fault.span.end, format!("use probe\nprobe.{name}").len());
    }
}

#[test]
fn nonfinite_float_arguments_are_rejected_before_invocation() {
    let descriptor = descriptor("pub echo: F64 -> F64");
    let mut provider = provider(&descriptor);
    let program = program(&provider, "probe.echo 1.0");
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut request = request(&program, &[]);
        let Request::Call { arguments, .. } = &mut request else {
            panic!("call")
        };
        arguments[0] = Value::F64(number);
        CALLS.set(0);
        assert_eq!(
            contract(provider.call(&request, &program.tables.borrow())),
            "invalid native scalar argument"
        );
        assert_eq!(CALLS.get(), 0);
    }
}
