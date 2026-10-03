//! Tests for grouping and `list` output.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;
use crate::test_support::{Rec, entry, parse, section};

fn all() -> Filter {
    Filter::default()
}

#[test]
fn glob____patterns____match_as_shell_globs() {
    assert!(glob("*", ""));
    assert!(glob("*", "fetch__entry"));
    assert!(glob("fetch__*", "fetch__entry"));
    assert!(glob("*__entry", "fetch__entry"));
    assert!(glob("f?tch*y", "fetch__entry"));
    assert!(glob("*a*a*", "banana"));
    assert!(!glob("*__return", "fetch__entry"));
    assert!(!glob("fetch", "fetch__entry"));
    assert!(!glob("?", ""));
}

#[test]
fn groups____records____are_sorted_and_filtered() {
    let bytes = section(&[
        entry("b__entry", "b", &[]),
        Rec {
            provider: "other",
            ..entry("a__entry", "a", &[])
        },
        entry("a__entry", "a", &[]),
    ]);
    let names: Vec<_> = groups(parse(&bytes), None, &all())
        .iter()
        .map(Group::full_name)
        .collect();
    assert_eq!(names, ["app:a__entry", "app:b__entry", "other:a__entry"]);

    let filter = Filter {
        provider: Some("app".to_owned()),
        probe: Some("b*".to_owned()),
    };
    let names: Vec<_> = groups(parse(&bytes), None, &filter)
        .iter()
        .map(Group::full_name)
        .collect();
    assert_eq!(names, ["app:b__entry"]);
}

#[test]
fn groups____sites____mark_probes_without_one() {
    let bytes = section(&[
        entry("kept__entry", "kept", &[]),
        entry("gone__entry", "gone", &[]),
    ]);
    let sites = HashSet::from([site_key("app", "kept__entry")]);
    let g = groups(parse(&bytes), Some(&sites), &all());
    assert_eq!(g[0].name, "gone__entry");
    assert_eq!(g[0].has_site, Some(false));
    assert_eq!(g[1].has_site, Some(true));
    assert_eq!(groups(parse(&bytes), None, &all())[0].has_site, None);
}

#[test]
fn group____same_name_same_args____has_one_layout() {
    let args = [("x", "u32")];
    let bytes = section(&[
        entry("new__entry", "new", &args),
        entry("new__entry", "new", &args),
    ]);
    let g = groups(parse(&bytes), None, &all());
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].records.len(), 2);
    assert_eq!(g[0].layouts(), 1);
    assert!(g[0].args().is_some());
    assert!(warnings(&g).is_empty());
}

#[test]
fn group____same_name_different_args____warns() {
    let bytes = section(&[
        entry("new__entry", "new", &[("x", "u32")]),
        entry("new__entry", "new", &[]),
        entry("new__entry", "new", &[("x", "str")]),
    ]);
    let g = groups(parse(&bytes), None, &all());
    assert_eq!(g[0].layouts(), 3);
    assert!(g[0].args().is_none());
    let w = warnings(&g);
    assert_eq!(w.len(), 1);
    assert!(
        w[0].starts_with("app:new__entry has 3 different argument lists"),
        "{}",
        w[0]
    );
}

#[test]
fn list_text____probes____show_arguments_origin_and_site() {
    let bytes = section(&[
        entry("get__entry", "get", &[("id", "u64"), ("opts", "debug")]),
        Rec {
            name: "tick",
            origin: "probes",
            function: "",
            line: 3,
            ..entry("", "", &[])
        },
    ]);
    let sites = HashSet::from([site_key("app", "get__entry")]);
    let text = list_text(&groups(parse(&bytes), Some(&sites), &all()));
    assert_eq!(
        text,
        "app:get__entry(id: u64, opts: debug)\n\
         \x20   entry of fn get in app::net, src/net.rs:10\n\
         app:tick()  (no site: its code is not in the binary)\n\
         \x20   probes! in app::net, src/net.rs:3\n\
         2 probes in 1 provider\n"
    );
}

#[test]
fn list_text____different_layouts____show_each_definition() {
    let bytes = section(&[
        entry("new__entry", "new", &[("x", "u32")]),
        entry("new__entry", "new", &[]),
    ]);
    let text = list_text(&groups(parse(&bytes), None, &all()));
    assert_eq!(
        text,
        "app:new__entry\n\
         \x20   (x: u32)  entry of fn new in app::net, src/net.rs:10\n\
         \x20   ()  entry of fn new in app::net, src/net.rs:10\n\
         1 probe in 1 provider\n"
    );
}

#[test]
fn list_json____record____has_every_field_and_argument_index() {
    let bytes = section(&[entry(
        "get__entry",
        "get",
        &[("path", "str"), ("id", "u64")],
    )]);
    let json: serde_json::Value =
        serde_json::from_str(&list_json(&groups(parse(&bytes), None, &all()))).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{
            "provider": "app",
            "name": "get__entry",
            "origin": "entry",
            "function": "get",
            "module_path": "app::net",
            "file": "src/net.rs",
            "line": 10,
            "args": [
                { "name": "path", "type": "str", "index": 0 },
                { "name": "id", "type": "u64", "index": 2 },
            ],
            "has_site": null,
        }])
    );
}
