//! Probe records emitted into the `set_anyprobe_probes` section, and their
//! conversion to DOF.
//!
//! Each probe site and each is-enabled site emits one record next to its
//! code. At registration the section is read back, grouped by provider and
//! probe, and turned into a DOF section the kernel's `fasttrap` provider
//! understands. Only FreeBSD uses this at runtime; the parsing is plain data
//! processing, so it is tested on every host.
//!
//! Record layout, native endian, 8-byte aligned:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | total length of the record, padding included |
//! | 4 | 1 | version, [`RECORD_VERSION`] |
//! | 5 | 1 | kind: 0 probe site, 1 is-enabled site |
//! | 6 | 2 | argument count |
//! | 8 | 8 | site address |
//! | 16 | .. | NUL-terminated provider, probe, function, then one C type per argument |

use std::collections::BTreeMap;

/// Version written by this crate's records. Records with another version are
/// skipped, so an old and a new copy of the crate in one binary do not
/// misread each other.
pub(crate) const RECORD_VERSION: u8 = 1;

const HEADER_LEN: usize = 16;

/// What a record failed on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RecordError {
    /// Fewer bytes remain than the record header or its length field claims.
    Truncated { offset: usize },
    /// A string field had no terminating NUL inside the record.
    UnterminatedString { offset: usize },
    /// A string field was not UTF-8.
    NotUtf8 { offset: usize },
    /// A record's kind byte was neither 0 nor 1.
    UnknownKind { offset: usize, kind: u8 },
}

/// One site, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Site {
    pub(crate) is_enabled: bool,
    pub(crate) address: u64,
    pub(crate) provider: String,
    pub(crate) probe: String,
    pub(crate) function: String,
    pub(crate) arguments: Vec<String>,
}

/// Decodes every record in `section`.
pub(crate) fn parse(section: &[u8]) -> Result<Vec<Site>, RecordError> {
    let mut sites = Vec::new();
    let mut offset = 0;
    while offset < section.len() {
        let rest = &section[offset..];
        let header = rest
            .get(..HEADER_LEN)
            .ok_or(RecordError::Truncated { offset })?;
        let len = u32::from_ne_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if len < HEADER_LEN || len > rest.len() {
            return Err(RecordError::Truncated { offset });
        }
        let record = &rest[..len];
        if record[4] == RECORD_VERSION {
            sites.push(parse_record(record, offset)?);
        }
        offset += len;
    }
    Ok(sites)
}

fn parse_record(record: &[u8], offset: usize) -> Result<Site, RecordError> {
    let is_enabled = match record[5] {
        0 => false,
        1 => true,
        kind => return Err(RecordError::UnknownKind { offset, kind }),
    };
    let n_args = u16::from_ne_bytes([record[6], record[7]]);
    let mut address = [0; 8];
    address.copy_from_slice(&record[8..16]);
    let address = u64::from_ne_bytes(address);

    let mut strings = Strings {
        data: &record[HEADER_LEN..],
        offset: offset + HEADER_LEN,
    };
    let provider = strings.next()?;
    let probe = strings.next()?;
    let function = strings.next()?;
    let arguments = (0..n_args)
        .map(|_| strings.next())
        .collect::<Result<_, _>>()?;
    Ok(Site {
        is_enabled,
        address,
        provider,
        probe,
        function,
        arguments,
    })
}

struct Strings<'a> {
    data: &'a [u8],
    offset: usize,
}

impl Strings<'_> {
    fn next(&mut self) -> Result<String, RecordError> {
        let offset = self.offset;
        let end = self
            .data
            .iter()
            .position(|&b| b == 0)
            .ok_or(RecordError::UnterminatedString { offset })?;
        let s = std::str::from_utf8(&self.data[..end])
            .map_err(|_| RecordError::NotUtf8 { offset })?
            .to_owned();
        self.data = &self.data[end + 1..];
        self.offset += end + 1;
        Ok(s)
    }
}

/// Groups sites into the DOF model: one DOF probe per provider and probe
/// name, based at the lowest site address, with every site an offset from it.
pub(crate) fn to_dof(sites: &[Site]) -> dof::Section {
    let mut providers: BTreeMap<String, dof::Provider> = BTreeMap::new();
    let mut by_probe: BTreeMap<(&str, &str), Vec<&Site>> = BTreeMap::new();
    for site in sites {
        by_probe
            .entry((&site.provider, &site.probe))
            .or_default()
            .push(site);
    }
    for ((provider, probe), sites) in by_probe {
        let base = sites.iter().map(|s| s.address).min().unwrap_or(0);
        let mut offsets = Vec::new();
        let mut enabled_offsets = Vec::new();
        for site in &sites {
            // Offsets are 32-bit in DOF. Every site of a probe is in the same
            // binary, so the distance from the lowest one fits.
            let off = (site.address - base) as u32;
            if site.is_enabled {
                enabled_offsets.push(off);
            } else {
                offsets.push(off);
            }
        }
        offsets.sort_unstable();
        enabled_offsets.sort_unstable();
        // Argument types and the function name come from the probe sites,
        // since is-enabled records carry no arguments.
        let described = sites.iter().find(|s| !s.is_enabled).unwrap_or(&sites[0]);
        providers
            .entry(provider.to_owned())
            .or_insert_with(|| dof::Provider {
                name: provider.to_owned(),
                probes: BTreeMap::new(),
            })
            .probes
            .insert(
                probe.to_owned(),
                dof::Probe {
                    name: probe.to_owned(),
                    function: described.function.clone(),
                    address: base,
                    offsets,
                    enabled_offsets,
                    arguments: described.arguments.clone(),
                },
            );
    }
    dof::Section {
        providers,
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "dof_records_tests.rs"]
mod dof_records_tests;
