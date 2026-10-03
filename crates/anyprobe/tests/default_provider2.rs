//! The default provider of a crate whose name ends in a digit. Cargo names
//! this test crate `default_provider2`, which DTrace cannot take as a
//! provider, so both macros default to `default_provider2_`.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

anyprobe::probes! {
    fn hit(n: u64);
}

#[anyprobe::probe]
fn work(n: u64) -> u64 {
    n
}

#[test]
fn probes____crate_name_ends_in_digit____provider_gets_an_underscore() {
    hit::fire(1);
    assert_eq!(hit::PROVIDER, "default_provider2_");
}

#[test]
fn probe____crate_name_ends_in_digit____provider_gets_an_underscore() {
    assert_eq!(work(1), 1);
    let providers: Vec<&str> = anyprobe::list()
        .map(|p| p.expect("records written by this anyprobe"))
        .filter(|p| p.function == Some("work"))
        .map(|p| p.provider)
        .collect();
    if anyprobe::BACKEND != "noop" {
        assert_eq!(providers.len(), 2, "entry and return: {providers:?}");
    }
    assert!(
        providers.iter().all(|p| *p == "default_provider2_"),
        "{providers:?}"
    );
}
