//! Explicit linked scalar providers: offline interface pairing is Host policy.
use fipe::{
    check, compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    lex::lex,
    parse::parse,
    value::Value,
    vm::Machine,
};
use fipe_ffi::{abi, Provider};
use fipe_linked_rust_provider as rust_provider;
use std::{env, fs, path::Path};

const MODULE: &str = "proof.ops";
const SCRIPT: &str = "use proof.ops\n(ops.integer 42, ops.text \"hello\")";

extern "C" {
    fn linked_c_descriptor() -> abi::Provider;
    #[cfg(test)]
    fn linked_c_calls() -> usize;
}

#[derive(Clone, Copy)]
enum Language {
    C,
    Rust,
}
impl Language {
    fn descriptor(self) -> abi::Provider {
        match self {
            // This descriptor and all its buffers/callbacks are in the linked image.
            Self::C => unsafe { linked_c_descriptor() },
            Self::Rust => rust_provider::descriptor(),
        }
    }
}

fn snapshot(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_absolute() {
        return Err("interface path must be absolute".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("cannot read interface: {error}"))?;
    std::str::from_utf8(&bytes).map_err(|_| "interface must be UTF-8".to_owned())?;
    Ok(bytes)
}

fn paired<'a>(
    descriptor: &'a abi::Provider,
    module: &str,
    saved: &[u8],
) -> Result<Provider<'a>, String> {
    std::str::from_utf8(saved).map_err(|_| "interface must be UTF-8".to_owned())?;
    // All callers supply immutable linked descriptors; tests change only valid
    // static metadata or version/size. Code and buffers outlive the Provider.
    let provider = unsafe { Provider::new(descriptor) }.map_err(|error| error.to_string())?;
    if provider.module() != module {
        return Err("provider module does not match selection".into());
    }
    if provider.declarations().as_bytes() != saved {
        return Err("provider declarations do not match saved interface bytes".into());
    }
    Ok(provider)
}

fn execute(provider: &mut Provider<'_>, source: &str, allowed: bool) -> Result<Value, String> {
    let mut declarations = Declarations::new();
    provider
        .register(&mut declarations)
        .map_err(|error| error.to_string())?;
    let module = parse(lex(source)).map_err(|error| format!("parse: {error:?}"))?;
    let unit = check::check(&module, &declarations, &[], check::Options::default())
        .map_err(|error| format!("check: {error:?}"))?;
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(10_000));
    let mut outcome = machine.run(&program);
    loop {
        match outcome {
            Outcome::Request(request @ Request::Call { .. }) => {
                let reply = {
                    let tables = program.tables.borrow();
                    let Request::Call {
                        declaration,
                        environment,
                        ..
                    } = &request
                    else {
                        unreachable!()
                    };
                    let declared = &tables.declarations[*declaration];
                    let permitted = allowed
                        && environment.is_empty()
                        && tables.namespaces[declared.namespace] == MODULE
                        && matches!(declared.label.as_str(), "integer" | "text" | "fail");
                    if permitted {
                        provider
                            .call(&request, &tables)
                            .map_err(|error| error.to_string())
                    } else {
                        Err("denied by driver".into())
                    }
                };
                outcome = machine.reply(&program, Reply::Value(reply));
            }
            Outcome::Request(Request::File { .. }) => return Err("unexpected file request".into()),
            Outcome::Done(result) => {
                return result.map_err(|fault| {
                    let tables = program.tables.borrow();
                    format!(
                        "fault in {} at {}..{}: {}",
                        tables.namespaces[fault.namespace],
                        fault.span.start,
                        fault.span.end,
                        fault.reason.describe(&tables)
                    )
                })
            }
        }
    }
}

fn main() -> Result<(), String> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: linked-provider-example c|rust ABSOLUTE_API_FIPI".into());
    }
    let language = match args[0].to_str() {
        Some("c") => Language::C,
        Some("rust") => Language::Rust,
        _ => return Err("expected c or rust".into()),
    };
    let saved = snapshot(Path::new(&args[1]))?;
    let descriptor = language.descriptor();
    let mut provider = paired(&descriptor, MODULE, &saved)?;
    let result = execute(&mut provider, SCRIPT, true)?;
    // This fixed script returns ordinary scalars; no live metadata is inspected.
    let Value::Tuple(values) = result else {
        return Err("expected tuple result".into());
    };
    let expected = [Value::Int(42), Value::text("hello")];
    if values.as_slice() != expected {
        return Err("unexpected provider result".into());
    }
    println!("42\nhello");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn calls(language: Language) -> usize {
        match language {
            // One serial test owns every C invocation in this executable.
            Language::C => unsafe { linked_c_calls() },
            Language::Rust => rust_provider::calls(),
        }
    }

    #[test]
    fn linked_pairing_controls_and_actual_calls() {
        let saved = include_bytes!("../api.fipi");
        for language in [Language::C, Language::Rust] {
            let descriptor = language.descriptor();
            let before = calls(language);
            assert!(paired(&descriptor, "wrong.module", saved).is_err());
            let changed = [saved.as_slice(), b"## stale snapshot\n"].concat();
            assert!(paired(&descriptor, MODULE, &changed).is_err());
            assert!(paired(&descriptor, MODULE, b"\xff").is_err());
            let mut invalid = language.descriptor();
            invalid.version = 1;
            assert!(paired(&invalid, MODULE, saved).is_err());
            invalid = language.descriptor();
            invalid.size -= 1;
            assert!(paired(&invalid, MODULE, saved).is_err());
            assert_eq!(calls(language), before);

            let mut provider = paired(&descriptor, MODULE, saved).unwrap();
            let source = "use proof.ops\nops.integer 42";
            let denied = execute(&mut provider, source, false).unwrap_err();
            assert!(denied.contains("fault in main at 14..28"), "{denied}");
            assert!(denied.contains("denied by driver"), "{denied}");
            assert_eq!(calls(language), before);
            assert_eq!(
                execute(&mut provider, source, true).unwrap(),
                Value::Int(42)
            );
            assert_eq!(
                execute(&mut provider, "use proof.ops\nops.text \"한\\0글\"", true).unwrap(),
                Value::text("한\0글")
            );
            assert_eq!(calls(language), before + 2);
            let failed = execute(&mut provider, "use proof.ops\nops.fail ()", true).unwrap_err();
            assert!(failed.contains("fault in main at 14..25"), "{failed}");
            assert!(failed.contains("provider refused"), "{failed}");
            assert_eq!(calls(language), before + 3);
        }
        let before_files = (calls(Language::C), calls(Language::Rust));
        let dir = env::temp_dir().join(format!(
            "fipe-linked-provider-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("api.fipi");
        assert!(snapshot(&path).is_err());
        fs::write(&path, b"\xff").unwrap();
        assert!(snapshot(&path).is_err());
        fs::write(&path, saved).unwrap();
        assert_eq!(snapshot(&path).unwrap(), saved);
        fs::remove_dir_all(dir).unwrap();
        assert_eq!((calls(Language::C), calls(Language::Rust)), before_files);
    }
}
