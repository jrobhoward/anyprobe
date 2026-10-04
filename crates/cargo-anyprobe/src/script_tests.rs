//! Tests for the generated scripts.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::collections::HashSet;

use super::*;
use crate::binary::site_key;
use crate::report::{Filter, groups};
use crate::test_support::{entry, parse, section};

const ARGS: [(&str, &str); 5] = [
    ("id", "u32"),
    ("path", "str"),
    ("delta", "i64"),
    ("p", "ptr"),
    ("opts", "json"),
];

fn bin() -> &'static Path {
    Path::new("/srv/app")
}

#[test]
fn bpftrace____arguments____are_read_by_type() {
    let bytes = section(&[entry("get__entry", "get", &ARGS)]);
    let out = bpftrace(bin(), &groups(parse(&bytes), None, &Filter::default()));
    assert!(
        out.contains(
            "usdt:/srv/app:app:get__entry\n{\n\tprintf(\"app:get__entry id=%lu path=%s delta=%ld \
             p=0x%lx opts=%s\\n\", arg0, str(arg1, arg2), (int64)arg3, arg4, str(arg5, arg6));\n}\n"
        ),
        "{out}"
    );
}

#[test]
fn bpftrace____bytes____use_buf() {
    let bytes = section(&[entry("put__entry", "put", &[("data", "bytes")])]);
    let out = bpftrace(bin(), &groups(parse(&bytes), None, &Filter::default()));
    assert!(out.contains("data=%r\\n\", buf(arg0, arg1));"), "{out}");
}

#[test]
fn bpftrace____no_site_or_conflict____is_commented() {
    let bytes = section(&[
        entry("gone__entry", "gone", &[]),
        entry("new__entry", "new", &[("x", "u32")]),
        entry("new__entry", "new", &[]),
    ]);
    let sites = HashSet::from([site_key("app", "new__entry")]);
    let out = bpftrace(
        bin(),
        &groups(parse(&bytes), Some(&sites), &Filter::default()),
    );
    assert!(
        out.contains("// app:gone__entry: no site: its code is not in the binary\n"),
        "{out}"
    );
    assert!(!out.contains("usdt:/srv/app:app:gone__entry"), "{out}");
    assert!(
        out.contains(
            "// app:new__entry: functions define it with different arguments; printing none\n\
             usdt:/srv/app:app:new__entry\n{\n\tprintf(\"app:new__entry\\n\");\n}\n"
        ),
        "{out}"
    );
}

#[test]
fn dtrace____arguments____use_dtrace_names_and_copyinstr() {
    let bytes = section(&[
        entry("get__entry", "get", &ARGS),
        entry("put__entry", "put", &[("data", "bytes")]),
    ]);
    let out = dtrace(bin(), &groups(parse(&bytes), None, &Filter::default()));
    assert!(out.contains("#pragma D option quiet\n"), "{out}");
    assert!(
        out.contains(
            "app$target:::get-entry\n{\n\tprintf(\"app:get__entry id=%u path=%s delta=%d p=0x%x \
             opts=%s\\n\", arg0, copyinstr(arg1, arg2), arg3, arg4, copyinstr(arg5, arg6));\n}\n"
        ),
        "{out}"
    );
    assert!(out.contains("data=<%u bytes>\\n\", arg1);"), "{out}");
}

#[test]
fn dtrace____strsize____holds_the_longest_encoded_value_and_its_nul() {
    let bytes = section(&[entry("get__entry", "get", &ARGS)]);
    let out = dtrace(bin(), &groups(parse(&bytes), None, &Filter::default()));
    let strsize = format!(
        "#pragma D option strsize={}\n",
        anyprobe::encode::MAX_LEN + 1
    );
    assert!(out.contains(&strsize), "{out}");
}

#[test]
fn etw_guid____provider_name____matches_tracelogging() {
    // The GUID `Guid::from_name` documents for "MyProvider".
    assert_eq!(
        etw_guid("MyProvider"),
        "b3864c38-4273-58c5-545b-8b3608343471"
    );
}

#[test]
fn wprp____providers____are_enabled_by_guid_in_both_modes() {
    let bytes = section(&[
        entry("get__entry", "get", &[]),
        crate::test_support::Rec {
            provider: "other",
            ..entry("tick", "", &[])
        },
    ]);
    let out = wprp(bin(), &groups(parse(&bytes), None, &Filter::default()));
    let app = etw_guid("app");
    assert!(
        out.contains(&format!(
            "<EventProvider Id=\"anyprobe_app\" Name=\"{app}\" Level=\"5\"/>"
        )),
        "{out}"
    );
    assert!(
        out.contains("<EventProvider Id=\"anyprobe_other\""),
        "{out}"
    );
    assert!(out.contains("LoggingMode=\"File\""), "{out}");
    assert!(out.contains("LoggingMode=\"Memory\""), "{out}");
    assert_eq!(
        out.matches("<EventProviderId Value=\"anyprobe_app\"/>")
            .count(),
        2
    );
    assert!(
        out.contains(&format!("  Provider app: {app}\n    get__entry\n")),
        "{out}"
    );
}

#[test]
fn xml_text____double_dash____cannot_end_the_comment() {
    assert_eq!(xml_text("/a--b"), "/a- -b");
}

const OPTIONAL: [(&str, &str); 3] = [("name", "opt_str"), ("key", "opt_bytes"), ("label", "cstr")];

#[test]
fn bpftrace____optional_and_c_strings____are_read_by_length_or_to_their_nul() {
    let bytes = section(&[entry("look__entry", "look", &OPTIONAL)]);
    let out = bpftrace(bin(), &groups(parse(&bytes), None, &Filter::default()));
    assert!(
        out.contains(
            "printf(\"app:look__entry name=%s key=%r label=%s\\n\", \
             str(arg0, arg1), buf(arg2, arg3), str(arg4));"
        ),
        "{out}"
    );
}

#[test]
fn dtrace____optional_str____is_not_copied_from_a_null_pointer() {
    let bytes = section(&[entry("look__entry", "look", &OPTIONAL)]);
    let out = dtrace(bin(), &groups(parse(&bytes), None, &Filter::default()));
    assert!(
        out.contains(
            "printf(\"app:look__entry name=%s key=<%u bytes> label=%s\\n\", \
             arg0 ? copyinstr(arg0, arg1) : \"(none)\", arg3, copyinstr(arg4));"
        ),
        "{out}"
    );
}
