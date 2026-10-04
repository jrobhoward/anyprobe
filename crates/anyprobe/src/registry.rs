//! The probe registry: a description of every probe compiled into a binary.
//!
//! Each probe defined by [`probes!`](crate::probes) or
//! [`#[probe]`](crate::probe) adds one record to a section of the binary:
//! its provider, name and arguments, and where it was written. The record is
//! built at compile time and holds no pointers, so nothing runs at startup,
//! and the same bytes can be read from the running program with [`list`] or
//! from the file on disk with [`parse`], for any target.
//!
//! A record describes a probe definition, not a probe site. The linker may
//! drop the code of a function nothing calls and keep its record, so a probe
//! listed here can have no site a tracer could attach to.
//!
//! [`list`] covers the executable or library it is called from, not shared
//! libraries loaded alongside it. Targets with no probe backend have no
//! records, and [`list`] returns nothing.
//!
//! ```
//! #[anyprobe::probe(provider = "myapp")]
//! fn handle(id: u64, path: &str) {}
//!
//! for probe in anyprobe::list() {
//!     let probe = probe.expect("records written by this anyprobe");
//!     println!("{}:{} at {}:{}", probe.provider, probe.name, probe.file, probe.line);
//! }
//! # handle(1, "/");
//! ```
//!
//! # Section layout
//!
//! Records follow each other in the section, in no particular order, possibly
//! with zero bytes between them. Each is the 4 bytes `APRB`, the length of
//! the body as 2 little-endian bytes, and the body: UTF-8 fields, each
//! followed by a NUL. The fields are the format version ([`VERSION`]), the
//! provider, the probe name, the origin, the function, the module path, the
//! file, the line and the argument count, then the name and type of each
//! argument. The parser accepts only the names the macros write (see
//! [`parse`]). The section is `anyprobe_probes` on Linux and FreeBSD,
//! `__DATA,__anyprobe` on macOS and `.aprobe` on Windows.

use crate::error::RegistryError;

/// The record format version this anyprobe writes and reads.
pub const VERSION: &str = "1";

/// The bytes that start every record.
const MAGIC: &[u8; 4] = b"APRB";

/// Bytes before a record's body: [`MAGIC`] and the body's length.
const HEADER_LEN: usize = MAGIC.len() + 2;

/// Every probe in this binary.
///
/// Reads the registry section of the executable or library this is called
/// from. The records were written by this anyprobe's macros, so every item
/// is `Ok` unless the section was damaged.
#[must_use]
pub fn list() -> Probes<'static> {
    parse(crate::backend::registry_section())
}

/// The probes in `section`, the contents of a registry section read from a
/// binary.
///
/// A record is [`Malformed`](RegistryError::Malformed) unless its names are
/// ones the macros write: the provider and probe name ASCII letters, digits
/// and `_`, the function and argument names Rust identifiers, the module
/// path a Rust path, and the file free of control characters. A
/// [`ProbeInfo`] from a file of unknown origin can therefore be written into
/// a tracer script or a terminal as it is.
#[must_use]
pub fn parse(section: &[u8]) -> Probes<'_> {
    Probes {
        section,
        offset: 0,
        done: false,
    }
}

/// An iterator over registry records. Returned by [`list`] and [`parse`].
///
/// It ends after the first error, since the bytes after a bad record cannot
/// be trusted to start another.
#[derive(Debug, Clone)]
pub struct Probes<'a> {
    section: &'a [u8],
    offset: usize,
    done: bool,
}

impl<'a> Iterator for Probes<'a> {
    type Item = Result<ProbeInfo<'a>, RegistryError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let rest = self.section.get(self.offset..).unwrap_or_default();
        let skipped = rest.iter().take_while(|&&b| b == 0).count();
        self.offset += skipped;
        let rest = &rest[skipped..];
        if rest.is_empty() {
            self.done = true;
            return None;
        }
        let offset = self.offset;
        let result = read_record(rest, offset).map(|(probe, len)| {
            self.offset += len;
            probe
        });
        if result.is_err() {
            self.done = true;
        }
        Some(result)
    }
}

/// Reads the record at the start of `bytes`, which is at `offset` in the
/// section, and returns it with its length.
fn read_record(bytes: &[u8], offset: usize) -> Result<(ProbeInfo<'_>, usize), RegistryError> {
    let truncated = RegistryError::Truncated { offset };
    let malformed = |what| RegistryError::Malformed { offset, what };
    let magic = bytes.get(..MAGIC.len()).ok_or(truncated.clone())?;
    if magic != MAGIC {
        return Err(RegistryError::NotARecord { offset });
    }
    let len = bytes
        .get(MAGIC.len()..HEADER_LEN)
        .ok_or(truncated.clone())?;
    let len = usize::from(u16::from_le_bytes([len[0], len[1]]));
    let body = bytes.get(HEADER_LEN..HEADER_LEN + len).ok_or(truncated)?;
    let body = core::str::from_utf8(body).map_err(|_| malformed("not UTF-8"))?;
    let body = body
        .strip_suffix('\0')
        .ok_or(malformed("the last field has no NUL"))?;
    let mut fields = body.split('\0');
    let mut field = || fields.next().ok_or(malformed("too few fields"));

    let version = field()?;
    if version != VERSION {
        return Err(RegistryError::UnsupportedVersion {
            offset,
            version: version.to_owned(),
        });
    }
    let provider = Some(field()?)
        .filter(|p| is_ascii_name(p))
        .ok_or(malformed("provider is not a name the macros write"))?;
    let name = Some(field()?)
        .filter(|n| is_ascii_name(n))
        .ok_or(malformed("probe name is not a name the macros write"))?;
    let origin = Origin::from_str(field()?).ok_or(malformed("unknown origin"))?;
    let function = match field()? {
        "" => None,
        f if is_identifier(f) => Some(f),
        _ => return Err(malformed("function is not a Rust identifier")),
    };
    let module_path = Some(field()?)
        .filter(|m| is_module_path(m))
        .ok_or(malformed("module path is not a Rust path"))?;
    let module_path = strip_probe_module(module_path, origin);
    let file = Some(field()?)
        .filter(|f| !f.contains(char::is_control))
        .ok_or(malformed("file holds a control character"))?;
    let line = field()?
        .parse()
        .map_err(|_| malformed("line is not a number"))?;
    let count: usize = field()?
        .parse()
        .map_err(|_| malformed("argument count is not a number"))?;
    let mut args = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        let name = Some(field()?)
            .filter(|n| is_identifier(n))
            .ok_or(malformed("argument name is not a Rust identifier"))?;
        let ty = ArgType::from_str(field()?).ok_or(malformed("unknown argument type"))?;
        args.push(ArgInfo { name, ty });
    }
    if fields.next().is_some() {
        return Err(malformed("too many fields"));
    }
    let probe = ProbeInfo {
        provider,
        name,
        origin,
        function,
        module_path,
        file,
        line,
        args,
    };
    Ok((probe, HEADER_LEN + len))
}

/// Whether `s` is a provider or probe name the macros accept: ASCII
/// letters, digits and `_`, not starting with a digit.
fn is_ascii_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `s` is a Rust identifier without its `r#`, as the macros write a
/// function or argument name.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || unicode_ident::is_xid_start(c))
        && chars.all(unicode_ident::is_xid_continue)
}

/// Whether `s` is a path `module_path!()` gives: identifiers, each possibly
/// raw, joined by `::`.
fn is_module_path(s: &str) -> bool {
    s.split("::")
        .all(|segment| is_identifier(segment.strip_prefix("r#").unwrap_or(segment)))
}

/// The module the probe was written in. The macros record `module_path!()`
/// from inside the modules they generate: the probe's own module for
/// `probes!`, and two more for `#[probe]`.
fn strip_probe_module(path: &str, origin: Origin) -> &str {
    let generated = match origin {
        Origin::Probes => 1,
        Origin::Entry | Origin::Return | Origin::Unwind => 2,
    };
    let mut path = path;
    for _ in 0..generated {
        match path.rsplit_once("::") {
            Some((parent, _)) => path = parent,
            None => break,
        }
    }
    path
}

/// One probe, as its registry record describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProbeInfo<'a> {
    /// The provider: the SDT provider, DTrace provider (without the pid) or
    /// ETW provider name.
    pub provider: &'a str,
    /// The probe name as SDT and ETW spell it. DTrace shows each `__` as `-`.
    pub name: &'a str,
    /// Which macro defined the probe, and for `#[probe]` which probe of the
    /// function it is.
    pub origin: Origin,
    /// The function `#[probe]` annotates; `None` for `probes!`.
    pub function: Option<&'a str>,
    /// The module the probe was written in, as `module_path!()` gives it.
    pub module_path: &'a str,
    /// The source file, as `file!()` gives it.
    pub file: &'a str,
    /// The line: of the probe's name in `probes!`, of the function's name for
    /// `#[probe]`.
    pub line: u32,
    /// The arguments, in the order the probe passes them.
    pub args: Vec<ArgInfo<'a>>,
}

impl ProbeInfo<'_> {
    /// The probe name as DTrace shows it, with each `__` as `-`.
    #[must_use]
    pub fn dtrace_name(&self) -> String {
        self.name.replace("__", "-")
    }

    /// The tracer argument index of each argument: `arg0`, `arg1`, ... on
    /// Linux, macOS and FreeBSD, where `str` and `bytes` arguments take two
    /// values.
    pub fn arg_indices(&self) -> impl Iterator<Item = (usize, &ArgInfo<'_>)> {
        self.args.iter().scan(0, |next, arg| {
            let index = *next;
            *next += arg.ty.slots();
            Some((index, arg))
        })
    }
}

/// One argument of a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArgInfo<'a> {
    /// The argument's name: the parameter name in `probes!` and for
    /// `#[probe]`, `ret` for a return value, `invocation` for an `async fn`'s
    /// invocation id, `panicking` on an `async fn`'s unwind probe, and `args`
    /// for arguments collapsed into one JSON object.
    pub name: &'a str,
    /// How the value reaches the tracer.
    pub ty: ArgType,
}

/// Which macro defined a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Origin {
    /// [`probes!`](crate::probes).
    Probes,
    /// The entry probe of a function under [`#[probe]`](crate::probe).
    Entry,
    /// The return probe of a function under `#[probe]`.
    Return,
    /// The unwind probe of a function under `#[probe(unwind)]`.
    Unwind,
}

impl Origin {
    /// The name the record uses: `probes`, `entry`, `return` or `unwind`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Probes => "probes",
            Origin::Entry => "entry",
            Origin::Return => "return",
            Origin::Unwind => "unwind",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        [
            Origin::Probes,
            Origin::Entry,
            Origin::Return,
            Origin::Unwind,
        ]
        .into_iter()
        .find(|o| o.as_str() == s)
    }
}

/// How an argument's value reaches the tracer.
///
/// On Linux, macOS and FreeBSD every integer, `bool`, `char` and pointer is
/// one 64-bit value, zero-extended (sign-extended for signed integers); ETW
/// keeps the width. `str` and `bytes` are two values, a pointer and a length,
/// and so are `opt_str` and `opt_bytes`, whose pointer is null (and length 0)
/// for `None`. `cstr` is one value, a pointer to NUL-terminated bytes. The
/// encoded types are UTF-8 strings passed like `str`, with a NUL after the
/// last byte, so tools that read up to a NUL see the same text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArgType {
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`, and a type passed through [`Native`](crate::Native).
    U64,
    /// `usize`.
    Usize,
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i64`.
    I64,
    /// `isize`.
    Isize,
    /// `bool`: 0 or 1.
    Bool,
    /// `char`, as its code point.
    Char,
    /// A raw pointer, as an address.
    Ptr,
    /// `&str`: pointer and length.
    Str,
    /// `&[u8]`: pointer and length.
    Bytes,
    /// `Option<&str>`: pointer and length; a null pointer for `None`. ETW
    /// records `None` as an empty string.
    OptStr,
    /// `Option<&[u8]>`: pointer and length; a null pointer for `None`. ETW
    /// records `None` as no bytes.
    OptBytes,
    /// `&CStr`: a pointer to NUL-terminated bytes.
    CStr,
    /// JSON text, from `serde(..)` or `ret = serde`.
    Json,
    /// `{:?}` text, from `debug(..)` or `ret = debug`.
    Debug,
    /// JSON or `{:?}` text, picked by the `autoref` feature from the
    /// argument's type.
    Auto,
    /// A JSON object holding every argument, when they would take more than
    /// five values.
    Object,
}

const ARG_TYPES: [(ArgType, &str); 22] = [
    (ArgType::U8, "u8"),
    (ArgType::U16, "u16"),
    (ArgType::U32, "u32"),
    (ArgType::U64, "u64"),
    (ArgType::Usize, "usize"),
    (ArgType::I8, "i8"),
    (ArgType::I16, "i16"),
    (ArgType::I32, "i32"),
    (ArgType::I64, "i64"),
    (ArgType::Isize, "isize"),
    (ArgType::Bool, "bool"),
    (ArgType::Char, "char"),
    (ArgType::Ptr, "ptr"),
    (ArgType::Str, "str"),
    (ArgType::Bytes, "bytes"),
    (ArgType::OptStr, "opt_str"),
    (ArgType::OptBytes, "opt_bytes"),
    (ArgType::CStr, "cstr"),
    (ArgType::Json, "json"),
    (ArgType::Debug, "debug"),
    (ArgType::Auto, "auto"),
    (ArgType::Object, "object"),
];

impl ArgType {
    /// The name the record uses: the Rust type for the native ones, else
    /// `ptr`, `str`, `bytes`, `opt_str`, `opt_bytes`, `cstr`, `json`,
    /// `debug`, `auto` or `object`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        ARG_TYPES
            .iter()
            .find(|(t, _)| *t == self)
            .map_or("", |(_, s)| s)
    }

    fn from_str(s: &str) -> Option<Self> {
        ARG_TYPES.iter().find(|(_, n)| *n == s).map(|(t, _)| *t)
    }

    /// How many tracer arguments the value takes on Linux, macOS and FreeBSD:
    /// 2 for a pointer and length, else 1.
    #[must_use]
    pub fn slots(self) -> usize {
        if self.is_text() || matches!(self, ArgType::Bytes | ArgType::OptStr | ArgType::OptBytes) {
            2
        } else {
            1
        }
    }

    /// Whether the value is UTF-8 text passed as a pointer and a length:
    /// `str` and the encoded types.
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(
            self,
            ArgType::Str | ArgType::Json | ArgType::Debug | ArgType::Auto | ArgType::Object
        )
    }

    /// Whether a signed integer.
    #[must_use]
    pub fn is_signed(self) -> bool {
        matches!(
            self,
            ArgType::I8 | ArgType::I16 | ArgType::I32 | ArgType::I64 | ArgType::Isize
        )
    }
}

/// The length of the record for `body`, for the array type `register!`
/// declares.
#[doc(hidden)]
#[must_use]
pub const fn record_len(body: &str) -> usize {
    HEADER_LEN + body.len()
}

/// The record for `body`: [`MAGIC`], the body's length and the body.
/// Evaluated at compile time; a body too long for its length field fails
/// the build.
#[doc(hidden)]
#[must_use]
pub const fn record<const N: usize>(body: &str) -> [u8; N] {
    let body = body.as_bytes();
    assert!(N == HEADER_LEN + body.len(), "record length mismatch");
    assert!(body.len() <= u16::MAX as usize, "registry record too long");
    let mut out = [0u8; N];
    let mut i = 0;
    while i < MAGIC.len() {
        out[i] = MAGIC[i];
        i += 1;
    }
    let len = (body.len() as u16).to_le_bytes();
    out[MAGIC.len()] = len[0];
    out[MAGIC.len() + 1] = len[1];
    let mut i = 0;
    while i < body.len() {
        out[HEADER_LEN + i] = body[i];
        i += 1;
    }
    out
}

/// Emits one registry record into `section`. Called by each backend's
/// `register!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_record {
    ($section:literal, $body:expr) => {
        const _: () = {
            const BODY: &str = $body;
            #[used]
            #[unsafe(link_section = $section)]
            static RECORD: [u8; $crate::registry::record_len(BODY)] =
                $crate::registry::record(BODY);
        };
    };
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod registry_tests;
