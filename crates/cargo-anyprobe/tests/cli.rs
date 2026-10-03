//! `cargo-anyprobe` run on this test binary, which defines probes of its
//! own: what `list` reports and the scripts it writes. On targets with no
//! probe backend there are no records to read.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::process::Command;

anyprobe::probes! {
    provider = "clitest";

    /// A request started.
    pub fn request__start(id: u64, path: &str);
}

#[anyprobe::probe(provider = "clitest", ret = native)]
fn lookup(id: u32, key: &str) -> u64 {
    u64::from(id) + key.len() as u64
}

struct Foo;
struct Bar;

impl Foo {
    #[anyprobe::probe(provider = "clitest")]
    fn new(x: u32) -> Self {
        let _ = x;
        Foo
    }
}

impl Bar {
    #[anyprobe::probe(provider = "clitest")]
    fn new() -> Self {
        Bar
    }
}

/// Uses every probe, so the linker keeps every site.
fn call_all() {
    request__start::fire(1, "/");
    assert_eq!(lookup(1, "ab"), 3);
    let _ = (Foo::new(1), Bar::new());
}

struct Run {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Runs the CLI with `args`, then the path of this test binary.
fn cli(args: &[&str]) -> Run {
    call_all();
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-anyprobe"))
        .args(args)
        .arg(&exe)
        .output()
        .unwrap();
    Run {
        status: out.status,
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

fn listed() -> bool {
    anyprobe::BACKEND != "noop"
}

#[test]
fn list____this_binary____shows_each_probe_with_its_arguments() {
    let run = cli(&["anyprobe", "list", "--provider", "clitest"]);
    assert!(run.status.success(), "{}", run.stderr);
    if !listed() {
        assert!(
            run.stdout.ends_with("0 probes in 0 providers\n"),
            "{}",
            run.stdout
        );
        return;
    }
    let out = &run.stdout;
    assert!(
        out.contains("clitest:request__start(id: u64, path: str)\n    probes! in cli, "),
        "{out}"
    );
    assert!(
        out.contains("clitest:lookup__entry(id: u32, key: str)\n    entry of fn lookup in cli, "),
        "{out}"
    );
    assert!(out.contains("clitest:lookup__return(ret: u64)\n"), "{out}");
    assert!(
        out.contains("clitest:new__entry\n    (x: u32)  entry of fn new in cli, "),
        "{out}"
    );
    assert!(out.contains("    ()  entry of fn new in cli, "), "{out}");
    assert!(out.ends_with("5 probes in 1 provider\n"), "{out}");
    // Every probe here is called, so each has a site where the format
    // records them.
    assert!(!out.contains("no site"), "{out}");
    assert!(
        run.stderr
            .contains("warning: clitest:new__entry has 2 different argument lists"),
        "{}",
        run.stderr
    );
}

#[test]
fn list_json____this_binary____gives_one_object_per_definition() {
    let run = cli(&[
        "list",
        "--json",
        "--provider",
        "clitest",
        "--probe",
        "new__*",
    ]);
    assert!(run.status.success(), "{}", run.stderr);
    let json: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    let records = json.as_array().unwrap();
    if !listed() {
        assert!(records.is_empty());
        return;
    }
    assert_eq!(records.len(), 4, "{json:#}");
    let entries = records.iter().filter(|r| r["name"] == "new__entry").count();
    assert_eq!(entries, 2);
    let site = if cfg!(windows) {
        serde_json::Value::Null
    } else {
        serde_json::Value::Bool(true)
    };
    assert!(records.iter().all(|r| r["has_site"] == site), "{json:#}");
}

#[test]
fn bpftrace____this_binary____has_a_clause_per_probe() {
    let run = cli(&["bpftrace", "--provider", "clitest"]);
    assert!(run.status.success(), "{}", run.stderr);
    if !listed() {
        return;
    }
    let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let out = &run.stdout;
    assert!(
        out.contains(&format!(
            "usdt:{}:clitest:lookup__entry\n{{\n\tprintf(\"clitest:lookup__entry id=%lu key=%s\\n\", \
             arg0, str(arg1, arg2));\n}}\n",
            exe.display()
        )),
        "{out}"
    );
    assert_eq!(out.matches("usdt:").count(), 5, "{out}");
}

#[test]
fn dtrace____this_binary____uses_dtrace_probe_names() {
    let run = cli(&["dtrace", "--provider", "clitest", "--probe", "lookup__*"]);
    assert!(run.status.success(), "{}", run.stderr);
    if !listed() {
        return;
    }
    assert!(
        run.stdout.contains("clitest$target:::lookup-entry\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("clitest$target:::lookup-return\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn not_a_binary____fails_with_the_reason() {
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-anyprobe"))
        .args(["list", file!()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.starts_with("error: "), "{stderr}");
}
