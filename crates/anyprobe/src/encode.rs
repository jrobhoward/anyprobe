//! Encoding of `serde` and `debug` arguments for `#[probe]`.
//!
//! Generated code builds one [`Value`] per argument inside the enabled check
//! and passes them to a cold helper, which writes them into a thread-local
//! buffer with [`text`] or [`object`] and fires the probe with the result.
//! Nothing here runs unless a tracer is attached.
//!
//! Each payload is followed by a NUL byte that its length excludes, so tools
//! that only read NUL-terminated strings (perf, gdb) read it too. A payload
//! longer than [`MAX_LEN`] bytes is cut at the last whole UTF-8 character that
//! leaves room for [`CUT_MARKER`], which then ends it. A payload ends with the
//! marker only if it was cut.

use core::fmt::{self, Debug, Write as _};
use core::ops::Range;
use std::cell::RefCell;

/// Longest payload, in bytes, before its terminating NUL.
pub const MAX_LEN: usize = 4096;

/// The last bytes of a payload that was cut at [`MAX_LEN`].
pub const CUT_MARKER: &str = "...";

/// Initial capacity of each thread's buffer, so a first small payload does
/// not grow it several times.
const INITIAL_CAPACITY: usize = 256;

/// One argument or return value, as generated code passes it to the encoder.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum Value<'a> {
    /// An unsigned integer.
    U64(u64),
    /// A signed integer.
    I64(i64),
    /// A `bool`.
    Bool(bool),
    /// A `char`.
    Char(char),
    /// A raw pointer's address.
    Ptr(usize),
    /// A `&str`.
    Str(&'a str),
    /// A `&[u8]`.
    Bytes(&'a [u8]),
    /// An absent value, `None` of an `Option<&str>` or `Option<&[u8]>`.
    Null,
    /// A value encoded with `{:?}`.
    Debug(&'a dyn Debug),
    /// A value encoded as JSON.
    #[cfg(feature = "serde")]
    Serde(&'a dyn SerializeJson),
}

impl Debug for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::U64(v) => f.debug_tuple("U64").field(v).finish(),
            Value::I64(v) => f.debug_tuple("I64").field(v).finish(),
            Value::Bool(v) => f.debug_tuple("Bool").field(v).finish(),
            Value::Char(v) => f.debug_tuple("Char").field(v).finish(),
            Value::Ptr(v) => f.debug_tuple("Ptr").field(v).finish(),
            Value::Str(v) => f.debug_tuple("Str").field(v).finish(),
            Value::Bytes(v) => f.debug_tuple("Bytes").field(v).finish(),
            Value::Null => f.write_str("Null"),
            Value::Debug(v) => f.debug_tuple("Debug").field(v).finish(),
            #[cfg(feature = "serde")]
            Value::Serde(_) => f.write_str("Serde(..)"),
        }
    }
}

impl<'a> Value<'a> {
    /// A value encoded with `{:?}`.
    #[inline(always)]
    #[must_use]
    pub fn debug<T: Debug>(value: &'a T) -> Self {
        Value::Debug(value)
    }

    /// A value encoded as JSON.
    #[cfg(feature = "serde")]
    #[inline(always)]
    #[must_use]
    pub fn serde<T: serde::Serialize>(value: &'a T) -> Self {
        Value::Serde(value)
    }

    /// Appends the value as text: `Debug` and `Serde` values as their
    /// encoding, the rest as they would appear in JSON.
    fn write_text(&self, out: &mut Capped<'_>) {
        match self {
            Value::Debug(v) => {
                let _ = write!(out, "{v:?}");
            }
            #[cfg(feature = "serde")]
            Value::Serde(v) => v.write_json(out),
            _ => self.write_json(out),
        }
    }

    /// Appends the value as a JSON value. `Debug` output becomes a JSON
    /// string.
    fn write_json(&self, out: &mut Capped<'_>) {
        let _ = match self {
            Value::U64(v) => write!(out, "{v}"),
            Value::I64(v) => write!(out, "{v}"),
            Value::Bool(v) => write!(out, "{v}"),
            Value::Ptr(v) => write!(out, "{v}"),
            Value::Char(c) => {
                let mut utf8 = [0; 4];
                json_string(out, c.encode_utf8(&mut utf8))
            }
            Value::Str(s) => json_string(out, s),
            Value::Bytes(bytes) => json_bytes(out, bytes),
            Value::Null => out.write_str("null"),
            Value::Debug(v) => {
                // Escapes as it goes, so the output is never held twice.
                out.write_char('"')
                    .and_then(|()| write!(JsonEscape(out), "{v:?}"))
                    .and_then(|()| out.write_char('"'))
            }
            #[cfg(feature = "serde")]
            Value::Serde(v) => {
                v.write_json(out);
                Ok(())
            }
        };
    }
}

/// Object-safe `serde::Serialize`, so [`Value`] can hold any serializable
/// type behind a reference.
#[cfg(feature = "serde")]
pub trait SerializeJson {
    /// Appends `self` as JSON.
    #[doc(hidden)]
    fn write_json(&self, out: &mut Capped<'_>);
}

#[cfg(feature = "serde")]
impl<T: serde::Serialize + ?Sized> SerializeJson for T {
    fn write_json(&self, out: &mut Capped<'_>) {
        // An error means the cap was reached or the value refused to
        // serialize; either way the bytes written so far are kept.
        let _ = serde_json::to_writer(out, self);
    }
}

/// Encodes each value as text and calls `fire` with the results.
///
/// The slices point into a thread-local buffer, each followed by a NUL byte.
/// They are valid only during `fire`.
pub fn text<const N: usize>(values: [Value<'_>; N], fire: impl FnOnce([&str; N])) {
    with_buffer(|buf| {
        let ranges = values.map(|value| append(buf, |out| value.write_text(out)));
        fire(ranges.map(|r| as_str(buf, r)));
    });
}

/// Encodes the values as one JSON object, `{"name": value, ...}`, and calls
/// `fire` with it. Used when a probe has more arguments than the backends
/// can pass separately.
pub fn object<const N: usize>(names: [&str; N], values: [Value<'_>; N], fire: impl FnOnce(&str)) {
    with_buffer(|buf| {
        let range = append(buf, |out| {
            let _ = out.write_char('{');
            for (i, (name, value)) in names.iter().zip(&values).enumerate() {
                if i > 0 {
                    let _ = out.write_char(',');
                }
                let _ = json_string(out, name);
                let _ = out.write_char(':');
                value.write_json(out);
            }
            let _ = out.write_char('}');
        });
        fire(as_str(buf, range));
    });
}

thread_local! {
    static BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(INITIAL_CAPACITY));
}

/// Runs `f` with this thread's buffer, emptied. If the buffer is in use (a
/// probe fired while encoding another, from inside a `Debug` impl) or the
/// thread is exiting, `f` gets a fresh one instead.
fn with_buffer(f: impl FnOnce(&mut Vec<u8>)) {
    let mut f = Some(f);
    let _ = BUFFER.try_with(|cell| {
        if let Ok(mut buf) = cell.try_borrow_mut() {
            buf.clear();
            if let Some(f) = f.take() {
                f(&mut buf);
            }
        }
    });
    if let Some(f) = f {
        f(&mut Vec::new());
    }
}

/// Appends one payload with `write` and its NUL, and returns the payload's
/// range. The payload is valid UTF-8: [`Capped`] cuts `str` writes at a
/// character boundary, and anything else (serde's byte writes) is cut back to
/// the last whole character here. A payload that did not fit is cut further,
/// to make room for [`CUT_MARKER`], and ends with it.
fn append(buf: &mut Vec<u8>, write: impl FnOnce(&mut Capped<'_>)) -> Range<usize> {
    let start = buf.len();
    let mut out = Capped {
        buf,
        limit: start + MAX_LEN,
        cut: false,
    };
    write(&mut out);
    let cut = out.cut;
    if cut {
        buf.truncate(buf.len().min(start + MAX_LEN - CUT_MARKER.len()));
    }
    if let Err(e) = core::str::from_utf8(&buf[start..]) {
        buf.truncate(start + e.valid_up_to());
    }
    if cut {
        buf.extend_from_slice(CUT_MARKER.as_bytes());
    }
    let end = buf.len();
    buf.push(0);
    start..end
}

fn as_str(buf: &[u8], range: Range<usize>) -> &str {
    // `append` left every range valid UTF-8; an empty string would only
    // stand in for a bug there.
    core::str::from_utf8(&buf[range]).unwrap_or_default()
}

/// A writer into the buffer that stops at `limit`.
#[doc(hidden)]
#[derive(Debug)]
pub struct Capped<'a> {
    buf: &'a mut Vec<u8>,
    limit: usize,
    /// Set once a write did not fit. Every later write is refused, so a
    /// shorter one cannot land after the gap.
    cut: bool,
}

impl fmt::Write for Capped<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.cut {
            return Err(fmt::Error);
        }
        let room = self.limit.saturating_sub(self.buf.len());
        if s.len() <= room {
            self.buf.extend_from_slice(s.as_bytes());
            return Ok(());
        }
        let mut cut = room;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.buf.extend_from_slice(&s.as_bytes()[..cut]);
        self.cut = true;
        Err(fmt::Error)
    }
}

impl std::io::Write for Capped<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let room = self.limit.saturating_sub(self.buf.len());
        if self.cut || room == 0 {
            self.cut = true;
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        let n = bytes.len().min(room);
        self.buf.extend_from_slice(&bytes[..n]);
        self.cut = n < bytes.len();
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Escapes everything written through it as the inside of a JSON string.
struct JsonEscape<'a, 'b>(&'a mut Capped<'b>);

impl fmt::Write for JsonEscape<'_, '_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut plain = 0;
        for (i, c) in s.char_indices() {
            let escape = match c {
                '"' => "\\\"",
                '\\' => "\\\\",
                '\n' => "\\n",
                '\r' => "\\r",
                '\t' => "\\t",
                c if (c as u32) < 0x20 => "",
                _ => continue,
            };
            self.0.write_str(&s[plain..i])?;
            if escape.is_empty() {
                write!(self.0, "\\u{:04x}", c as u32)?;
            } else {
                self.0.write_str(escape)?;
            }
            plain = i + c.len_utf8();
        }
        self.0.write_str(&s[plain..])
    }
}

fn json_string(out: &mut Capped<'_>, s: &str) -> fmt::Result {
    out.write_char('"')?;
    JsonEscape(out).write_str(s)?;
    out.write_char('"')
}

fn json_bytes(out: &mut Capped<'_>, bytes: &[u8]) -> fmt::Result {
    out.write_char('[')?;
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.write_char(',')?;
        }
        write!(out, "{b}")?;
    }
    out.write_char(']')
}

#[cfg(test)]
#[path = "encode_tests.rs"]
mod encode_tests;
