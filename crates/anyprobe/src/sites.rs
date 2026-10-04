//! The FreeBSD probe site table, and its conversion to DOF.
//!
//! Each probe site and each is-enabled site emits one record into the
//! `anyprobe_sites` section, next to its code. At startup the backend reads
//! the section back, groups the records by provider and probe, and turns
//! them into the DOF the kernel's `fasttrap` provider reads. Only FreeBSD
//! uses this at runtime; the parsing is plain data processing, so it is
//! tested on every host.
//!
//! Records follow each other, 8-byte aligned, native endian:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | total length of the record, padding included |
//! | 4 | 1 | version, [`RECORD_VERSION`] |
//! | 5 | 1 | kind: 0 probe site, 1 is-enabled site |
//! | 6 | 2 | zero |
//! | 8 | 8 | site address |
//! | 16 | .. | NUL-terminated provider, probe and function, then one C type per argument, then zero padding |
//!
//! The function may be empty. A C type never is, so the argument list ends
//! at the first empty string or at the end of the record.
//!
//! Unlike the registry, a record holds a pointer: the site address, which the
//! dynamic linker relocates in a position-independent executable.

use std::collections::BTreeMap;

use crate::error::RegistrationError;

/// Version written by this crate's records. Records with another version are
/// skipped, so two copies of the crate in one binary do not misread each
/// other.
pub(crate) const RECORD_VERSION: u8 = 1;

const HEADER_LEN: usize = 16;

/// One site, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Site {
    /// Where the site's record starts in the table, for errors.
    pub(crate) offset: usize,
    pub(crate) is_enabled: bool,
    pub(crate) address: u64,
    pub(crate) provider: String,
    pub(crate) probe: String,
    pub(crate) function: String,
    pub(crate) arguments: Vec<String>,
}

/// Decodes every record in `section`. Zero bytes between records are skipped
/// 8 at a time.
pub(crate) fn parse(section: &[u8]) -> Result<Vec<Site>, RegistrationError> {
    let mut sites = Vec::new();
    let mut offset = 0;
    while offset < section.len() {
        let rest = &section[offset..];
        let truncated = RegistrationError::SiteTable {
            offset,
            what: "truncated",
        };
        if rest.iter().take(8).all(|&b| b == 0) {
            offset += rest.len().min(8);
            continue;
        }
        let header = rest.get(..HEADER_LEN).ok_or(truncated.clone())?;
        let len = u32::from_ne_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if len < HEADER_LEN || len > rest.len() {
            return Err(truncated);
        }
        let record = &rest[..len];
        if record[4] == RECORD_VERSION {
            sites.push(parse_record(record, offset)?);
        }
        offset += len;
    }
    Ok(sites)
}

fn parse_record(record: &[u8], offset: usize) -> Result<Site, RegistrationError> {
    let malformed = |what| RegistrationError::SiteTable { offset, what };
    let is_enabled = match record[5] {
        0 => false,
        1 => true,
        _ => return Err(malformed("unknown site kind")),
    };
    let mut address = [0; 8];
    address.copy_from_slice(&record[8..16]);
    let address = u64::from_ne_bytes(address);

    // A well-formed record ends in a NUL, either its last string's or
    // padding.
    if record.last() != Some(&0) {
        return Err(malformed("unterminated string"));
    }
    let mut strings = record[HEADER_LEN..].split(|&b| b == 0);
    let mut next = || -> Result<String, RegistrationError> {
        let s = strings.next().ok_or_else(|| malformed("missing string"))?;
        std::str::from_utf8(s)
            .map(str::to_owned)
            .map_err(|_| malformed("string is not UTF-8"))
    };
    let provider = next()?;
    let probe = next()?;
    let function = next()?;
    if provider.is_empty() || probe.is_empty() {
        return Err(malformed("empty provider or probe name"));
    }
    let mut arguments = Vec::new();
    for ty in strings.by_ref() {
        if ty.is_empty() {
            break;
        }
        let ty = std::str::from_utf8(ty).map_err(|_| malformed("argument type is not UTF-8"))?;
        arguments.push(ty.to_owned());
    }
    Ok(Site {
        offset,
        is_enabled,
        address,
        provider,
        probe,
        function,
        arguments,
    })
}

/// Groups sites into the DOF model: one DOF probe per provider, probe name,
/// function and argument list, based at the lowest site address, with every
/// site an offset from it.
///
/// Two functions can define the same probe name, with different arguments
/// (`Foo::new` and `Bar::new` both define `new__entry`). Each gets its own
/// DOF probe, as ld64 gives each function its own on macOS, so each keeps its
/// argument types; a tracer matching the name matches all of them. Is-enabled
/// records carry the argument types too, so they group with their probe's
/// sites.
///
/// DOF gives each site as a 32-bit offset from the probe's base. Sites of
/// one probe more than 4 GiB apart are a malformed table rather than offsets
/// cut short, which would put a breakpoint at the wrong address.
pub(crate) fn to_dof(sites: &[Site]) -> Result<dof::Section, RegistrationError> {
    type Key<'a> = (&'a str, &'a str, &'a str, &'a [String]);
    let mut by_probe: BTreeMap<Key<'_>, Vec<&Site>> = BTreeMap::new();
    for site in sites {
        by_probe
            .entry((&site.provider, &site.probe, &site.function, &site.arguments))
            .or_default()
            .push(site);
    }
    let mut providers: BTreeMap<String, dof::Provider> = BTreeMap::new();
    for ((provider, probe, function, arguments), sites) in by_probe {
        let base = sites.iter().map(|s| s.address).min().unwrap_or(0);
        let mut offsets = Vec::new();
        let mut enabled_offsets = Vec::new();
        for site in &sites {
            let off =
                u32::try_from(site.address - base).map_err(|_| RegistrationError::SiteTable {
                    offset: site.offset,
                    what: "site is more than 4 GiB from its probe's first site",
                })?;
            if site.is_enabled {
                enabled_offsets.push(off);
            } else {
                offsets.push(off);
            }
        }
        offsets.sort_unstable();
        offsets.dedup();
        enabled_offsets.sort_unstable();
        enabled_offsets.dedup();
        // `dof` writes each value as a probe and ignores the key, which only
        // has to be unique within the provider.
        let key = format!("{probe}\0{function}\0{}", arguments.join("\0"));
        providers
            .entry(provider.to_owned())
            .or_insert_with(|| dof::Provider {
                name: provider.to_owned(),
                probes: BTreeMap::new(),
            })
            .probes
            .insert(
                key,
                dof::Probe {
                    name: probe.to_owned(),
                    function: function.to_owned(),
                    address: base,
                    offsets,
                    enabled_offsets,
                    arguments: arguments.to_vec(),
                },
            );
    }
    Ok(dof::Section {
        providers,
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "sites_tests.rs"]
mod sites_tests;
