//! `#[probe]` on functions with each kind of argument encoding, for
//! `spike/scripts/attach-linux-attr.sh` and
//! `spike/scripts/attach-windows-attr.ps1`.
//!
//! Usage: `attr [ITERATIONS] [INTERVAL_MS]`; `ITERATIONS` of 0 (the default)
//! runs until killed. Each iteration calls every probed function once with
//! the iteration number as `id`. The probes' enabled checks are inside the
//! functions; encoding runs only behind them, so a tracer that reads an
//! encoded argument has shown the check turned on.
//!
//! Probes, all in the provider `attr`:
//!
//! - `lookup__entry(id, path)`, `lookup__return(ret)`: native arguments and
//!   a native return value.
//! - `query__entry(id, q)`, `query__return(ret)`: `q` as JSON, the return
//!   value as `{:?}`.
//! - `wide__entry(args)`: more arguments than fit, as one JSON object.
//! - `counter_bump__entry(self, by)`: a method with `debug(self)`.
//! - `five__entry(id, neg, c, on, last)`: five native values, the most a
//!   probe passes in registers, called with `(id, -12, 13, true, 16)`.
//!   The fifth register is checked on its own value.
//! - `optional__entry(name, key, label)`: `Option<&str>`, `Option<&[u8]>`
//!   and `&CStr` passed natively. Even ids pass `(Some("opt"), None,
//!   c"cee")`, odd ids `(None, Some(b"k"), c"cee")`.

use std::io::Write;
use std::time::Duration;

#[derive(Debug, serde::Serialize)]
struct Query {
    table: &'static str,
    limit: u32,
}

#[anyprobe::probe(provider = "attr", ret = native)]
fn lookup(id: u64, path: &str) -> u64 {
    id * 10 + path.len() as u64
}

#[anyprobe::probe(provider = "attr", serde(q), ret = debug)]
fn query(id: u64, q: &Query) -> Result<u32, String> {
    if id.is_multiple_of(2) {
        Ok(q.limit)
    } else {
        Err(format!("odd {id}"))
    }
}

#[anyprobe::probe(provider = "attr", debug(tags))]
fn wide(id: u64, a: &str, b: &str, c: &str, tags: Vec<&str>) -> usize {
    id as usize + a.len() + b.len() + c.len() + tags.len()
}

#[anyprobe::probe(provider = "attr")]
fn five(id: u64, neg: i64, c: u32, on: bool, last: u64) -> u64 {
    id ^ neg.unsigned_abs() ^ u64::from(c) ^ u64::from(on) ^ last
}

#[anyprobe::probe(provider = "attr")]
fn optional(name: Option<&str>, key: Option<&[u8]>, label: &std::ffi::CStr) -> usize {
    name.map_or(0, str::len) + key.map_or(0, <[u8]>::len) + label.to_bytes().len()
}

#[derive(Debug)]
struct Counter {
    n: u64,
}

impl Counter {
    #[anyprobe::probe(provider = "attr", name = "counter_bump", debug(self))]
    fn bump(&mut self, by: u64) {
        self.n += by;
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let interval = Duration::from_millis(args.next().and_then(|a| a.parse().ok()).unwrap_or(100));

    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "pid={}", std::process::id());
    let _ = writeln!(out, "backend={}", anyprobe::BACKEND);
    #[cfg(windows)]
    {
        let guid = anyprobe::__private::etw::guid_string("attr");
        let _ = writeln!(out, "etw-provider=attr etw-guid={guid}");
    }
    let _ = out.flush();

    let q = Query {
        table: "rows",
        limit: 5,
    };
    let mut counter = Counter { n: 0 };
    let mut checksum = 0u64;
    let mut i = 0u64;
    while iterations == 0 || i < iterations {
        checksum ^= lookup(i, "/index");
        checksum ^= u64::from(query(i, &q).unwrap_or(0));
        checksum ^= wide(i, "x", "y", "z", vec!["t"]) as u64;
        counter.bump(1);
        checksum ^= five(i, -12, 13, true, 16);
        let (name, key) = if i.is_multiple_of(2) {
            (Some("opt"), None)
        } else {
            (None, Some(&b"k"[..]))
        };
        checksum ^= optional(name, key, c"cee") as u64;

        i += 1;
        std::thread::sleep(interval);
    }
    let _ = writeln!(out, "done checksum={checksum:#x} counter={}", counter.n);
}
