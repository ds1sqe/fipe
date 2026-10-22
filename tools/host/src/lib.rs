//! The std host: its declarations, as `tools/host/README.md` documents
//! them.
#![forbid(unsafe_code)]

use fipe::decl::Declarations;

/// The std host's declaration files by namespace, `commands` first.
const SOURCES: &[(&str, &str)] = &[
    ("commands", include_str!("../decl/commands.fipe")),
    ("fs", include_str!("../decl/fs.fipe")),
    ("sys", include_str!("../decl/sys.fipe")),
    ("cli", include_str!("../decl/cli.fipe")),
    ("text", include_str!("../decl/text.fipe")),
];

/// The std host's declaration files by namespace, `commands` first.
#[must_use]
pub const fn sources() -> &'static [(&'static str, &'static str)] {
    SOURCES
}

/// The std host's declarations, loaded from [`sources`].
///
/// # Panics
/// When a declaration file does not load, a defect the tests catch.
#[must_use]
pub fn declarations() -> Declarations {
    let mut declarations = Declarations::new();
    for (name, text) in SOURCES {
        let loaded = if *name == "commands" {
            declarations.commands(text)
        } else {
            declarations.module(name, text)
        };
        if let Err(errors) = loaded {
            panic!("the std host's {name} declarations: {errors:?}");
        }
    }
    declarations
}
