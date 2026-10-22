//! The copies of `src/ops/` against their sources: each source's hash
//! equals the one `COPIED.md` records, so a change of the VM fails here
//! until its copy is reviewed and the ledger re-blessed with
//! `FIPE_BLESS=1`.
use std::path::Path;

/// FNV-1a, 64 bits, as `codec::fnv`; the test cannot link the runtime,
/// whose `main` is the executable's.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Each copied source, by its path in the crate.
const SOURCES: &[(&str, &str)] = &[
    ("src/vm.rs", include_str!("../../../../src/vm.rs")),
    (
        "src/compile/dispatch.rs",
        include_str!("../../../../src/compile/dispatch.rs"),
    ),
    (
        "src/vm/assign.rs",
        include_str!("../../../../src/vm/assign.rs"),
    ),
    (
        "src/vm/budget.rs",
        include_str!("../../../../src/vm/budget.rs"),
    ),
    (
        "src/vm/calls.rs",
        include_str!("../../../../src/vm/calls.rs"),
    ),
    (
        "src/vm/casts.rs",
        include_str!("../../../../src/vm/casts.rs"),
    ),
    (
        "src/vm/defaults.rs",
        include_str!("../../../../src/vm/defaults.rs"),
    ),
    (
        "src/vm/dispatch.rs",
        include_str!("../../../../src/vm/dispatch.rs"),
    ),
    (
        "src/vm/drive.rs",
        include_str!("../../../../src/vm/drive.rs"),
    ),
    (
        "src/vm/entries.rs",
        include_str!("../../../../src/vm/entries.rs"),
    ),
    ("src/vm/fits.rs", include_str!("../../../../src/vm/fits.rs")),
    (
        "src/vm/frames.rs",
        include_str!("../../../../src/vm/frames.rs"),
    ),
    (
        "src/vm/globals.rs",
        include_str!("../../../../src/vm/globals.rs"),
    ),
    (
        "src/vm/immediate.rs",
        include_str!("../../../../src/vm/immediate.rs"),
    ),
    (
        "src/vm/natives.rs",
        include_str!("../../../../src/vm/natives.rs"),
    ),
    (
        "src/vm/order.rs",
        include_str!("../../../../src/vm/order.rs"),
    ),
    (
        "src/vm/pending.rs",
        include_str!("../../../../src/vm/pending.rs"),
    ),
    (
        "src/vm/predicates.rs",
        include_str!("../../../../src/vm/predicates.rs"),
    ),
    ("src/vm/step.rs", include_str!("../../../../src/vm/step.rs")),
    ("src/vm/walk.rs", include_str!("../../../../src/vm/walk.rs")),
];

/// The ledger's rows: copy, source and recorded hash.
fn rows(ledger: &str) -> Vec<(String, String, String)> {
    ledger
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let code = |cell: &str| {
                let inner = cell.strip_prefix('`')?.strip_suffix('`')?;
                Some(inner.to_owned())
            };
            match cells.as_slice() {
                ["", copy, source, hash, _, ""] => {
                    Some((code(copy)?, code(source)?, code(hash)?))
                }
                _ => None,
            }
        })
        .collect()
}

fn hash(source: &str) -> String {
    let text = SOURCES
        .iter()
        .find(|(path, _)| *path == source)
        .unwrap_or_else(|| panic!("{source} is not in SOURCES"))
        .1;
    format!("{:016x}", fnv(text.as_bytes()))
}

#[test]
fn every_copied_source_has_the_hash_its_copy_was_reviewed_at() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("COPIED.md");
    let ledger = std::fs::read_to_string(&path).unwrap();
    let rows = rows(&ledger);
    assert!(!rows.is_empty(), "COPIED.md has no rows");
    if std::env::var_os("FIPE_BLESS").is_some() {
        let mut blessed = ledger;
        for (_, source, recorded) in &rows {
            blessed = blessed.replace(
                &format!("| `{source}` | `{recorded}` |"),
                &format!("| `{source}` | `{}` |", hash(source)),
            );
        }
        std::fs::write(&path, blessed).unwrap();
        return;
    }
    let changed: Vec<String> = rows
        .iter()
        .filter(|(_, source, recorded)| hash(source) != *recorded)
        .map(|(copy, source, _)| {
            format!("{source} changed: review {copy}, then re-bless")
        })
        .collect();
    assert!(changed.is_empty(), "{}", changed.join("\n"));
    for (source, _) in SOURCES {
        assert!(
            rows.iter().any(|(_, listed, _)| listed == source),
            "{source} has no row in COPIED.md"
        );
    }
    for (copy, _, _) in &rows {
        let copy = Path::new(env!("CARGO_MANIFEST_DIR")).join(copy);
        assert!(copy.exists(), "{} is not a file", copy.display());
    }
}
