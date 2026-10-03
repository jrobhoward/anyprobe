//! Errors.

/// A registry record that could not be read. See [`registry`](crate::registry).
///
/// Records are written by the `probes!` and `#[probe]` macros of the same
/// anyprobe version that reads them through [`list`](crate::list), so these
/// errors come from reading bytes from elsewhere with
/// [`registry::parse`](crate::registry::parse): a section from a binary built
/// with another anyprobe version, or bytes that are not a registry section.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// The bytes end inside a record.
    #[error("registry record at byte {offset} is truncated")]
    Truncated {
        /// Where the record starts.
        offset: usize,
    },
    /// The bytes at `offset` do not start a record.
    #[error("no registry record at byte {offset}")]
    NotARecord {
        /// Where a record was expected.
        offset: usize,
    },
    /// The record was written by an anyprobe that uses another record format.
    #[error(
        "registry record at byte {offset} has format version {version}; this anyprobe reads version {}",
        crate::registry::VERSION
    )]
    UnsupportedVersion {
        /// Where the record starts.
        offset: usize,
        /// The version the record gives.
        version: String,
    },
    /// The record is not laid out as its version says.
    #[error("registry record at byte {offset} is malformed: {what}")]
    Malformed {
        /// Where the record starts.
        offset: usize,
        /// What is wrong with it.
        what: &'static str,
    },
}

/// Why the probes of this executable or library are not registered with the
/// tracer. See [`registration`](crate::registration).
///
/// Only FreeBSD registers probes at runtime, so only FreeBSD returns these.
/// The probes stay compiled in and their checks stay `false`; nothing else in
/// the program changes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistrationError {
    /// The tracer's device could not be opened. `ENOENT` means DTrace is not
    /// loaded (`kldload dtraceall`); `EACCES` that the process may not use it.
    #[error("cannot open {path}: {}", std::io::Error::from_raw_os_error(*code))]
    Open {
        /// The device.
        path: &'static str,
        /// The `errno` value.
        code: i32,
    },
    /// The kernel refused the probe description.
    #[error("the kernel refused the probes: {}", std::io::Error::from_raw_os_error(*code))]
    Refused {
        /// The `errno` value.
        code: i32,
    },
    /// The table of probe sites in the binary could not be read.
    #[error("probe site table at byte {offset} is malformed: {what}")]
    SiteTable {
        /// Where the bad record starts.
        offset: usize,
        /// What is wrong with it.
        what: &'static str,
    },
}
