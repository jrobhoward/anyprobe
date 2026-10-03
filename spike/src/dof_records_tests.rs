//! Tests for decoding probe records and grouping them into DOF.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

fn record(version: u8, kind: u8, address: u64, strings: &[&str]) -> Vec<u8> {
    let n_args = (strings.len() - 3) as u16;
    let mut rec = vec![0; 4];
    rec.push(version);
    rec.push(kind);
    rec.extend_from_slice(&n_args.to_ne_bytes());
    rec.extend_from_slice(&address.to_ne_bytes());
    for s in strings {
        rec.extend_from_slice(s.as_bytes());
        rec.push(0);
    }
    while !rec.len().is_multiple_of(8) {
        rec.push(0);
    }
    let len = rec.len() as u32;
    rec[..4].copy_from_slice(&len.to_ne_bytes());
    rec
}

fn probe_site(address: u64) -> Vec<u8> {
    record(
        RECORD_VERSION,
        0,
        address,
        &["spike", "work-entry", "work", "uint64_t", "char *"],
    )
}

fn enabled_site(address: u64) -> Vec<u8> {
    record(RECORD_VERSION, 1, address, &["spike", "work-entry", "work"])
}

#[test]
fn parse____empty_section____has_no_sites() {
    assert_eq!(parse(&[]).unwrap(), vec![]);
}

#[test]
fn parse____probe_record____decodes_every_field() {
    let sites = parse(&probe_site(0x1000)).unwrap();
    assert_eq!(
        sites,
        vec![Site {
            is_enabled: false,
            address: 0x1000,
            provider: "spike".into(),
            probe: "work-entry".into(),
            function: "work".into(),
            arguments: vec!["uint64_t".into(), "char *".into()],
        }]
    );
}

#[test]
fn parse____records_back_to_back____decodes_each() {
    let mut section = probe_site(0x1000);
    section.extend(enabled_site(0x0ff0));
    let sites = parse(&section).unwrap();
    assert_eq!(sites.len(), 2);
    assert!(sites[1].is_enabled);
    assert_eq!(sites[1].address, 0x0ff0);
}

#[test]
fn parse____other_version____is_skipped() {
    let mut section = record(RECORD_VERSION + 1, 0, 0x2000, &["a", "b", "c"]);
    section.extend(probe_site(0x1000));
    let sites = parse(&section).unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].address, 0x1000);
}

#[test]
fn parse____length_past_end____is_truncated() {
    let mut section = probe_site(0x1000);
    section.truncate(section.len() - 8);
    assert_eq!(parse(&section), Err(RecordError::Truncated { offset: 0 }));
}

#[test]
fn parse____missing_nul____is_unterminated() {
    let mut section = record(RECORD_VERSION, 0, 0x1000, &["spike", "x", "f"]);
    for b in &mut section[16..] {
        if *b == 0 {
            *b = b'z';
        }
    }
    assert!(matches!(
        parse(&section),
        Err(RecordError::UnterminatedString { .. })
    ));
}

#[test]
fn parse____unknown_kind____is_rejected() {
    let section = record(RECORD_VERSION, 7, 0x1000, &["a", "b", "c"]);
    assert_eq!(
        parse(&section),
        Err(RecordError::UnknownKind { offset: 0, kind: 7 })
    );
}

#[test]
fn to_dof____sites_of_one_probe____share_the_lowest_base() {
    let mut section = probe_site(0x1040);
    section.extend(probe_site(0x1000));
    section.extend(enabled_site(0x1010));
    section.extend(enabled_site(0x1080));
    let dof = to_dof(&parse(&section).unwrap());

    let probe = &dof.providers["spike"].probes["work-entry"];
    assert_eq!(probe.address, 0x1000);
    assert_eq!(probe.offsets, vec![0x00, 0x40]);
    assert_eq!(probe.enabled_offsets, vec![0x10, 0x80]);
}

#[test]
fn to_dof____is_enabled_site_first____takes_arguments_from_probe_site() {
    let mut section = enabled_site(0x1000);
    section.extend(probe_site(0x1100));
    let dof = to_dof(&parse(&section).unwrap());

    let probe = &dof.providers["spike"].probes["work-entry"];
    assert_eq!(probe.arguments, vec!["uint64_t", "char *"]);
    assert_eq!(probe.function, "work");
}

#[test]
fn to_dof____two_probes____are_separate_entries() {
    let mut section = probe_site(0x1000);
    section.extend(record(
        RECORD_VERSION,
        0,
        0x2000,
        &["spike", "work-return", "work", "uint64_t", "uint64_t"],
    ));
    let dof = to_dof(&parse(&section).unwrap());
    let probes = &dof.providers["spike"].probes;
    assert_eq!(probes.len(), 2);
    assert_eq!(probes["work-return"].address, 0x2000);
}

#[test]
fn to_dof____serialized____starts_with_dof_magic() {
    let dof = to_dof(&parse(&probe_site(0x1000)).unwrap());
    let bytes = dof::serialize_section(&dof);
    assert_eq!(&bytes[..4], b"\x7fDOF");
}
