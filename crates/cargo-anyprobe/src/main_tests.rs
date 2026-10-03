//! Tests for command-line parsing.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

fn parse(args: &str) -> Result<Options, String> {
    let args: Vec<String> = args.split_whitespace().map(str::to_owned).collect();
    parse_args(&args).map_err(|e| e.to_string())
}

#[test]
fn parse_args____run_by_cargo____skips_the_subcommand_name() {
    let o = parse("anyprobe list target/debug/app").unwrap();
    assert_eq!(o.command, Command::List);
    assert_eq!(o.path, Some(PathBuf::from("target/debug/app")));
}

#[test]
fn parse_args____build_options____fill_the_build() {
    let o =
        parse("bpftrace --example=attr -p anyprobe --profile release --probe *__entry").unwrap();
    assert_eq!(o.command, Command::Bpftrace);
    assert_eq!(o.path, None);
    assert_eq!(o.build.example.as_deref(), Some("attr"));
    assert_eq!(o.build.package.as_deref(), Some("anyprobe"));
    assert_eq!(o.build.profile.as_deref(), Some("release"));
    assert_eq!(o.filter.probe.as_deref(), Some("*__entry"));
}

#[test]
fn parse_args____nothing____is_help() {
    assert_eq!(parse("").unwrap().command, Command::Help);
    assert_eq!(parse("anyprobe").unwrap().command, Command::Help);
    assert_eq!(parse("list --help").unwrap().command, Command::Help);
}

#[test]
fn parse_args____bad_input____says_what_is_wrong() {
    let cases = [
        ("list", "give the binary as a PATH"),
        ("list app --bin app", "give the binary as a PATH"),
        ("list --bin a --example b", "not both"),
        ("list app extra", "unexpected argument `extra`"),
        ("frobnicate app", "unknown command `frobnicate`"),
        ("list app --frob", "unknown option `--frob`"),
        ("list app --provider", "--provider needs a value"),
        ("dtrace app --json", "--json applies to `list` only"),
    ];
    for (args, expected) in cases {
        let err = parse(args).unwrap_err();
        assert!(err.contains(expected), "{args}: {err}");
        assert!(err.contains("cargo anyprobe --help"), "{args}: {err}");
    }
}
