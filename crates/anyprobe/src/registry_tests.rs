//! Tests for registry record encoding and parsing.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

/// A record for `body`, built as `register!` builds one.
fn rec(body: &str) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&u16::try_from(body.len()).unwrap().to_le_bytes());
    out.extend_from_slice(body.as_bytes());
    out
}

/// A record body: each field followed by a NUL.
fn body(fields: &[&str]) -> String {
    fields.iter().map(|f| format!("{f}\0")).collect()
}

fn entry() -> String {
    body(&[
        "1",
        "app",
        "get__entry",
        "entry",
        "get",
        "app::net::__anyprobe::__anyprobe_entry",
        "src/net.rs",
        "12",
        "3",
        "id",
        "u64",
        "path",
        "str",
        "opts",
        "debug",
    ])
}

/// The fields of a `probes!` record with no arguments.
const PROBES: [&str; 9] = [
    "1",
    "app",
    "tick",
    "probes",
    "",
    "app::tick",
    "src/lib.rs",
    "7",
    "0",
];

fn probes() -> String {
    body(&PROBES)
}

/// `PROBES` with field `i` replaced by `with`, and `extra` fields after.
fn probes_with(i: usize, with: &str, extra: &[&str]) -> String {
    let mut fields = PROBES.to_vec();
    fields[i] = with;
    fields.extend_from_slice(extra);
    body(&fields)
}

fn all(section: &[u8]) -> Vec<Result<ProbeInfo<'_>, RegistryError>> {
    parse(section).collect()
}

#[test]
fn record____const_fn____matches_the_layout_the_parser_reads() {
    const BODY: &str = "1\0app\0x\0";
    const R: [u8; record_len(BODY)] = record(BODY);
    assert_eq!(R.to_vec(), rec(BODY));
}

#[test]
fn parse____entry_record____reads_every_field() {
    let bytes = rec(&entry());
    let probes = all(&bytes);
    assert_eq!(probes.len(), 1);
    let p = probes[0].as_ref().unwrap();
    assert_eq!(p.provider, "app");
    assert_eq!(p.name, "get__entry");
    assert_eq!(p.dtrace_name(), "get-entry");
    assert_eq!(p.origin, Origin::Entry);
    assert_eq!(p.function, Some("get"));
    assert_eq!(p.module_path, "app::net");
    assert_eq!(p.file, "src/net.rs");
    assert_eq!(p.line, 12);
    let args: Vec<_> = p.args.iter().map(|a| (a.name, a.ty)).collect();
    assert_eq!(
        args,
        [
            ("id", ArgType::U64),
            ("path", ArgType::Str),
            ("opts", ArgType::Debug)
        ]
    );
    let indices: Vec<_> = p.arg_indices().map(|(i, a)| (i, a.name)).collect();
    assert_eq!(indices, [(0, "id"), (1, "path"), (3, "opts")]);
}

#[test]
fn parse____probes_record____has_no_function_and_strips_the_probe_module() {
    let bytes = rec(&probes());
    let p = parse(&bytes).next().unwrap().unwrap();
    assert_eq!(p.origin, Origin::Probes);
    assert_eq!(p.function, None);
    assert_eq!(p.module_path, "app");
    assert!(p.args.is_empty());
}

#[test]
fn parse____zero_padding_between_records____is_skipped() {
    let mut bytes = vec![0, 0, 0];
    bytes.extend(rec(&entry()));
    bytes.extend([0; 5]);
    bytes.extend(rec(&probes()));
    bytes.extend([0; 2]);
    let names: Vec<_> = all(&bytes).into_iter().map(|p| p.unwrap().name).collect();
    assert_eq!(names, ["get__entry", "tick"]);
}

#[test]
fn parse____empty_or_all_zero____yields_nothing() {
    assert!(all(&[]).is_empty());
    assert!(all(&[0; 16]).is_empty());
}

#[test]
fn parse____truncated_record____errors_once_then_ends() {
    let bytes = rec(&entry());
    for cut in [2, 5, bytes.len() - 1] {
        let probes = all(&bytes[..cut]);
        assert_eq!(
            probes,
            [Err(RegistryError::Truncated { offset: 0 })],
            "cut at {cut}"
        );
    }
}

#[test]
fn parse____garbage____is_not_a_record() {
    let mut bytes = rec(&probes());
    let second = bytes.len();
    bytes.extend(b"XXXX");
    bytes.extend(rec(&probes()));
    let probes = all(&bytes);
    assert_eq!(probes.len(), 2);
    assert!(probes[0].is_ok());
    assert_eq!(probes[1], Err(RegistryError::NotARecord { offset: second }));
}

#[test]
fn parse____other_version____is_unsupported() {
    let bytes = rec("2\0app\0x\0");
    assert_eq!(
        all(&bytes),
        [Err(RegistryError::UnsupportedVersion {
            offset: 0,
            version: "2".to_owned()
        })]
    );
}

#[test]
fn parse____malformed_bodies____say_what_is_wrong() {
    let no_final_nul = {
        let mut b = probes();
        b.pop();
        b
    };
    let cases = [
        (body(&PROBES[..7]), "too few fields"),
        (no_final_nul, "the last field has no NUL"),
        (probes_with(3, "nowhere", &[]), "unknown origin"),
        (probes_with(7, "one", &[]), "line is not a number"),
        (probes_with(8, "-1", &[]), "argument count is not a number"),
        (probes_with(8, "1", &["a", "u128"]), "unknown argument type"),
        (probes_with(8, "1", &["a"]), "too few fields"),
        (probes_with(8, "0", &["extra"]), "too many fields"),
    ];
    for (body, what) in cases {
        let bytes = rec(&body);
        assert_eq!(
            all(&bytes),
            [Err(RegistryError::Malformed { offset: 0, what })],
            "{body:?}"
        );
    }
}

#[test]
fn parse____names_the_macros_do_not_write____are_malformed() {
    let provider = "provider is not a name the macros write";
    let probe = "probe name is not a name the macros write";
    let function = "function is not a Rust identifier";
    let module = "module path is not a Rust path";
    let file = "file holds a control character";
    let arg = "argument name is not a Rust identifier";
    let cases = [
        (probes_with(1, "", &[]), provider),
        (probes_with(1, "app\nBEGIN", &[]), provider),
        (probes_with(1, "a:b", &[]), provider),
        (probes_with(1, "café", &[]), provider),
        (probes_with(1, "2app", &[]), provider),
        (probes_with(2, "", &[]), probe),
        (probes_with(2, "tick { system(\"id\") }", &[]), probe),
        (probes_with(2, "t*", &[]), probe),
        (probes_with(4, "new\"); system(\"id", &[]), function),
        (probes_with(4, "a b", &[]), function),
        (probes_with(5, "app::\u{1b}[2J", &[]), module),
        (probes_with(5, "app::", &[]), module),
        (probes_with(5, "", &[]), module),
        (probes_with(6, "src/\u{1b}[2Jlib.rs", &[]), file),
        (probes_with(6, "src/a\nb.rs", &[]), file),
        (
            probes_with(8, "1", &["id=%s\\n\", arg0); system(\"id", "u64"]),
            arg,
        ),
        (probes_with(8, "1", &["", "u64"]), arg),
        (probes_with(8, "1", &["a%n", "u64"]), arg),
        (probes_with(8, "1", &["1a", "u64"]), arg),
    ];
    for (body, what) in cases {
        let bytes = rec(&body);
        assert_eq!(
            all(&bytes),
            [Err(RegistryError::Malformed { offset: 0, what })],
            "{body:?}"
        );
    }
}

#[test]
fn parse____names_the_macros_write____are_accepted() {
    let body = body(&[
        "1",
        "_app2_",
        "_9",
        "entry",
        "café",
        "app::r#type::__anyprobe::__anyprobe_entry",
        "src/my file (1).rs",
        "1",
        "2",
        "größe",
        "u64",
        "_x",
        "u64",
    ]);
    let bytes = rec(&body);
    let probe = parse(&bytes).next().unwrap().unwrap();
    assert_eq!(probe.provider, "_app2_");
    assert_eq!(probe.function, Some("café"));
    assert_eq!(probe.module_path, "app::r#type");
    assert_eq!(probe.file, "src/my file (1).rs");
    assert_eq!(probe.args[0].name, "größe");
}

#[test]
fn parse____invalid_utf8____is_malformed() {
    let mut bytes = rec("1\0app\0x\0");
    let last = bytes.len() - 2;
    bytes[last] = 0xff;
    assert_eq!(
        all(&bytes),
        [Err(RegistryError::Malformed {
            offset: 0,
            what: "not UTF-8"
        })]
    );
}

#[test]
fn arg_type____names____round_trip() {
    for (ty, name) in ARG_TYPES {
        assert_eq!(ty.as_str(), name);
        assert_eq!(ArgType::from_str(name), Some(ty));
    }
}

#[test]
fn arg_type____slots____two_for_pointer_and_length() {
    assert_eq!(ArgType::U8.slots(), 1);
    assert_eq!(ArgType::Ptr.slots(), 1);
    for ty in [
        ArgType::Str,
        ArgType::Bytes,
        ArgType::Json,
        ArgType::Debug,
        ArgType::Auto,
        ArgType::Object,
    ] {
        assert_eq!(ty.slots(), 2, "{ty:?}");
    }
}

#[test]
fn error____unsupported_version____names_both_versions() {
    let e = RegistryError::UnsupportedVersion {
        offset: 8,
        version: "2".to_owned(),
    };
    assert_eq!(
        e.to_string(),
        "registry record at byte 8 has format version 2; this anyprobe reads version 1"
    );
}
