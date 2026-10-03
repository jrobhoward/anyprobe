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
