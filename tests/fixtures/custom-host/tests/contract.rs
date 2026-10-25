use fipe::{
    token::Span,
    value::{Reason, Value},
    vm::Fault,
};
use fipe_custom_host_proof::{Counts, Event, Host};
use std::rc::Rc;

fn host_fault(
    result: Result<Value, Fault>,
    expected: &str,
    source: &str,
    call: &str,
) {
    let Err(Fault {
        reason: Reason::Host(message),
        span,
        ..
    }) = result
    else {
        panic!("expected Host failure: {result:?}");
    };
    assert_eq!(message, expected);
    assert_eq!(
        source.matches(call).count(),
        1,
        "unique failing source call"
    );
    let start = source.find(call).unwrap();
    assert_eq!(
        span,
        Span {
            start,
            end: start + call.len()
        }
    );
}

#[test]
fn namespace_memory_source_and_driver_output() {
    let mut host = Host::default();
    host.run("t = text.open.acquire ()").unwrap();
    assert_eq!(host.run("text.open.read t").unwrap(), Value::text("row"));
    assert_eq!(host.run("client.open.read t").unwrap(), Value::Int(3));
    host.source("config", "pub seven = 7");
    assert_eq!(host.run("use config\nconfig.seven").unwrap(), Value::Int(7));
    let calls: Vec<_> = host
        .events()
        .iter()
        .filter_map(|event| {
            if let Event::Call(namespace, label) = event {
                Some((namespace.as_str(), label.as_str()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("text.open", "acquire"),
            ("text.open", "read"),
            ("client.open", "read"),
        ]
    );
    assert_eq!(
        host.events()
            .iter()
            .filter(|event| {
                matches!(event, Event::File(stem) if stem == "config")
            })
            .count(),
        1
    );
    assert_eq!(host.run("1\n2").unwrap(), Value::Int(2));
    assert_eq!(host.output(), [Value::Int(1), Value::Int(2)]);
    assert_eq!(
        host.counts(),
        Counts {
            created: 1,
            live: 1,
            reads: 2,
            released: 0
        }
    );
}

#[test]
fn denial_is_before_payload_access() {
    let mut host = Host::default();
    let setup = "t = text.open.acquire ()";
    host.run(setup).unwrap();
    let before = host.events().len();
    host.grant_read(false);
    let cell = "text.open.read t\nclient.open.read t";
    host_fault(
        host.run(cell),
        "kernel read denied",
        &format!("{setup}\n\n{cell}"),
        "text.open.read t",
    );
    assert_eq!(
        host.counts(),
        Counts {
            created: 1,
            live: 1,
            reads: 0,
            released: 0
        }
    );
    assert!(host.output().is_empty());
    assert_eq!(
        &host.events()[before..],
        [
            Event::Call("text.open".into(), "read".into()),
            Event::Withdraw,
        ]
    );
    host.grant_read(true);
    assert_eq!(host.run("text.open.read t").unwrap(), Value::text("row"));
    assert_eq!(host.counts().reads, 1);
}

#[test]
fn fault_cleans_only_new_entries() {
    let mut host = Host::default();
    let setup = "t = text.open.acquire ()";
    host.run(setup).unwrap();
    let before = host.events().len();
    let cell = "fresh = text.open.acquire ()\nclient.open.fail ()";
    host_fault(
        host.run(cell),
        "kernel refused",
        &format!("{setup}\n\n{cell}"),
        "client.open.fail ()",
    );
    assert_eq!(
        host.counts(),
        Counts {
            created: 2,
            live: 1,
            reads: 0,
            released: 1
        }
    );
    assert_eq!(
        &host.events()[before..],
        [
            Event::Call("text.open".into(), "acquire".into()),
            Event::Create(2),
            Event::Call("client.open".into(), "fail".into()),
            Event::Revoke(2),
            Event::Withdraw,
        ]
    );
    assert!(host.withdrawn("fresh"));
    assert_eq!(host.run("text.open.read t").unwrap(), Value::text("row"));
    assert_eq!(host.counts().released, 1);
}

#[test]
fn aliases_and_explicit_disposal() {
    let mut host = Host::default();
    host.run("t = text.open.acquire ()").unwrap();
    let alias = host.run("t").unwrap();
    let copy = alias.clone();
    assert_eq!(
        host.run("reader = x => text.open.read x\nreader t")
            .unwrap(),
        Value::text("row"),
    );
    let tuple = host.run("(t, t)").unwrap();
    let Value::Tuple(parts) = &tuple else {
        panic!("typed aliases")
    };
    let Value::Resource(original) = &alias else {
        panic!("real acquisition")
    };
    for part in parts.iter() {
        assert!(
            matches!(part, Value::Resource(item) if Rc::ptr_eq(item, original))
        );
    }
    drop(tuple);
    drop(copy);
    host.discard_language();
    assert_eq!(
        host.counts(),
        Counts {
            created: 1,
            live: 1,
            reads: 1,
            released: 0
        }
    );
    // A real Rust alias survives; the same no_std Kernel validates it directly.
    assert_eq!(host.read_alias(&alias).unwrap(), "row");
    host.dispose();
    assert_eq!(
        host.counts(),
        Counts {
            created: 1,
            live: 0,
            reads: 2,
            released: 1
        }
    );
    assert_eq!(host.read_alias(&alias).unwrap_err(), "closed entry");
    assert_eq!(host.counts().reads, 2);
    drop(alias);
    host.dispose();
    assert_eq!(host.counts().released, 1);
}
