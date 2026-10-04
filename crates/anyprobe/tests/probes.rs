//! `probes!` from a user's point of view, with no tracer attached.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

anyprobe::probes! {
    provider = "anyprobe_test";

    /// A probe with several argument kinds, at the five-value limit.
    pub fn everything(a: u8, b: i64, d: *const u32, text: &str);
    fn empty();
    fn bytes(data: &[u8]);
}

mod nested {
    anyprobe::probes! {
        pub fn inner(x: u32);
    }
}

mod pointees {
    use std::ffi::c_void;

    pub struct Request;
    pub struct Borrowed<'a>(pub std::marker::PhantomData<&'a u8>);

    // Pointers to names that only resolve in this module: a local struct,
    // an imported type and a struct with an elided lifetime.
    anyprobe::probes! {
        provider = "anyprobe_test";
        pub fn local_pointee(r: *const Request, v: *mut c_void, b: *const Borrowed);
    }
}

#[test]
fn probes____pointer_to_a_type_from_the_callers_module____compiles() {
    let borrowed = pointees::Borrowed(std::marker::PhantomData);
    assert!(!pointees::local_pointee::enabled());
    pointees::local_pointee::fire(&pointees::Request, std::ptr::null_mut(), &borrowed);
}

#[test]
fn probes____in_a_function_body____compile_and_work() {
    // A module nested in a function body resolves `super::` to the enclosing
    // module, past the function's scope; the provider has to be reachable
    // anyway. Compiling this for Windows is what checks it.
    anyprobe::probes! {
        provider = "anyprobe_local";
        fn local(x: u32);
        fn other();
    }
    assert!(!local::enabled());
    local::fire(1);
    other::fire();
    assert_eq!(local::PROVIDER, "anyprobe_local");
}

#[test]
fn enabled____no_tracer____is_false() {
    assert!(!everything::enabled());
    assert!(!empty::enabled());
    assert!(!bytes::enabled());
    assert!(!nested::inner::enabled());
}

#[test]
fn fire____no_tracer____does_nothing() {
    let value = 7u32;
    everything::fire(1, -2, &value, "text");
    empty::fire();
    bytes::fire(&[1, 2, 3]);
    nested::inner::fire(3);
}

#[test]
fn enabled____checked_repeatedly____stays_false() {
    // On Windows the first check registers the provider.
    for _ in 0..3 {
        assert!(!everything::enabled());
    }
}

#[test]
fn constants____explicit_provider____name_the_probe() {
    assert_eq!(everything::PROVIDER, "anyprobe_test");
    assert_eq!(everything::NAME, "everything");
}

#[test]
fn constants____default_provider____is_the_crate_name() {
    assert_eq!(nested::inner::PROVIDER, "probes");
    assert_eq!(nested::inner::NAME, "inner");
}

#[test]
fn backend____this_target____is_named() {
    let expected = if cfg!(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )) {
        "linux-sdt"
    } else if cfg!(all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )) {
        "macos-dtrace"
    } else if cfg!(all(target_os = "freebsd", target_arch = "x86_64")) {
        "freebsd-dtrace"
    } else if cfg!(windows) {
        "windows-etw"
    } else {
        "noop"
    };
    assert_eq!(anyprobe::BACKEND, expected);
}

#[test]
fn next_id____called_twice____ids_differ_and_are_not_0() {
    let a = anyprobe::next_id();
    let b = anyprobe::next_id();
    assert_ne!(a, 0);
    assert_ne!(b, 0);
    assert_ne!(a, b);
}

#[test]
fn next_id____from_several_threads____never_repeats() {
    let threads: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| (0..1000).map(|_| anyprobe::next_id()).collect::<Vec<_>>()))
        .collect();
    let mut ids: Vec<u64> = threads
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n);
    assert!(!ids.contains(&0));
}

#[test]
fn next_id____and_async_invocation_ids____share_one_counter() {
    let a = anyprobe::next_id();
    let b = anyprobe::__private::next_invocation();
    assert!(b > a);
}

#[test]
fn fire_macro____no_tracer____does_not_evaluate_arguments() {
    let mut evaluated = 0;
    let mut count = |v: u32| {
        evaluated += 1;
        v
    };
    anyprobe::fire!(nested::inner(count(1)));
    anyprobe::fire!(crate::nested::inner(count(2)));
    anyprobe::fire!(self::everything(1, -2, &count(3), &format!("{}", count(4))));
    anyprobe::fire!(empty());
    anyprobe::fire!(bytes(&[1, 2, 3],));
    assert_eq!(evaluated, 0);
}

#[test]
fn fire_macro____as_a_statement_and_an_expression____is_unit() {
    let unit: () = anyprobe::fire!(empty());
    assert_eq!(unit, ());
}
