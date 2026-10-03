//! `probes!` from a user's point of view, with no tracer attached.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

anyprobe::probes! {
    provider = "anyprobe_test";

    /// A probe with one of each argument kind.
    pub fn everything(a: u8, b: i64, c: bool, d: *const u32, text: &str);
    fn empty();
    fn bytes(data: &[u8]);
}

mod nested {
    anyprobe::probes! {
        pub fn inner(x: u32);
    }
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
    everything::fire(1, -2, true, &value, "text");
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
