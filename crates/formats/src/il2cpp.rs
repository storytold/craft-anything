//! Unity IL2CPP's `global-metadata.dat`: the names a game's C# had before it was compiled to native
//! code. Assembly images, types and their methods, and the string literals.
//!
//! Metadata versions read: 24 (only the later 24.x files, with a 264-byte header), 27, 29 and 31
//! (256-byte header), and 39 (380-byte header). Each is a row of [`LAYOUTS`]. A file has to prove a
//! row fits before it's used: the header's size, every record size dividing its section, the types'
//! method counts adding up to the methods there are, and names that read as names. A version with
//! no row, or a file no row fits, gives only its header and the reason: other versions moved fields
//! around, and a layout is never guessed. Games often change or encrypt this file, which reads as
//! [`Reading::Altered`].
//!
//! Version 39's row was measured from one shipped game's file (38,092 types, 264,589 methods, 216
//! images), not taken from another reader: each field is the only place in its record that agrees
//! with the rest of the file (a type's method range with the methods' declaring types, the field
//! counts with the fields table, an image's type range with the next image's). Its header gives
//! every section's record count as well, and a file must agree with those too. In that file a type
//! index takes two bytes; a file with more types than two bytes can count isn't read as this row.

use crate::bytes::Reader;
use serde::Serialize;

/// The file's first four bytes, little-endian.
pub const SANITY: u32 = 0xFAB1_1BAF;
/// Longest metadata name read; a longer one means the file isn't what it seems.
const MAX_NAME: usize = 4096;
/// The sections named in the header up to version 31, in header order, as (offset, size) word
/// pairs after the two-word start. Indexes past the last used one differ between versions and
/// aren't read.
const SECTIONS: [&str; 22] = [
    "string_literals",
    "string_literal_data",
    "strings",
    "events",
    "properties",
    "methods",
    "parameter_default_values",
    "field_default_values",
    "default_value_data",
    "field_marshaled_sizes",
    "parameters",
    "fields",
    "generic_parameters",
    "generic_parameter_constraints",
    "generic_containers",
    "nested_types",
    "interfaces",
    "vtable_methods",
    "interface_offsets",
    "type_definitions",
    "images",
    "assemblies",
];
/// Version 39's 31 sections, as (offset, size, count) word triples. Only the sections whose records
/// were matched against the rest of the file are named; the others are numbered.
const SECTIONS_39: [&str; 31] = [
    "string_literals",
    "string_literal_data",
    "strings",
    "section_3",
    "section_4",
    "methods",
    "section_6",
    "section_7",
    "section_8",
    "section_9",
    "section_10",
    "fields",
    "section_12",
    "section_13",
    "section_14",
    "section_15",
    "section_16",
    "section_17",
    "section_18",
    "type_definitions",
    "images",
    "assemblies",
    "section_22",
    "section_23",
    "section_24",
    "section_25",
    "section_26",
    "section_27",
    "section_28",
    "section_29",
    "section_30",
];
const LITERALS: usize = 0;
const LITERAL_DATA: usize = 1;
const STRINGS: usize = 2;
const METHODS: usize = 5;
const TYPES: usize = 19;
const IMAGES: usize = 20;

/// How the header lists its sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// 22 (offset, size) pairs.
    Pairs,
    /// 31 (offset, size, record count) triples.
    Triples,
}

impl Header {
    fn names(self) -> &'static [&'static str] {
        match self {
            Header::Pairs => &SECTIONS,
            Header::Triples => &SECTIONS_39,
        }
    }

    fn stride(self) -> usize {
        match self {
            Header::Pairs => 8,
            Header::Triples => 12,
        }
    }
}

/// How the string literals' records find their bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Literals {
    /// One (length, offset) pair per literal.
    LengthOffset,
    /// One start offset per literal and a last one where the data ends: each literal runs to the
    /// next one's start.
    Starts,
}

impl Literals {
    fn record(self) -> usize {
        match self {
            Literals::LengthOffset => 8,
            Literals::Starts => 4,
        }
    }
}

/// Where one metadata version keeps what we read. One row per layout seen in that version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub version: i32,
    header: Header,
    header_size: u32,
    literals: Literals,
    /// Bytes in a type index where methods and images name a type: 4, or 2.
    type_index: usize,
    type_size: usize,
    type_flags: usize,
    type_method_start: usize,
    type_method_count: usize,
    type_field_count: usize,
    type_token: usize,
    method_size: usize,
    method_type: usize,
    method_token: usize,
    method_flags: usize,
    method_parameters: usize,
    image_size: usize,
    image_type_start: usize,
    image_type_count: usize,
    image_token: Option<usize>,
}

/// Up to version 31: what the type and method layouts don't change.
const fn classic(version: i32, header_size: u32, type_size: usize, method_size: usize) -> Layout {
    // A later 24.x type also has its by-reference type index, which moves the rest on by four.
    let older = type_size - 88;
    // A method's fields after its first three move on by the return-parameter token, when it has one.
    let shift = method_size - 32;
    Layout {
        version,
        header: Header::Pairs,
        header_size,
        literals: Literals::LengthOffset,
        type_index: 4,
        type_size,
        type_flags: 28 + older,
        type_method_start: 36 + older,
        type_method_count: 64 + older,
        type_field_count: 68 + older,
        type_token: 84 + older,
        method_size,
        method_type: 4,
        method_token: 20 + shift,
        method_flags: 24 + shift,
        method_parameters: 30 + shift,
        image_size: 40,
        image_type_start: 8,
        image_type_count: 12,
        image_token: Some(28),
    }
}

/// Every layout read. Version 29 has two: later files added a return-parameter token to methods
/// without changing the version number, and the checks tell them apart.
pub const LAYOUTS: &[Layout] = &[
    classic(24, 264, 92, 32),
    classic(27, 256, 88, 32),
    classic(29, 256, 88, 32),
    classic(29, 256, 88, 36),
    classic(31, 256, 88, 36),
    Layout {
        version: 39,
        header: Header::Triples,
        header_size: 380,
        literals: Literals::Starts,
        type_index: 2,
        type_size: 82,
        type_flags: 22,
        type_method_start: 30,
        type_method_count: 58,
        type_field_count: 62,
        type_token: 78,
        method_size: 32,
        method_type: 4,
        method_token: 20,
        method_flags: 24,
        method_parameters: 30,
        image_size: 36,
        image_type_start: 8,
        image_type_count: 10,
        // No word of the measured images reads as a token, so none is claimed.
        image_token: None,
    },
];

/// The metadata versions a row exists for.
pub fn known_versions() -> Vec<i32> {
    let mut v: Vec<i32> = LAYOUTS.iter().map(|l| l.version).collect();
    v.dedup();
    v
}

/// One section of the header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub name: &'static str,
    pub offset: u32,
    pub size: u32,
    /// How many records the header says it holds, for a header that says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// How far this file could be read.
#[derive(Debug)]
pub enum Reading<'a> {
    /// It doesn't start like this format: changed, encrypted, or another kind of file.
    Altered {
        first_bytes: Vec<u8>,
        why: String,
    },
    /// A real header, with `version`, but its contents can't be read safely. `sections` are given
    /// only for a version known to place them that way.
    HeaderOnly {
        version: Option<i32>,
        sections: Vec<Section>,
        why: String,
    },
    Readable(Metadata<'a>),
}

/// A file whose layout is proven.
#[derive(Debug)]
pub struct Metadata<'a> {
    data: &'a [u8],
    layout: Layout,
    sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Image {
    pub index: usize,
    pub name: String,
    pub type_start: u32,
    pub type_count: u32,
    /// `None` where the version's layout has no place known for it.
    pub token: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Type {
    pub index: usize,
    pub namespace: String,
    pub name: String,
    pub method_start: i32,
    pub method_count: u16,
    pub field_count: u16,
    pub flags: u32,
    pub token: u32,
}

impl Type {
    pub fn full_name(&self) -> String {
        if self.namespace.is_empty() { self.name.clone() } else { format!("{}.{}", self.namespace, self.name) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Method {
    pub index: usize,
    pub name: String,
    /// The index of the type declaring it.
    pub type_index: i32,
    pub parameter_count: u16,
    pub flags: u16,
    pub token: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Literal {
    pub index: usize,
    /// Its full length in bytes.
    pub length: u32,
    /// At most the bytes asked for, as text.
    pub text: String,
    /// False when the length asked for cut it short.
    pub complete: bool,
}

/// Read a file's header and pick its layout. Never fails: every way a file can be wrong is a
/// [`Reading`] saying so.
pub fn read(data: &[u8]) -> Reading<'_> {
    let r = Reader::new(data);
    let first_bytes = data.get(..data.len().min(16)).unwrap_or_default().to_vec();
    let Some(sanity) = r.u32_le(0) else {
        return Reading::Altered { first_bytes, why: format!("the file is only {} bytes long", data.len()) };
    };
    if sanity != SANITY {
        return Reading::Altered { first_bytes, why: format!("it starts with {sanity:#010x} instead of {SANITY:#010x}") };
    }
    let Some(version) = r.u32_le(4).map(|v| v as i32) else {
        return Reading::HeaderOnly { version: None, sections: Vec::new(), why: "the file ends before its version".into() };
    };
    let rows: Vec<&Layout> = LAYOUTS.iter().filter(|l| l.version == version).collect();
    let Some(first) = rows.first() else {
        return Reading::HeaderOnly { version: Some(version), sections: Vec::new(), why: format!("metadata version {version} has no layout known here") };
    };
    // Every row of one version shares its header.
    let Some(sections) = sections(&r, first.header) else {
        return Reading::HeaderOnly { version: Some(version), sections: Vec::new(), why: "the file ends inside its header".into() };
    };
    // A layout whose shape fits but whose names don't says more than one of the wrong shape.
    let (mut misfit, mut unreadable) = (None, None);
    for layout in rows {
        let m = Metadata { data, layout: *layout, sections: sections.clone() };
        match m.fits().and_then(|()| m.names_read().map_err(|e| unreadable.insert(e).clone())) {
            Ok(()) => return Reading::Readable(m),
            Err(e) => misfit = misfit.or(Some(e)),
        }
    }
    let why = unreadable.or(misfit).unwrap_or_default();
    Reading::HeaderOnly { version: Some(version), sections, why }
}

fn sections(r: &Reader<'_>, header: Header) -> Option<Vec<Section>> {
    let names = header.names();
    let mut out = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let at = 8usize.checked_add(i.checked_mul(header.stride())?)?;
        let count = match header {
            Header::Pairs => None,
            Header::Triples => Some(r.u32_le(at.checked_add(8)?)?),
        };
        out.push(Section { name, offset: r.u32_le(at)?, size: r.u32_le(at.checked_add(4)?)?, count });
    }
    Some(out)
}

impl<'a> Metadata<'a> {
    fn section(&self, which: usize) -> &'a [u8] {
        self.sections.get(which).and_then(|s| self.data.get(s.offset as usize..(s.offset as usize).checked_add(s.size as usize)?)).unwrap_or_default()
    }

    fn record(&self, which: usize, size: usize, index: usize) -> Option<Reader<'a>> {
        let at = index.checked_mul(size)?;
        Some(Reader::new(self.section(which).get(at..at.checked_add(size)?)?))
    }

    /// Whether this file has the shape this layout says it should.
    fn fits(&self) -> Result<(), String> {
        let l = &self.layout;
        for s in &self.sections {
            let end = u64::from(s.offset) + u64::from(s.size);
            if end > self.data.len() as u64 {
                return Err(format!("its {} section lies outside the file, so the file is cut short or changed", s.name));
            }
            if s.size > 0 && s.offset < l.header_size {
                return Err(format!("its {} section starts inside the header", s.name));
            }
        }
        let first = self.sections.first().map_or(0, |s| s.offset);
        if first != l.header_size {
            return Err(format!("its data starts at {first}, but version {} has a {}-byte header in the layouts known here", l.version, l.header_size));
        }
        for (which, size) in [(LITERALS, l.literals.record()), (METHODS, l.method_size), (TYPES, l.type_size), (IMAGES, l.image_size)] {
            let s = self.section(which).len();
            let name = l.header.names().get(which).copied().unwrap_or_default();
            if !s.is_multiple_of(size) {
                return Err(format!("its {name} section ({s} bytes) doesn't divide into {size}-byte records"));
            }
            let records = s / size;
            if let Some(count) = self.sections.get(which).and_then(|sec| sec.count)
                && count as usize != records
            {
                return Err(format!("its header counts {count} {name} records, but the section holds {records}"));
            }
        }
        let (types, methods) = (self.type_count(), self.method_count());
        if l.type_index == 2 && types > usize::from(u16::MAX) {
            return Err(format!("it has {types} types, more than the two-byte type indexes of version {}'s layout can name", l.version));
        }
        self.literal_starts_fit()?;
        let mut total = 0u64;
        for i in 0..types {
            let r = self.record(TYPES, l.type_size, i).ok_or("a type record can't be read")?;
            let start = r.u32_le(l.type_method_start).ok_or("a type record can't be read")? as i32;
            let count = r.u16_le(l.type_method_count).ok_or("a type record can't be read")?;
            if count > 0 {
                let range = usize::try_from(start).ok().and_then(|start| Some(start..start.checked_add(usize::from(count))?));
                let Some(range) = range.filter(|r| r.end <= methods) else { return Err(format!("type {i} points at methods outside the table")) };
                for method in range {
                    if self.method_owner(method) != Some(i) {
                        return Err(format!("method {method} is in type {i}'s range but declares a different owner"));
                    }
                }
            }
            total = total.saturating_add(u64::from(count));
        }
        if total != methods as u64 {
            return Err(format!("its types name {total} methods but the table holds {methods}"));
        }
        for i in 0..self.image_count() {
            let Some(image) = self.image(i) else { return Err(format!("image {i} can't be read")) };
            if u64::from(image.type_start) + u64::from(image.type_count) > types as u64 {
                return Err(format!("image {i} points at types outside the table"));
            }
        }
        Ok(())
    }

    /// Whether the first names read as names: a file can keep its shape and encrypt its strings.
    fn names_read(&self) -> Result<(), String> {
        if self.image_count() > 0 && !self.image(0).is_some_and(|im| looks_like_name(&im.name)) {
            return Err("its first assembly's name doesn't read as a name, so its strings are probably encrypted".into());
        }
        if self.type_count() > 0 && !self.type_def(0).is_some_and(|t| looks_like_name(&t.name)) {
            return Err("its first type's name doesn't read as a name, so its strings are probably encrypted".into());
        }
        Ok(())
    }

    pub fn version(&self) -> i32 {
        self.layout.version
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn image_count(&self) -> usize {
        self.section(IMAGES).len() / self.layout.image_size
    }

    pub fn type_count(&self) -> usize {
        self.section(TYPES).len() / self.layout.type_size
    }

    pub fn method_count(&self) -> usize {
        self.section(METHODS).len() / self.layout.method_size
    }

    pub fn literal_count(&self) -> usize {
        let records = self.section(LITERALS).len() / self.layout.literals.record();
        match self.layout.literals {
            Literals::LengthOffset => records,
            // The last start only says where the data ends.
            Literals::Starts => records.saturating_sub(1),
        }
    }

    /// A literal start table has to run in order from the data's start (0) to its end.
    fn literal_starts_fit(&self) -> Result<(), String> {
        if self.layout.literals != Literals::Starts {
            return Ok(());
        }
        let table = self.section(LITERALS);
        let data = self.section(LITERAL_DATA).len();
        if table.is_empty() && data > 0 {
            return Err(format!("it has {data} bytes of string literal data but no literal start table"));
        }
        let mut last = 0usize;
        for i in 0..table.len() / 4 {
            let Some(start) = Reader::new(table).u32_le(i * 4).map(|v| v as usize) else { return Err("a literal record can't be read".into()) };
            if (i == 0 && start != 0) || start < last || start > data {
                return Err(format!("its string literal {i} starts at {start}, out of order, past the literal data or not at its start"));
            }
            last = start;
        }
        if table.len() >= 4 && last != data {
            return Err(format!("its string literals end at {last}, but the literal data is {data} bytes"));
        }
        Ok(())
    }

    /// A type index of the layout's width at `at`, as a `u32` (or `None` past the record).
    fn type_index(&self, r: &Reader<'_>, at: usize) -> Option<u32> {
        match self.layout.type_index {
            2 => r.u16_le(at).map(u32::from),
            _ => r.u32_le(at),
        }
    }

    /// The type a method says declares it.
    fn method_owner(&self, method: usize) -> Option<usize> {
        let r = self.record(METHODS, self.layout.method_size, method)?;
        usize::try_from(self.type_index(&r, self.layout.method_type)?).ok()
    }

    /// A name from the strings section; `""` when its index is outside it or it isn't terminated.
    fn string(&self, index: u32) -> String {
        let strings = self.section(STRINGS);
        let rest = strings.get(index as usize..).unwrap_or_default();
        let rest = rest.get(..rest.len().min(MAX_NAME)).unwrap_or_default();
        let Some(end) = rest.iter().position(|b| *b == 0) else { return String::new() };
        String::from_utf8_lossy(rest.get(..end).unwrap_or_default()).into_owned()
    }

    pub fn image(&self, index: usize) -> Option<Image> {
        let l = &self.layout;
        let r = self.record(IMAGES, l.image_size, index)?;
        let token = match l.image_token {
            Some(at) => Some(r.u32_le(at)?),
            None => None,
        };
        Some(Image {
            index,
            name: self.string(r.u32_le(0)?),
            type_start: self.type_index(&r, l.image_type_start)?,
            type_count: self.type_index(&r, l.image_type_count)?,
            token,
        })
    }

    pub fn type_def(&self, index: usize) -> Option<Type> {
        let l = &self.layout;
        let r = self.record(TYPES, l.type_size, index)?;
        Some(Type {
            index,
            name: self.string(r.u32_le(0)?),
            namespace: self.string(r.u32_le(4)?),
            method_start: r.u32_le(l.type_method_start)? as i32,
            method_count: r.u16_le(l.type_method_count)?,
            field_count: r.u16_le(l.type_field_count)?,
            flags: r.u32_le(l.type_flags)?,
            token: r.u32_le(l.type_token)?,
        })
    }

    pub fn method(&self, index: usize) -> Option<Method> {
        let l = &self.layout;
        let r = self.record(METHODS, l.method_size, index)?;
        Some(Method {
            index,
            name: self.string(r.u32_le(0)?),
            type_index: self.type_index(&r, l.method_type)? as i32,
            parameter_count: r.u16_le(l.method_parameters)?,
            flags: r.u16_le(l.method_flags)?,
            token: r.u32_le(l.method_token)?,
        })
    }

    /// A string literal, at most `max_bytes` of it. `None` when its record or bytes lie outside.
    pub fn literal(&self, index: usize, max_bytes: usize) -> Option<Literal> {
        let (length, start) = match self.layout.literals {
            Literals::LengthOffset => {
                let r = self.record(LITERALS, 8, index)?;
                (r.u32_le(0)?, r.u32_le(4)? as usize)
            }
            Literals::Starts => {
                let table = Reader::new(self.section(LITERALS));
                let start = table.u32_le(index.checked_mul(4)?)?;
                let end = table.u32_le(index.checked_add(1)?.checked_mul(4)?)?;
                (end.checked_sub(start)?, start as usize)
            }
        };
        let bytes = self.section(LITERAL_DATA).get(start..start.checked_add(length as usize)?)?;
        let (text, complete) = literal_text(bytes, max_bytes);
        Some(Literal { index, length, text, complete })
    }

    /// The first image holding this type.
    pub fn image_of(&self, type_index: usize) -> Option<Image> {
        (0..self.image_count()).filter_map(|i| self.image(i)).find(|im| {
            let start = im.type_start as usize;
            type_index >= start && start.checked_add(im.type_count as usize).is_some_and(|end| type_index < end)
        })
    }

    /// How many fields the types declare.
    pub fn field_count(&self) -> u64 {
        (0..self.type_count()).filter_map(|i| self.type_def(i)).map(|t| u64::from(t.field_count)).sum()
    }
}

fn literal_text(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    let shown = bytes.get(..bytes.len().min(max_bytes)).unwrap_or_default();
    let mut text = String::new();
    let mut consumed = 0;
    for chunk in shown.utf8_chunks() {
        let valid = chunk.valid();
        let mut end = valid.len().min(max_bytes.saturating_sub(text.len()));
        while !valid.is_char_boundary(end) {
            end -= 1;
        }
        text.push_str(valid.get(..end).unwrap_or_default());
        consumed += end;
        if end < valid.len() {
            break;
        }
        let invalid = chunk.invalid();
        if invalid.is_empty() {
            continue;
        }
        if consumed + invalid.len() == shown.len() && shown.len() < bytes.len() && std::str::from_utf8(invalid).is_err_and(|error| error.error_len().is_none())
        {
            break;
        }
        if max_bytes.saturating_sub(text.len()) < '\u{FFFD}'.len_utf8() {
            break;
        }
        text.push('\u{FFFD}');
        consumed += invalid.len();
    }
    (text, consumed == bytes.len())
}

/// Text a compiler would write as a name: not empty, no control characters, and valid UTF-8 (a bad
/// byte shows up as U+FFFD after a lossy read).
fn looks_like_name(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control() || c == '\u{FFFD}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::games::{Il2cppSample, il2cpp_metadata};

    fn sample(version: i32, method_size: usize) -> Vec<u8> {
        il2cpp_metadata(&Il2cppSample {
            version,
            header_size: match version {
                24 => 264,
                39 => 380,
                _ => 256,
            },
            type_size: match version {
                24 => 92,
                39 => 82,
                _ => 88,
            },
            method_size,
        })
    }

    fn readable(data: &[u8]) -> Metadata<'_> {
        match read(data) {
            Reading::Readable(m) => m,
            other => panic!("not readable: {other:?}"),
        }
    }

    fn all_layouts() -> Vec<(i32, usize)> {
        LAYOUTS.iter().map(|l| (l.version, l.method_size)).collect()
    }

    #[test]
    fn every_known_layout_reads_back() {
        for (version, method_size) in all_layouts() {
            let data = sample(version, method_size);
            let m = readable(&data);
            assert_eq!(m.version(), version);
            let image_token = m.layout.image_token.map(|_| 0x2000_0001);
            assert_eq!((m.image_count(), m.type_count(), m.method_count(), m.literal_count()), (2, 3, 4, 3), "version {version}");
            assert_eq!(m.field_count(), 7);
            let image = m.image(1).unwrap();
            assert_eq!((image.name.as_str(), image.type_start, image.type_count, image.token), ("Assembly-CSharp.dll", 1, 2, image_token));
            let t = m.type_def(2).unwrap();
            assert_eq!(t.full_name(), "Game.Player");
            assert_eq!((t.method_start, t.method_count, t.field_count, t.flags, t.token), (1, 3, 5, 0x0010_0001, 0x0200_0003));
            assert_eq!(m.type_def(0).unwrap().full_name(), "<Module>");
            let method = m.method(3).unwrap();
            assert_eq!((method.name.as_str(), method.type_index, method.parameter_count, method.flags, method.token), ("TakeDamage", 2, 2, 0x86, 0x0600_0004));
            assert_eq!(m.image_of(2).unwrap().name, "Assembly-CSharp.dll");
            assert_eq!(m.image_of(0).unwrap().name, "mscorlib.dll");
            assert!(m.image_of(99).is_none());
        }
    }

    #[test]
    fn literals_are_bounded_and_say_when_cut_short() {
        // Version 39 finds a literal's end by the next one's start, the others by its length.
        for (version, method_size) in [(29, 32), (39, 32)] {
            let data = sample(version, method_size);
            let m = readable(&data);
            let l = m.literal(0, 1000).unwrap();
            assert_eq!((l.text.as_str(), l.length, l.complete), ("Hello, player", 13, true));
            let cut = m.literal(0, 5).unwrap();
            assert_eq!((cut.text.as_str(), cut.length, cut.complete), ("Hello", 13, false));
            assert_eq!(m.literal(1, 1000).unwrap().text, "Game over");
            assert_eq!(m.literal(2, 1000).unwrap().text, "");
            assert!(m.literal(3, 1000).is_none());
        }
    }

    #[test]
    fn version_39_literal_starts_must_run_in_order_to_the_data_end() {
        let original = sample(39, 32);
        let (table, _) = read_section(&original, LITERALS);
        // The third literal starting inside the first one, before the second.
        let mut backwards = original.clone();
        backwards[table + 8..table + 12].copy_from_slice(&5u32.to_le_bytes());
        assert!(matches!(read(&backwards), Reading::HeaderOnly { why, .. } if why.contains("out of order")));
        // The first literal starting after the data's start, which would cut its text short.
        let mut late = original.clone();
        late[table..table + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(read(&late), Reading::HeaderOnly { why, .. } if why.contains("not at its start")));
        // The last start falling short of the data's end.
        let mut short = original;
        short[table + 12..table + 16].copy_from_slice(&5u32.to_le_bytes());
        assert!(matches!(read(&short), Reading::HeaderOnly { .. }));
    }

    #[test]
    fn version_39_header_counts_must_match_the_records() {
        // The types section's count word, one more than the records it holds.
        let mut data = sample(39, 32);
        let at = 8 + TYPES * 12 + 8;
        data[at..at + 4].copy_from_slice(&4u32.to_le_bytes());
        assert!(matches!(read(&data), Reading::HeaderOnly { why, .. } if why.contains("counts")));
    }

    #[test]
    fn version_29s_two_method_shapes_are_told_apart() {
        // Both layouts are rows for 29; the checks pick the one that fits.
        assert_eq!(readable(&sample(29, 32)).method(3).unwrap().token, 0x0600_0004);
        assert_eq!(readable(&sample(29, 36)).method(3).unwrap().token, 0x0600_0004);
    }

    #[test]
    fn other_versions_give_the_header_and_a_reason() {
        for version in [0, 16, 19, 23, 25, 28, 30, 35, 38, 40, -1, i32::MAX] {
            let mut data = sample(31, 36);
            data[4..8].copy_from_slice(&version.to_le_bytes());
            match read(&data) {
                Reading::HeaderOnly { version: Some(v), sections, why } => {
                    assert_eq!(v, version);
                    assert!(sections.is_empty(), "sections aren't named for a version without a layout");
                    assert!(why.contains("no layout known"));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_relabelled_version_does_not_read_as_that_version() {
        // A version 31 file calling itself 24: its header is the wrong size.
        let mut data = sample(31, 36);
        data[4..8].copy_from_slice(&24i32.to_le_bytes());
        assert!(matches!(read(&data), Reading::HeaderOnly { why, .. } if why.contains("header")));
        // A version 31 file calling itself 27: its methods don't divide or don't add up.
        let mut data = sample(31, 36);
        data[4..8].copy_from_slice(&27i32.to_le_bytes());
        assert!(matches!(read(&data), Reading::HeaderOnly { version: Some(27), .. }));
    }

    #[test]
    fn altered_magic_is_altered() {
        let mut data = sample(29, 32);
        data[0] ^= 0xff;
        match read(&data) {
            Reading::Altered { first_bytes, why } => {
                assert_eq!(first_bytes.len(), 16);
                assert!(why.contains("instead of"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(read(b""), Reading::Altered { .. }));
        assert!(matches!(read(b"abc"), Reading::Altered { .. }));
    }

    #[test]
    fn scrambled_strings_are_not_read_as_names() {
        let data = sample(29, 32);
        let strings = read_section(&data, STRINGS);
        let mut bad = data.clone();
        for b in &mut bad[strings.0..strings.0 + strings.1] {
            *b = (*b ^ 0xa5) | 0x80;
        }
        assert!(matches!(read(&bad), Reading::HeaderOnly { why, .. } if why.contains("encrypted")));
    }

    /// A section's (offset, size), from a header of pairs, or of triples for version 39.
    fn read_section(data: &[u8], which: usize) -> (usize, usize) {
        let r = Reader::new(data);
        let stride = if r.u32_le(4) == Some(39) { 12 } else { 8 };
        (r.u32_le(8 + which * stride).unwrap() as usize, r.u32_le(12 + which * stride).unwrap() as usize)
    }

    #[test]
    fn method_owners_must_agree_with_the_declaring_types_range() {
        for layout in LAYOUTS {
            let original = sample(layout.version, layout.method_size);
            let mut wrong_owner = original.clone();
            let (methods, _) = read_section(&wrong_owner, METHODS);
            let owner = methods + layout.method_type;
            wrong_owner[owner..owner + 4].copy_from_slice(&2u32.to_le_bytes());
            assert!(matches!(read(&wrong_owner), Reading::HeaderOnly { .. }));
            let mut overlap = original;
            let (types, _) = read_section(&overlap, TYPES);
            let start = types + 2 * layout.type_size + layout.type_method_start;
            overlap[start..start + 4].copy_from_slice(&0u32.to_le_bytes());
            // Both ranges remain inside the method table and their counts still add up to four.
            assert!(matches!(read(&overlap), Reading::HeaderOnly { .. }));
        }
    }

    #[test]
    fn every_cut_short_prefix_is_handled() {
        for (version, method_size) in all_layouts() {
            let data = sample(version, method_size);
            for n in 0..data.len() {
                match read(&data[..n]) {
                    Reading::Readable(m) => panic!("{n} bytes of {} read as whole: version {}", data.len(), m.version()),
                    Reading::Altered { .. } | Reading::HeaderOnly { .. } => {}
                }
            }
        }
    }

    #[test]
    fn huge_numbers_in_every_header_word_do_not_panic() {
        for (version, method_size, header) in [(31, 36, 256), (39, 32, 380)] {
            let base = sample(version, method_size);
            for word in 2..(header / 4) {
                for value in [u32::MAX, u32::MAX - 1, i32::MAX as u32, 1 << 31, 0] {
                    let mut data = base.clone();
                    data[word * 4..word * 4 + 4].copy_from_slice(&value.to_le_bytes());
                    if let Reading::Readable(m) = read(&data) {
                        walk(&m);
                    }
                }
            }
        }
    }

    #[test]
    fn huge_numbers_in_records_do_not_panic() {
        // Overwrite each word in the records with extreme values, then read every row back.
        for (version, method_size) in [(31, 36), (39, 32)] {
            let base = sample(version, method_size);
            let first = read_section(&base, LITERALS).0;
            for at in (first..base.len().saturating_sub(4)).step_by(2) {
                for value in [u32::MAX, 1 << 31, 0x7fff_ffff, 0xffff] {
                    let mut data = base.clone();
                    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
                    if let Reading::Readable(m) = read(&data) {
                        walk(&m);
                    }
                }
            }
        }
    }

    fn walk(m: &Metadata<'_>) {
        for i in 0..m.image_count() {
            let _ = m.image(i);
        }
        for i in 0..m.type_count() {
            let _ = m.type_def(i);
            let _ = m.image_of(i);
        }
        for i in 0..m.method_count() {
            let _ = m.method(i);
        }
        for i in 0..m.literal_count() {
            let _ = m.literal(i, 16);
        }
        let _ = m.field_count();
    }

    #[test]
    fn an_empty_but_correct_file_reads_as_empty() {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&SANITY.to_le_bytes());
        data[4..8].copy_from_slice(&31i32.to_le_bytes());
        for i in 0..SECTIONS.len() {
            data[8 + i * 8..12 + i * 8].copy_from_slice(&256u32.to_le_bytes());
        }
        let m = readable(&data);
        assert_eq!((m.image_count(), m.type_count(), m.method_count(), m.literal_count()), (0, 0, 0, 0));
    }

    #[test]
    fn known_versions_are_listed_once() {
        assert_eq!(known_versions(), [24, 27, 29, 31, 39]);
    }
}
