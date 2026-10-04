#![allow(non_snake_case)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use super::*;

/// The text `text` would pass for one value, and the byte after it.
fn one(value: Value<'_>) -> (String, u8) {
    let mut out = None;
    text([value], |[s]| {
        // SAFETY: `text` documents a NUL after each payload, inside the
        // buffer `s` points into, valid for the duration of this call.
        let after = unsafe { *s.as_ptr().add(s.len()) };
        out = Some((s.to_owned(), after));
    });
    out.expect("text calls fire")
}

fn object_of<const N: usize>(names: [&str; N], values: [Value<'_>; N]) -> String {
    let mut out = None;
    object(names, values, |s| out = Some(s.to_owned()));
    out.expect("object calls fire")
}

// Fields are read only through `Debug`.
#[allow(dead_code)]
#[derive(Debug)]
struct Point {
    x: i32,
    label: &'static str,
}

#[test]
fn text____debug_value____is_its_debug_output() {
    let p = Point { x: -3, label: "a" };
    let (s, nul) = one(Value::debug(&p));
    assert_eq!(s, r#"Point { x: -3, label: "a" }"#);
    assert_eq!(nul, 0);
}

#[test]
fn text____several_values____each_is_separate_and_terminated() {
    let a = 1u8;
    let b = "two";
    let mut seen = Vec::new();
    text([Value::debug(&a), Value::debug(&b)], |[x, y]| {
        seen.push(x.to_owned());
        seen.push(y.to_owned());
        // SAFETY: as in `one`.
        let between = unsafe { *x.as_ptr().add(x.len()) };
        assert_eq!(between, 0);
    });
    assert_eq!(seen, ["1", r#""two""#]);
}

#[test]
fn text____native_values____are_written_as_json() {
    assert_eq!(one(Value::U64(7)).0, "7");
    assert_eq!(one(Value::I64(-7)).0, "-7");
    assert_eq!(one(Value::Bool(true)).0, "true");
    assert_eq!(one(Value::Char('q')).0, r#""q""#);
    assert_eq!(one(Value::Str("a\"b")).0, r#""a\"b""#);
    assert_eq!(one(Value::Bytes(&[1, 2])).0, "[1,2]");
}

#[cfg(feature = "serde")]
#[test]
fn text____serde_value____is_json() {
    #[derive(serde::Serialize)]
    struct Query {
        table: &'static str,
        limit: u32,
    }
    let q = Query {
        table: "rows",
        limit: 5,
    };
    assert_eq!(one(Value::serde(&q)).0, r#"{"table":"rows","limit":5}"#);
}

#[test]
fn object____mixed_values____is_one_json_object() {
    let p = Point { x: 1, label: "q\"" };
    let s = object_of(
        ["id", "path", "p"],
        [Value::U64(4), Value::Str("/x"), Value::debug(&p)],
    );
    assert_eq!(
        s,
        r#"{"id":4,"path":"/x","p":"Point { x: 1, label: \"q\\\"\" }"}"#
    );
}

#[test]
fn object____control_characters____are_escaped() {
    let s = object_of(["t"], [Value::Str("a\nb\u{1}")]);
    assert_eq!(s, r#"{"t":"a\nb\u0001"}"#);
}

#[test]
fn text____longer_than_the_cap____is_cut_at_a_character_boundary_and_marked() {
    // Three-byte characters, so the cut falls inside one.
    let long = "€".repeat(MAX_LEN);
    let (s, nul) = one(Value::debug(&long));
    assert!(s.len() <= MAX_LEN);
    assert!(s.len() > MAX_LEN - 3 - CUT_MARKER.len());
    assert!(s.starts_with("\"€€"));
    assert!(s.ends_with("€..."), "{}", &s[s.len() - 10..]);
    assert_eq!(nul, 0);
}

/// `Debug` output that is the string itself, unquoted, so its length is
/// exact.
struct Raw(String);

impl Debug for Raw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[test]
fn text____exactly_the_cap____is_whole_and_unmarked() {
    let full = "a".repeat(MAX_LEN);
    let (s, nul) = one(Value::debug(&Raw(full.clone())));
    assert_eq!(s, full);
    assert_eq!(nul, 0);
}

#[test]
fn text____one_byte_over_the_cap____is_marked() {
    let over = "a".repeat(MAX_LEN + 1);
    let (s, _) = one(Value::debug(&Raw(over.clone())));
    assert_eq!(s.len(), MAX_LEN);
    assert_eq!(
        &s[..MAX_LEN - CUT_MARKER.len()],
        &over[..MAX_LEN - CUT_MARKER.len()]
    );
    assert!(s.ends_with(CUT_MARKER));
}

#[test]
fn object____cut_inside_a_value____writes_nothing_after_the_cut() {
    // The first value ends two bytes short of the cap with a three-byte
    // character that does not fit; the `,` and the next name would.
    let first = format!("{}€", "a".repeat(MAX_LEN - 8));
    let s = object_of(["a", "b"], [Value::Str(&first), Value::U64(1)]);
    assert!(s.len() <= MAX_LEN);
    assert!(s.ends_with("aaa..."), "{}", &s[s.len() - 10..]);
    assert!(!s.contains("\"b\""), "{}", &s[s.len() - 10..]);
}

#[cfg(feature = "serde")]
#[test]
fn text____serde_longer_than_the_cap____is_valid_utf8_within_the_cap_and_marked() {
    let long = vec!["€"; MAX_LEN];
    let (s, _) = one(Value::serde(&long));
    assert!(s.len() <= MAX_LEN);
    assert!(s.starts_with(r#"["€","€""#));
    assert!(s.ends_with(CUT_MARKER));
}

#[cfg(feature = "serde")]
#[test]
fn text____serde_within_the_cap____is_unmarked() {
    let (s, _) = one(Value::serde(&["a", "b"]));
    assert_eq!(s, r#"["a","b"]"#);
}

#[test]
fn text____buffer____is_reused_between_calls() {
    let first = one(Value::debug(&"a long first value")).0;
    let second = one(Value::U64(1)).0;
    assert_eq!(first, r#""a long first value""#);
    assert_eq!(second, "1");
}

/// Encodes another value from inside its own `Debug`, as a probed function
/// called from a `Debug` impl would.
struct Reentrant;

impl Debug for Reentrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = one(Value::U64(42)).0;
        write!(f, "outer({inner})")
    }
}

#[test]
fn text____encoding_inside_encoding____uses_a_separate_buffer() {
    assert_eq!(one(Value::debug(&Reentrant)).0, "outer(42)");
}

#[test]
fn object____null_value____is_json_null() {
    assert_eq!(
        object_of(["a", "b"], [Value::Null, Value::Str("x")]),
        r#"{"a":null,"b":"x"}"#
    );
}
