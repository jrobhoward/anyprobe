//! Errors.

use std::path::PathBuf;

/// Why a command failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The command line could not be understood.
    #[error("{0}\n\nRun `cargo anyprobe --help` for usage.")]
    Usage(String),
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
    /// The file is not an executable or library this tool reads.
    #[error("{path}: {what}")]
    Format {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        what: String,
    },
    /// A registry record could not be read.
    #[error("{path}: {source}")]
    Registry {
        /// The file.
        path: PathBuf,
        /// What is wrong with the record.
        source: anyprobe::RegistryError,
    },
    /// A script cannot be written for the file.
    #[error("{path}: {what}")]
    Script {
        /// The file.
        path: PathBuf,
        /// Why not.
        what: String,
    },
    /// `cargo build` could not be run, failed, or built nothing matching.
    #[error("{0}")]
    Build(String),
}
