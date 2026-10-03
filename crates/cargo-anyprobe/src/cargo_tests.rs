//! Tests for the `cargo build` invocation and reading its messages.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

#[test]
fn args____every_option____is_passed_through() {
    let build = Build {
        example: Some("attr".to_owned()),
        package: Some("anyprobe".to_owned()),
        profile: Some("release-lto".to_owned()),
        target: Some("aarch64-apple-darwin".to_owned()),
        features: vec!["serde".to_owned()],
        no_default_features: true,
        ..Build::default()
    };
    assert_eq!(
        build.args().join(" "),
        "build --message-format=json-render-diagnostics --example attr --package anyprobe \
         --profile release-lto --target aarch64-apple-darwin --features serde --no-default-features"
    );
}

#[test]
fn executable____messages____picks_the_named_target_of_that_kind() {
    let messages = r#"{"reason":"compiler-artifact","target":{"name":"attr","kind":["lib"]},"executable":null}
not json
{"reason":"compiler-artifact","target":{"name":"attr","kind":["bin"]},"executable":"/t/attr-bin"}
{"reason":"compiler-artifact","target":{"name":"attr","kind":["example"]},"executable":"/t/examples/attr"}
{"reason":"compiler-artifact","target":{"name":"work","kind":["example"]},"executable":"/t/examples/work"}
{"reason":"build-finished","success":true}"#;
    assert_eq!(
        executable(messages, "example", "attr"),
        Some(PathBuf::from("/t/examples/attr"))
    );
    assert_eq!(
        executable(messages, "bin", "attr"),
        Some(PathBuf::from("/t/attr-bin"))
    );
    assert_eq!(executable(messages, "example", "nope"), None);
}
