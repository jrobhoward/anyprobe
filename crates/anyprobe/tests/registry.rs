//! `anyprobe::list()` in a binary that defines probes: each probe is listed
//! once, with its arguments, their types and where it was written. On
//! targets with no probe backend the list is empty.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use anyprobe::registry::{ArgType, Origin, ProbeInfo};

mod net {
    pub const START_LINE: u32 = line!() + 5;
    anyprobe::probes! {
        provider = "registry";

        /// A request started.
        pub fn request__start(id: u64, path: &str, ok: bool, code: i32);
        pub fn request__done();
    }
}

const GET_LINE: u32 = line!() + 2;
#[anyprobe::probe(provider = "registry", debug(opts), ret = debug)]
fn get(id: u32, opts: &Opts, data: &[u8]) -> Result<u8, String> {
    let _ = (id, opts, data);
    Ok(1)
}

#[derive(Debug)]
struct Opts;

#[anyprobe::probe(provider = "registry", unwind)]
async fn fetch(id: u64) -> u64 {
    id
}

#[anyprobe::probe(provider = "registry", unwind)]
fn risky(p: *const u8, c: char) {
    let _ = (p, c);
}

#[anyprobe::probe(provider = "registry", debug(a, b, c, d))]
fn wide(a: &Opts, b: &Opts, c: &Opts, d: &Opts) {
    let _ = (a, b, c, d);
}

#[anyprobe::probe(provider = "registry", skip(_t))]
fn generic<T: Copy>(n: usize, _t: T) -> usize {
    n
}

#[cfg(feature = "serde")]
#[anyprobe::probe(provider = "registry", serde(q))]
fn query(q: &[u32]) {
    let _ = q;
}

#[cfg(feature = "autoref")]
#[anyprobe::probe(provider = "registry")]
fn auto(opts: &Opts) {
    let _ = opts;
}

/// Calls every function, so no linker can drop one.
fn call_all() {
    net::request__start::fire(1, "/", true, -1);
    net::request__done::fire();
    let _ = get(1, &Opts, b"x");
    let _ = futures_lite_block_on(fetch(1));
    risky(core::ptr::null(), 'x');
    wide(&Opts, &Opts, &Opts, &Opts);
    let _ = generic(1, 1u8) + generic(2, 'c');
    #[cfg(feature = "serde")]
    query(&[1]);
    #[cfg(feature = "autoref")]
    auto(&Opts);
}

/// Polls `f` to completion. `fetch` never waits.
fn futures_lite_block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// Every probe of the provider `registry`, by name.
fn probes() -> Vec<ProbeInfo<'static>> {
    call_all();
    anyprobe::list()
        .map(|p| p.expect("records written by this anyprobe"))
        .filter(|p| p.provider == "registry")
        .collect()
}

fn find(name: &str) -> ProbeInfo<'static> {
    let all = probes();
    let mut found = all.iter().filter(|p| p.name == name);
    let probe = found
        .next()
        .unwrap_or_else(|| panic!("no probe {name} in {all:#?}"))
        .clone();
    assert!(found.next().is_none(), "{name} listed twice");
    probe
}

fn args(probe: &ProbeInfo<'_>) -> Vec<(String, ArgType)> {
    probe
        .args
        .iter()
        .map(|a| (a.name.to_owned(), a.ty))
        .collect()
}

fn named(args: &[(&str, ArgType)]) -> Vec<(String, ArgType)> {
    args.iter().map(|(n, t)| ((*n).to_owned(), *t)).collect()
}

fn listed() -> bool {
    anyprobe::BACKEND != "noop"
}

#[test]
fn list____no_backend____is_empty() {
    if !listed() {
        assert_eq!(anyprobe::list().count(), 0);
    }
}

#[test]
fn list____every_record____parses() {
    call_all();
    for probe in anyprobe::list() {
        probe.unwrap();
    }
}

#[test]
fn list____probes_block____gives_each_probe_with_its_source() {
    if !listed() {
        return;
    }
    let p = find("request__start");
    assert_eq!(p.origin, Origin::Probes);
    assert_eq!(p.function, None);
    assert_eq!(p.module_path, "registry::net");
    assert!(p.file.ends_with("registry.rs"), "{}", p.file);
    assert_eq!(p.line, net::START_LINE);
    assert_eq!(
        args(&p),
        named(&[
            ("id", ArgType::U64),
            ("path", ArgType::Str),
            ("ok", ArgType::Bool),
            ("code", ArgType::I32),
        ])
    );
    assert!(find("request__done").args.is_empty());
}

#[test]
fn list____probe_attribute____gives_entry_and_return_with_encodings() {
    if !listed() {
        return;
    }
    let entry = find("get__entry");
    assert_eq!(entry.origin, Origin::Entry);
    assert_eq!(entry.function, Some("get"));
    assert_eq!(entry.module_path, "registry");
    assert_eq!(entry.line, GET_LINE);
    assert_eq!(
        args(&entry),
        named(&[
            ("id", ArgType::U32),
            ("opts", ArgType::Debug),
            ("data", ArgType::Bytes),
        ])
    );
    let ret = find("get__return");
    assert_eq!(ret.origin, Origin::Return);
    assert_eq!(ret.line, GET_LINE);
    assert_eq!(args(&ret), named(&[("ret", ArgType::Debug)]));
}

#[test]
fn list____async_unwind____passes_invocation_and_panicking() {
    if !listed() {
        return;
    }
    assert_eq!(
        args(&find("fetch__entry")),
        named(&[("invocation", ArgType::U64), ("id", ArgType::U64)])
    );
    assert_eq!(
        args(&find("fetch__return")),
        named(&[("invocation", ArgType::U64)])
    );
    let unwind = find("fetch__unwind");
    assert_eq!(unwind.origin, Origin::Unwind);
    assert_eq!(
        args(&unwind),
        named(&[("invocation", ArgType::U64), ("panicking", ArgType::Bool)])
    );
}

#[test]
fn list____sync_unwind____has_no_arguments() {
    if !listed() {
        return;
    }
    assert_eq!(
        args(&find("risky__entry")),
        named(&[("p", ArgType::Ptr), ("c", ArgType::Char)])
    );
    assert!(find("risky__unwind").args.is_empty());
}

#[test]
fn list____collapsed_arguments____are_one_object() {
    if !listed() {
        return;
    }
    assert_eq!(
        args(&find("wide__entry")),
        named(&[("args", ArgType::Object)])
    );
}

#[test]
fn list____generic_fn____is_listed_once() {
    if !listed() {
        return;
    }
    // `find` fails if listed twice: one record per definition, not per
    // instantiation.
    assert_eq!(
        args(&find("generic__entry")),
        named(&[("n", ArgType::Usize)])
    );
}

#[cfg(feature = "serde")]
#[test]
fn list____serde_argument____is_json() {
    if !listed() {
        return;
    }
    assert_eq!(args(&find("query__entry")), named(&[("q", ArgType::Json)]));
}

#[cfg(feature = "autoref")]
#[test]
fn list____autoref_argument____is_auto() {
    if !listed() {
        return;
    }
    assert_eq!(
        args(&find("auto__entry")),
        named(&[("opts", ArgType::Auto)])
    );
}
