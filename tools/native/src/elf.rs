//! The ELF64 relocatable object for x86-64.
//!
//! It has `.text`, `.rodata` and `.data.rel.ro`, their symbols, and the
//! relocations of `.text` and `.data.rel.ro`. A symbol a relocation
//! names and no symbol defines is written undefined and global, for the
//! linker to find.
use std::fmt;

/// A section a symbol can be defined in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// Machine code.
    Text,
    /// Constant bytes.
    Rodata,
    /// Constant after relocation, such as a table of pointers.
    DataRelRo,
}

/// A symbol defined in the object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// Its name.
    pub name: String,
    /// The section it is in: a function in `.text`, an object elsewhere.
    pub section: Section,
    /// Its offset in that section.
    pub offset: u64,
    /// Its size in bytes.
    pub size: u64,
    /// Whether it is seen outside the object.
    pub global: bool,
}

/// How a relocation computes its field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocationKind {
    /// `R_X86_64_PLT32`: a 32-bit displacement to a function.
    Plt32,
    /// `R_X86_64_64`: a 64-bit address.
    Absolute,
}

/// A field of a section the linker fills in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relocation {
    /// The field's offset in its section.
    pub offset: u64,
    /// The symbol the field refers to.
    pub symbol: String,
    /// How the field is computed.
    pub kind: RelocationKind,
    /// The constant added to the symbol's address.
    pub addend: i64,
}

/// The contents of an object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Object {
    /// `.text`
    pub text: Vec<u8>,
    /// The relocations of `.text`.
    pub text_relocations: Vec<Relocation>,
    /// `.rodata`
    pub rodata: Vec<u8>,
    /// `.data.rel.ro`
    pub data: Vec<u8>,
    /// The relocations of `.data.rel.ro`.
    pub data_relocations: Vec<Relocation>,
    /// The defined symbols.
    pub symbols: Vec<Symbol>,
}

/// Why an object cannot be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Two symbols of one name.
    Duplicate(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(name) => write!(f, "symbol {name} defined twice"),
        }
    }
}

impl std::error::Error for Error {}

// Section header indices, in the order the headers are written.
const TEXT: u16 = 1;
const RODATA: u16 = 2;
const DATA: u16 = 3;
const SYMTAB: u16 = 6;
const STRTAB: u16 = 7;
const SHSTRTAB: u16 = 9;
const SECTION_NAMES: [&str; 9] = [
    ".text",
    ".rodata",
    ".data.rel.ro",
    ".rela.text",
    ".rela.data.rel.ro",
    ".symtab",
    ".strtab",
    ".note.GNU-stack",
    ".shstrtab",
];

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_RELA: u32 = 4;
const SHF_WRITE: u64 = 1;
const SHF_ALLOC: u64 = 2;
const SHF_EXECINSTR: u64 = 4;
const SHF_INFO_LINK: u64 = 0x40;
const STB_GLOBAL: u8 = 1;
const STT_OBJECT: u8 = 1;
const STT_FUNC: u8 = 2;
const R_X86_64_64: u64 = 1;
const R_X86_64_PLT32: u64 = 4;

/// Writes `object` as an ELF64 relocatable file.
///
/// # Errors
///
/// When two symbols share a name.
///
/// # Panics
///
/// When the names outgrow a string table's 32-bit offsets.
pub fn write(object: &Object) -> Result<Vec<u8>, Error> {
    let symbols = Symbols::of(object)?;
    let rela_text = symbols.rela(&object.text_relocations);
    let rela_data = symbols.rela(&object.data_relocations);
    let symtab = Header {
        link: STRTAB.into(),
        info: symbols.first_global,
        ..Header::table(SHT_SYMTAB)
    };
    Ok(layout(&[
        (Header::bits(SHF_ALLOC | SHF_EXECINSTR, 16), &object.text),
        (Header::bits(SHF_ALLOC, 8), &object.rodata),
        (Header::bits(SHF_WRITE | SHF_ALLOC, 8), &object.data),
        (Header::rela(TEXT), &rela_text),
        (Header::rela(DATA), &rela_data),
        (symtab, &symbols.table),
        (Header::strings(), &symbols.strings.0),
        // Empty: the stack is not executable.
        (Header::bits(0, 1), &[]),
        (Header::strings(), &Strings::of(SECTION_NAMES).0),
    ]))
}

/// A string table: NUL-terminated names, offset 0 the empty name.
struct Strings(Vec<u8>);

impl Strings {
    fn of<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut strings = Self(vec![0]);
        for name in names {
            strings.add(name);
        }
        strings
    }

    fn add(&mut self, name: &str) -> u32 {
        let at = u32::try_from(self.0.len()).expect("names fit u32");
        self.0.extend(name.as_bytes());
        self.0.push(0);
        at
    }
}

/// `.symtab` and `.strtab`: the null symbol, the locals, the defined
/// globals, then the undefined ones the relocations name.
struct Symbols {
    table: Vec<u8>,
    strings: Strings,
    /// The names in table order, the null symbol left out.
    names: Vec<String>,
    first_global: u32,
}

impl Symbols {
    fn of(object: &Object) -> Result<Self, Error> {
        let mut symbols = Self {
            table: vec![0; 24],
            strings: Strings(vec![0]),
            names: Vec::new(),
            first_global: 1,
        };
        let mut ordered: Vec<&Symbol> = object.symbols.iter().collect();
        ordered.sort_by_key(|symbol| symbol.global);
        for symbol in ordered {
            if symbols.names.contains(&symbol.name) {
                return Err(Error::Duplicate(symbol.name.clone()));
            }
            let (section, kind) = match symbol.section {
                Section::Text => (TEXT, STT_FUNC),
                Section::Rodata => (RODATA, STT_OBJECT),
                Section::DataRelRo => (DATA, STT_OBJECT),
            };
            let info = u8::from(symbol.global) << 4 | kind;
            symbols.add(
                &symbol.name,
                info,
                section,
                symbol.offset,
                symbol.size,
            );
            if !symbol.global {
                symbols.first_global += 1;
            }
        }
        let relocations = object.text_relocations.iter();
        for relocation in relocations.chain(&object.data_relocations) {
            if !symbols.names.contains(&relocation.symbol) {
                // Section 0 is undefined; the type is none.
                symbols.add(&relocation.symbol, STB_GLOBAL << 4, 0, 0, 0);
            }
        }
        Ok(symbols)
    }

    fn add(&mut self, name: &str, info: u8, section: u16, at: u64, size: u64) {
        let name_at = self.strings.add(name);
        self.table.extend(name_at.to_le_bytes());
        self.table.extend([info, 0]);
        self.table.extend(section.to_le_bytes());
        self.table.extend(at.to_le_bytes());
        self.table.extend(size.to_le_bytes());
        self.names.push(name.to_owned());
    }

    /// The `.rela` entries of `relocations`.
    fn rela(&self, relocations: &[Relocation]) -> Vec<u8> {
        let mut out = Vec::new();
        for relocation in relocations {
            let index = self.names.iter().position(|n| *n == relocation.symbol);
            let index = index.expect("every relocation's symbol named") + 1;
            let kind = match relocation.kind {
                RelocationKind::Absolute => R_X86_64_64,
                RelocationKind::Plt32 => R_X86_64_PLT32,
            };
            out.extend(relocation.offset.to_le_bytes());
            out.extend(((index as u64) << 32 | kind).to_le_bytes());
            out.extend(relocation.addend.to_le_bytes());
        }
        out
    }
}

/// A section header's fields besides its name, offset and size.
struct Header {
    kind: u32,
    flags: u64,
    link: u32,
    info: u32,
    align: u64,
    entry: u64,
}

impl Header {
    const fn bits(flags: u64, align: u64) -> Self {
        Self {
            kind: SHT_PROGBITS,
            flags,
            link: 0,
            info: 0,
            align,
            entry: 0,
        }
    }

    const fn strings() -> Self {
        Self {
            kind: SHT_STRTAB,
            ..Self::bits(0, 1)
        }
    }

    /// A table of 24-byte entries.
    const fn table(kind: u32) -> Self {
        Self {
            kind,
            align: 8,
            entry: 24,
            ..Self::bits(0, 8)
        }
    }

    const fn rela(target: u16) -> Self {
        Self {
            flags: SHF_INFO_LINK,
            link: SYMTAB as u32,
            info: target as u32,
            ..Self::table(SHT_RELA)
        }
    }
}

/// The file: the ELF header, each section's contents aligned, then the
/// section headers, the null one first, named after `SECTION_NAMES`.
fn layout(sections: &[(Header, &[u8]); SECTION_NAMES.len()]) -> Vec<u8> {
    let mut names = Strings(vec![0]);
    let mut out = vec![0; 64];
    let mut headers = vec![0; 64];
    for ((header, contents), name) in sections.iter().zip(SECTION_NAMES) {
        pad(&mut out, header.align);
        let offset = out.len() as u64;
        out.extend(*contents);
        headers.extend(names.add(name).to_le_bytes());
        headers.extend(header.kind.to_le_bytes());
        headers.extend(header.flags.to_le_bytes());
        headers.extend(0u64.to_le_bytes()); // address
        headers.extend(offset.to_le_bytes());
        headers.extend((contents.len() as u64).to_le_bytes());
        headers.extend(header.link.to_le_bytes());
        headers.extend(header.info.to_le_bytes());
        headers.extend(header.align.to_le_bytes());
        headers.extend(header.entry.to_le_bytes());
    }
    pad(&mut out, 8);
    let headers_at = out.len() as u64;
    out.extend(headers);

    let mut header = Vec::with_capacity(64);
    // Magic, 64-bit, little-endian, version 1, System V ABI.
    header.extend(b"\x7fELF\x02\x01\x01");
    header.resize(16, 0);
    header.extend(1u16.to_le_bytes()); // ET_REL
    header.extend(62u16.to_le_bytes()); // EM_X86_64
    header.extend(1u32.to_le_bytes()); // EV_CURRENT
    header.extend(0u64.to_le_bytes()); // entry
    header.extend(0u64.to_le_bytes()); // program headers
    header.extend(headers_at.to_le_bytes());
    header.extend(0u32.to_le_bytes()); // flags
    let count = SHSTRTAB + 1; // the names come last
                              // Header sizes, program header size and count, section header size
                              // and count, then the index of the section names.
    for half in [64, 0, 0, 64, count, SHSTRTAB] {
        header.extend(u16::to_le_bytes(half));
    }
    out[..64].copy_from_slice(&header);
    out
}

fn pad(out: &mut Vec<u8>, align: u64) {
    while !(out.len() as u64).is_multiple_of(align) {
        out.push(0);
    }
}
