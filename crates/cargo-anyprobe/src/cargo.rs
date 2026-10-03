//! Building the binary to read with `cargo build`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::error::Error;

/// The `cargo build` options `cargo anyprobe` passes through.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Build {
    /// `--bin NAME`.
    pub bin: Option<String>,
    /// `--example NAME`.
    pub example: Option<String>,
    /// `-p` / `--package`.
    pub package: Option<String>,
    /// `--profile`.
    pub profile: Option<String>,
    /// `--release`.
    pub release: bool,
    /// `--target`.
    pub target: Option<String>,
    /// `--features`, each occurrence.
    pub features: Vec<String>,
    /// `--all-features`.
    pub all_features: bool,
    /// `--no-default-features`.
    pub no_default_features: bool,
    /// `--manifest-path`.
    pub manifest_path: Option<PathBuf>,
}

impl Build {
    /// Whether a target to build was named.
    pub fn is_requested(&self) -> bool {
        self.bin.is_some() || self.example.is_some()
    }

    /// The arguments after `cargo`.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            "build".to_owned(),
            "--message-format=json-render-diagnostics".to_owned(),
        ];
        let mut opt = |flag: &str, value: &Option<String>| {
            if let Some(v) = value {
                args.push(flag.to_owned());
                args.push(v.clone());
            }
        };
        opt("--bin", &self.bin);
        opt("--example", &self.example);
        opt("--package", &self.package);
        opt("--profile", &self.profile);
        opt("--target", &self.target);
        opt(
            "--manifest-path",
            &self.manifest_path.as_ref().map(|p| p.display().to_string()),
        );
        for f in &self.features {
            args.push("--features".to_owned());
            args.push(f.clone());
        }
        if self.release {
            args.push("--release".to_owned());
        }
        if self.all_features {
            args.push("--all-features".to_owned());
        }
        if self.no_default_features {
            args.push("--no-default-features".to_owned());
        }
        args
    }
}

/// Runs `cargo build` and returns the path of the executable it built.
pub fn build(options: &Build) -> Result<PathBuf, Error> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(&cargo)
        .args(options.args())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| Error::Build(format!("cannot run cargo: {e}")))?;
    if !output.status.success() {
        return Err(Error::Build(format!(
            "cargo build failed ({})",
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (kind, name) = match (&options.bin, &options.example) {
        (Some(bin), _) => ("bin", bin.as_str()),
        (None, Some(example)) => ("example", example.as_str()),
        (None, None) => return Err(Error::Build("no --bin or --example given".to_owned())),
    };
    executable(&stdout, kind, name).ok_or_else(|| {
        Error::Build(format!(
            "cargo build reported no executable for {kind} `{name}`"
        ))
    })
}

/// The executable of the `kind` target `name` in cargo's JSON messages.
pub fn executable(messages: &str, kind: &str, name: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| m["target"]["name"] == name)
        .filter(|m| {
            m["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|k| k == kind))
        })
        .filter_map(|m| m["executable"].as_str().map(PathBuf::from))
        .next_back()
}

#[cfg(test)]
#[path = "cargo_tests.rs"]
mod cargo_tests;
