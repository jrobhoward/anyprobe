//! `#[probe]` from a user's point of view, with no tracer attached: every
//! supported signature compiles, and the function behaves as it did without
//! the attribute.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::fmt::Debug;

#[anyprobe::probe]
fn natives(a: u8, b: i64, c: bool, d: char, e: *const u32, text: &str, data: &[u8]) -> usize {
    let _ = (a, b, c, d, e);
    text.len() + data.len()
}

#[anyprobe::probe]
fn references(a: &u32, b: &mut u64, c: &mut str, d: &mut [u8], e: &*mut u8) -> u64 {
    let _ = e;
    c.make_ascii_uppercase();
    d.reverse();
    *b += u64::from(*a);
    *b
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct Query {
    table: &'static str,
    limit: u32,
}

#[derive(Debug)]
struct Opts {
    verbose: bool,
}

struct Conn;

#[cfg_attr(
    feature = "serde",
    anyprobe::probe(name = "db_query", serde(q), debug(opts), skip(conn), ret = debug)
)]
#[cfg_attr(
    not(feature = "serde"),
    anyprobe::probe(name = "db_query", debug(q, opts), skip(conn), ret = debug)
)]
fn query(conn: &mut Conn, id: u64, q: &Query, opts: Opts) -> Result<u32, String> {
    let _ = conn;
    if opts.verbose {
        return Err(format!("{} {id}", q.table));
    }
    Ok(q.limit)
}

#[derive(Clone, Copy)]
struct RowId(u32);

impl anyprobe::Native for RowId {
    fn to_u64(&self) -> u64 {
        u64::from(self.0)
    }
}

type Alias = u64;

#[anyprobe::probe(native(row), native(alias), ret = native)]
fn natives_by_trait(row: RowId, alias: Alias) -> Alias {
    u64::from(row.0) + alias
}

/// More than six operands: the arguments collapse into one JSON object.
#[anyprobe::probe(debug(extra))]
fn many(a: &str, b: &str, c: &str, d: u64, extra: Vec<u32>) -> usize {
    a.len() + b.len() + c.len() + d as usize + extra.len()
}

#[anyprobe::probe(ret = native)]
fn early_return(values: &[u8]) -> u8 {
    for v in values {
        if *v > 3 {
            return *v;
        }
    }
    0
}

#[anyprobe::probe(ret = debug)]
fn question_mark(text: &str) -> Result<u32, std::num::ParseIntError> {
    let n: u32 = text.parse()?;
    Ok(n * 2)
}

#[anyprobe::probe]
fn boxed_error(text: &str) -> Result<u32, Box<dyn std::error::Error>> {
    let n: u32 = text.parse()?;
    Ok(n)
}

#[anyprobe::probe]
fn returns_impl(n: u32) -> impl Iterator<Item = u32> {
    0..n
}

#[anyprobe::probe]
fn returns_borrow(text: &str) -> &str {
    text.trim()
}

#[anyprobe::probe(debug(t), ret = debug)]
fn generic<T: Debug + Clone>(t: &T) -> T {
    t.clone()
}

#[anyprobe::probe(debug(t))]
fn impl_arg(t: impl Debug) -> String {
    format!("{t:?}")
}

#[anyprobe::probe]
fn wildcard(_: u64, mut counter: u32) -> u32 {
    counter += 1;
    counter
}

#[anyprobe::probe]
fn no_arguments() {}

#[derive(Debug, Default)]
struct Counter {
    value: u32,
    name: String,
}

impl Counter {
    #[anyprobe::probe(name = "counter_bump")]
    fn bump(&mut self, by: u32) -> &mut u32 {
        self.value += by;
        &mut self.value
    }

    #[anyprobe::probe(name = "counter_name", debug(self), ret = native)]
    fn name(&self) -> &str {
        &self.name
    }

    #[anyprobe::probe(name = "counter_into")]
    fn into_value(self) -> u32 {
        self.value
    }

    #[anyprobe::probe(name = "counter_new")]
    fn new(value: u32) -> Self {
        Counter {
            value,
            name: String::new(),
        }
    }
}

trait Shape {
    #[anyprobe::probe(name = "shape_area")]
    fn area(&self) -> u64 {
        0
    }
}

impl Shape for Counter {}

/// # Safety
/// `p` must be valid for reads.
#[anyprobe::probe]
unsafe fn unsafe_fn(p: *const u32) -> u32 {
    // SAFETY: the caller guarantees `p` is valid for reads.
    unsafe { *p }
}

#[anyprobe::probe(provider = "other_provider")]
fn other_provider(x: u32) -> u32 {
    #![allow(clippy::identity_op)]
    x + 0
}

#[test]
fn probe____natives____returns_the_bodys_value() {
    let x = 1u32;
    assert_eq!(natives(1, -2, true, 'z', &x, "abc", &[1, 2]), 5);
}

#[test]
fn probe____references____mutations_are_kept() {
    let mut b = 1u64;
    let mut text = String::from("abc");
    let mut data = [1u8, 2, 3];
    let mut byte = 0u8;
    let p: *mut u8 = &mut byte;
    assert_eq!(references(&2, &mut b, &mut text, &mut data, &p), 3);
    assert_eq!(b, 3);
    assert_eq!(text, "ABC");
    assert_eq!(data, [3, 2, 1]);
}

#[test]
fn probe____return_and_question_mark____keep_their_meaning() {
    let q = Query {
        table: "rows",
        limit: 9,
    };
    assert_eq!(query(&mut Conn, 1, &q, Opts { verbose: false }), Ok(9));
    assert_eq!(
        query(&mut Conn, 1, &q, Opts { verbose: true }),
        Err("rows 1".to_owned())
    );
    assert_eq!(early_return(&[1, 5, 7]), 5);
    assert_eq!(early_return(&[1]), 0);
    assert_eq!(question_mark("21"), Ok(42));
    assert!(question_mark("x").is_err());
    assert!(boxed_error("x").is_err());
    assert_eq!(boxed_error("4").unwrap(), 4);
}

#[test]
fn probe____other_signatures____behave_as_written() {
    assert_eq!(natives_by_trait(RowId(2), 3), 5);
    assert_eq!(many("a", "bb", "ccc", 4, vec![1, 2]), 12);
    assert_eq!(returns_impl(3).sum::<u32>(), 3);
    assert_eq!(returns_borrow("  x "), "x");
    assert_eq!(generic(&"g".to_owned()), "g");
    assert_eq!(impl_arg(5), "5");
    assert_eq!(wildcard(0, 1), 2);
    no_arguments();
    assert_eq!(other_provider(7), 7);
    let x = 11;
    // SAFETY: `x` is a live local.
    assert_eq!(unsafe { unsafe_fn(&x) }, 11);
}

#[test]
fn probe____methods____borrow_and_move_self() {
    let mut c = Counter::new(1);
    *c.bump(2) += 10;
    assert_eq!(c.value, 13);
    c.name.push('n');
    assert_eq!(c.name(), "n");
    assert_eq!(c.area(), 0);
    assert_eq!(c.into_value(), 13);
}
