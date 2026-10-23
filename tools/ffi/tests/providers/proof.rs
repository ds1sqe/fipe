use fipe::{
    check, compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    lex::lex,
    parse::parse,
    value::{Reason, Value},
    vm::Machine,
};
use fipe_ffi::{abi, Provider};
use fipe_proof_rust_provider as rust_provider;
use std::mem::{align_of, offset_of, size_of};

extern "C" {
    fn proof_c_descriptor() -> abi::Provider;
    fn proof_c_bad_version() -> abi::Provider;
    fn proof_c_bad_size() -> abi::Provider;
    fn proof_c_calls() -> usize;
    fn proof_c_layout(index: usize) -> usize;
}

#[derive(Debug, PartialEq)]
struct Trace {
    label: String,
    arguments: Vec<Value>,
    reply: Result<Value, String>,
}

#[derive(Debug, PartialEq)]
struct Failure {
    message: String,
    namespace: String,
    start: usize,
    end: usize,
}

#[derive(Debug, PartialEq)]
struct Run {
    result: Result<Value, Failure>,
    trace: Vec<Trace>,
}

fn c_calls() -> usize {
    // Single serial test owns the C counter; there are no overlapping calls.
    unsafe { proof_c_calls() }
}

fn run(provider: &mut Provider<'_>, source: &str, allowed: bool) -> Run {
    let mut declarations = Declarations::new();
    provider.register(&mut declarations).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(10_000));
    let mut outcome = machine.run(&program);
    let mut trace = Vec::new();
    loop {
        match outcome {
            Outcome::Request(request) => {
                let Request::Call {
                    declaration,
                    arguments,
                    ..
                } = &request
                else {
                    panic!("fully registered proof requests only calls");
                };
                let label = program.tables.borrow().declarations[*declaration]
                    .label
                    .clone();
                let reply = if allowed {
                    provider
                        .call(&request, &program.tables.borrow())
                        .map_err(|error| error.to_string())
                } else {
                    Err(String::from("denied by driver"))
                };
                trace.push(Trace {
                    label,
                    arguments: arguments.clone(),
                    reply: reply.clone(),
                });
                outcome = machine.reply(&program, Reply::Value(reply));
            }
            Outcome::Done(result) => {
                let result = result.map_err(|fault| {
                    let Reason::Host(message) = fault.reason else {
                        panic!("expected located Host Fault");
                    };
                    Failure {
                        message,
                        namespace: program.tables.borrow().namespaces
                            [fault.namespace]
                            .clone(),
                        start: fault.span.start,
                        end: fault.span.end,
                    }
                });
                return Run { result, trace };
            }
        }
    }
}

fn layout() {
    let fields = [
        size_of::<abi::Bytes>(),
        align_of::<abi::Bytes>(),
        offset_of!(abi::Bytes, data),
        offset_of!(abi::Bytes, len),
        size_of::<abi::Value>(),
        align_of::<abi::Value>(),
        offset_of!(abi::Value, tag),
        offset_of!(abi::Value, integer),
        offset_of!(abi::Value, float),
        offset_of!(abi::Value, bytes),
        size_of::<abi::Provider>(),
        align_of::<abi::Provider>(),
        offset_of!(abi::Provider, version),
        offset_of!(abi::Provider, size),
        offset_of!(abi::Provider, module),
        offset_of!(abi::Provider, declarations),
        offset_of!(abi::Provider, call),
        abi::VERSION as usize,
        abi::UNIT as usize,
        abi::INTEGER as usize,
        abi::FLOAT as usize,
        abi::TEXT as usize,
    ];
    for (index, expected) in fields.into_iter().enumerate() {
        // C returns only fixed sizeof/alignment/offset/tag constants.
        assert_eq!(
            unsafe { proof_c_layout(index) },
            expected,
            "layout entry {index}"
        );
    }
}

fn verify(descriptor: &abi::Provider, calls: fn() -> usize) -> Vec<Run> {
    let before = calls();
    // Both fixture descriptors refer to immutable static bytes and live code.
    let mut provider = unsafe { Provider::new(descriptor) }.unwrap();
    assert_eq!(provider.module(), "proof.ops");
    assert_eq!(provider.declarations(), include_str!("api.fipe"));
    let mut declarations = Declarations::new();
    provider.register(&mut declarations).unwrap();
    let selected = declarations.get_module("proof.ops").unwrap().clone();
    assert!(provider.register(&mut declarations).is_err());
    assert_eq!(declarations.get_module("proof.ops"), Some(&selected));
    let source = "use proof.ops\n(ops.unit (), ops.integer -9223372036854775808, ops.integer 9223372036854775807, ops.float -0.0, ops.text \"hé\\0x\", ops.choose 7 \"two args\")";
    // Checking/registration alone must never execute the provider.
    check::check(
        &parse(lex(source)).unwrap(),
        &declarations,
        &[],
        check::Options::default(),
    )
    .unwrap();
    assert_eq!(calls(), before);
    let success = run(&mut provider, source, true);
    let Value::Tuple(parts) = success.result.as_ref().unwrap() else {
        panic!("tuple result");
    };
    assert_eq!(parts.len(), 6);
    assert_eq!(parts[0], Value::Unit);
    assert_eq!(parts[1], Value::Int(i64::MIN));
    assert_eq!(parts[2], Value::Int(i64::MAX));
    let Value::F64(zero) = parts[3] else {
        panic!("F64 result");
    };
    assert_eq!(zero.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(parts[4], Value::text("hé\0x"));
    assert_eq!(parts[5], Value::text("two args"));
    assert_eq!(
        success
            .trace
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["unit", "integer", "integer", "float", "text", "choose"]
    );
    assert_eq!(
        success.trace[5].arguments,
        [Value::Int(7), Value::text("two args")]
    );
    assert_eq!(calls(), before + 6);
    let extra = run(
        &mut provider,
        "use proof.ops\n(ops.float 1.25, ops.text \"\")",
        true,
    );
    assert_eq!(
        extra.result,
        Ok(Value::Tuple(std::rc::Rc::new(vec![
            Value::F64(1.25),
            Value::text("")
        ])))
    );
    let mut results = vec![success, extra];
    for (label, message) in [
        ("fail", "provider refused"),
        ("wrong", "the reply is not an Integer"),
    ] {
        let source = format!("use proof.ops\nops.{label} ()");
        let result = run(&mut provider, &source, true);
        let fault = result.result.as_ref().unwrap_err();
        assert_eq!(fault.message, message);
        assert_eq!(fault.namespace, "main");
        assert_eq!(&source[fault.start..fault.end], format!("ops.{label} ()"));
        if label == "wrong" {
            assert!(result.trace[0].reply.is_ok());
        }
        results.push(result);
    }
    assert_eq!(calls(), before + 10);
    let denied = run(&mut provider, "use proof.ops\nops.integer 7", false);
    assert_eq!(
        denied.result.as_ref().unwrap_err().message,
        "denied by driver"
    );
    assert_eq!(calls(), before + 10);
    results.push(denied);
    results
}

#[test]
fn independently_compiled_c_and_rust_providers_match() {
    layout();
    // C functions return complete by-value descriptors backed by static storage.
    let (c, bad_version, bad_size) = unsafe {
        (
            proof_c_descriptor(),
            proof_c_bad_version(),
            proof_c_bad_size(),
        )
    };
    let rust = rust_provider::descriptor();
    for (descriptor, calls) in [
        (&c, c_calls as fn() -> usize),
        (&rust, rust_provider::calls),
    ] {
        let before = calls();
        let mut wrong_version = *descriptor;
        wrong_version.version = 1;
        let mut wrong_size = *descriptor;
        wrong_size.size -= 1;
        for invalid in [wrong_version, wrong_size] {
            // Invalid version/size do not truncate storage or invalidate its buffers.
            assert!(unsafe { Provider::new(&invalid) }.is_err());
        }
        assert_eq!(calls(), before);
    }
    let before = c_calls();
    for invalid in [bad_version, bad_size] {
        // Same complete storage and valid static buffers, constructed in C.
        assert!(unsafe { Provider::new(&invalid) }.is_err());
    }
    assert_eq!(c_calls(), before);
    assert_eq!(verify(&c, c_calls), verify(&rust, rust_provider::calls));
}
