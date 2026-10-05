//! `cargo anyprobe`: lists the anyprobe probes in a binary, and writes
//! bpftrace, DTrace and WPR scripts for them.
//!
//! It reads the registry that `anyprobe`'s macros put in every binary that
//! defines probes, from the file on disk, so it never runs the program and
//! reads a binary built for any target.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

mod binary;
mod cargo;
mod error;
mod report;
mod script;
#[cfg(test)]
mod test_support;

use error::Error;

const HELP: &str = "\
cargo anyprobe: the anyprobe probes in a binary, and tracer scripts for them

Usage: cargo anyprobe <COMMAND> [OPTIONS] [PATH]

Commands:
  list      Every probe: arguments and their types, where it is defined
  bpftrace  A bpftrace script printing every probe and its arguments
  dtrace    The same as a D script, for DTrace
  wprp      A WPR profile recording every provider, for ETW on Windows

The binary is PATH, or the one cargo builds for --bin or --example.

Options:
      --bin <NAME>          Build and read this binary target
      --example <NAME>      Build and read this example
  -p, --package <SPEC>      Package of the target
      --profile <NAME>      Build profile
  -r, --release             Release profile
      --target <TRIPLE>     Build for this target; picks the slice of a
                            universal macOS binary
  -F, --features <LIST>     Features to enable
      --all-features        Enable every feature
      --no-default-features Do not enable the default features
      --manifest-path <PATH>
      --provider <NAME>     Only this provider
      --probe <PATTERN>     Only probe names matching PATTERN (* and ?)
      --json                list: JSON, one object per definition
  -h, --help                Print this help
  -V, --version             Print the version
";

/// What to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    List,
    Bpftrace,
    Dtrace,
    Wprp,
    Help,
    Version,
}

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    command: Command,
    path: Option<PathBuf>,
    build: cargo::Build,
    filter: report::Filter,
    json: bool,
}

/// The arguments as UTF-8, or a usage error naming the first that is not.
/// `std::env::args` would panic on it instead.
fn utf8_args(args: impl IntoIterator<Item = OsString>) -> Result<Vec<String>, Error> {
    args.into_iter()
        .map(|arg| {
            arg.into_string().map_err(|arg| {
                Error::Usage(format!(
                    "argument `{}` is not valid UTF-8",
                    arg.to_string_lossy()
                ))
            })
        })
        .collect()
}

/// Parses the arguments after the program name. Run by cargo, they start
/// with `anyprobe`.
fn parse_args(args: &[String]) -> Result<Options, Error> {
    let usage = |msg: String| Error::Usage(msg);
    let mut args = args.iter().map(String::as_str).peekable();
    if args.peek() == Some(&"anyprobe") {
        args.next();
    }
    let mut command = None;
    let mut path = None;
    let mut build = cargo::Build::default();
    let mut filter = report::Filter::default();
    let mut json = false;
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        let mut value = |name: &str| -> Result<String, Error> {
            inline
                .map(str::to_owned)
                .or_else(|| args.next().map(str::to_owned))
                .ok_or_else(|| usage(format!("{name} needs a value")))
        };
        match flag {
            "-h" | "--help" => command = Some(Command::Help),
            "-V" | "--version" => command = Some(Command::Version),
            "--bin" => build.bin = Some(value(flag)?),
            "--example" => build.example = Some(value(flag)?),
            "-p" | "--package" => build.package = Some(value(flag)?),
            "--profile" => build.profile = Some(value(flag)?),
            "-r" | "--release" => build.release = true,
            "--target" => build.target = Some(value(flag)?),
            "-F" | "--features" => build.features.push(value(flag)?),
            "--all-features" => build.all_features = true,
            "--no-default-features" => build.no_default_features = true,
            "--manifest-path" => build.manifest_path = Some(PathBuf::from(value(flag)?)),
            "--provider" => filter.provider = Some(value(flag)?),
            "--probe" => filter.probe = Some(value(flag)?),
            "--json" => json = true,
            f if f.starts_with('-') && f.len() > 1 => {
                return Err(usage(format!("unknown option `{f}`")));
            }
            word if command.is_none() => {
                command = Some(match word {
                    "list" => Command::List,
                    "bpftrace" => Command::Bpftrace,
                    "dtrace" => Command::Dtrace,
                    "wprp" => Command::Wprp,
                    "help" => Command::Help,
                    _ => return Err(usage(format!("unknown command `{word}`"))),
                });
            }
            word if path.is_none() => path = Some(PathBuf::from(word)),
            word => return Err(usage(format!("unexpected argument `{word}`"))),
        }
    }
    let command = command.unwrap_or(Command::Help);
    if matches!(command, Command::Help | Command::Version) {
        return Ok(Options {
            command,
            path,
            build,
            filter,
            json,
        });
    }
    if build.bin.is_some() && build.example.is_some() {
        return Err(usage("give --bin or --example, not both".to_owned()));
    }
    if path.is_some() == build.is_requested() {
        return Err(usage(
            "give the binary as a PATH, or as --bin NAME or --example NAME".to_owned(),
        ));
    }
    if json && command != Command::List {
        return Err(usage("--json applies to `list` only".to_owned()));
    }
    Ok(Options {
        command,
        path,
        build,
        filter,
        json,
    })
}

/// Runs the command; returns what to print on stdout.
fn run(options: &Options) -> Result<String, Error> {
    match options.command {
        Command::Help => return Ok(HELP.to_owned()),
        Command::Version => {
            return Ok(format!("cargo-anyprobe {}\n", env!("CARGO_PKG_VERSION")));
        }
        _ => {}
    }
    let path = match &options.path {
        Some(path) => path.clone(),
        None => cargo::build(&options.build)?,
    };
    let path = std::fs::canonicalize(&path).map_err(|source| Error::Read {
        path: path.clone(),
        source,
    })?;
    let arch = options
        .build
        .target
        .as_deref()
        .and_then(|t| t.split('-').next());
    let binary = binary::read(&path, arch)?;
    let records = anyprobe::registry::parse(&binary.registry)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::Registry {
            path: path.clone(),
            source,
        })?;
    if records.is_empty() {
        eprintln!(
            "warning: {} has no anyprobe probes ({} file)",
            path.display(),
            binary.format
        );
    }
    let groups = report::groups(records, binary.sites.as_ref(), &options.filter);
    for warning in report::warnings(&groups) {
        eprintln!("warning: {warning}");
    }
    Ok(match options.command {
        Command::List if options.json => report::list_json(&groups),
        Command::List => report::list_text(&groups),
        Command::Bpftrace => script::bpftrace(&path, &groups),
        Command::Dtrace => script::dtrace(&path, &groups),
        Command::Wprp => script::wprp(&path, &groups),
        Command::Help | Command::Version => String::new(),
    })
}

fn main() -> ExitCode {
    let result = utf8_args(std::env::args_os().skip(1))
        .and_then(|args| parse_args(&args))
        .and_then(|options| run(&options));
    match result {
        Ok(out) => {
            // A closed pipe (`| head`) is not an error worth reporting.
            let _ = std::io::stdout().write_all(out.as_bytes());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod main_tests;
