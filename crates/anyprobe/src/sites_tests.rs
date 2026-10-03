//! Tests for decoding the FreeBSD site table and grouping it into DOF.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

fn record(version: u8, kind: u8, address: u64, strings: &[&str]) -> Vec<u8> {
    let mut rec = vec![0; 4];
    rec.push(version);
    rec.push(kind);
    rec.extend_from_slice(&[0, 0]);
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
    record(
        RECORD_VERSION,
        1,
        address,
        &["spike", "work-entry", "work", "uint64_t", "char *"],
    )
}

fn site_table_error(section: &[u8]) -> (usize, &'static str) {
    match parse(section) {
        Err(RegistrationError::SiteTable { offset, what }) => (offset, what),
        other => panic!("expected a site table error, got {other:?}"),
    }
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
fn parse____empty_function____keeps_the_arguments() {
    let section = record(RECORD_VERSION, 0, 0x1000, &["p", "n", "", "int64_t"]);
    let sites = parse(&section).unwrap();
    assert_eq!(sites[0].function, "");
    assert_eq!(sites[0].arguments, vec!["int64_t"]);
}

#[test]
fn parse____no_arguments____has_none() {
    let sites = parse(&record(RECORD_VERSION, 0, 0x1000, &["p", "n", "f"])).unwrap();
    assert!(sites[0].arguments.is_empty());
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
fn parse____zero_padding_between_records____is_skipped() {
    let mut section = vec![0; 16];
    section.extend(probe_site(0x1000));
    section.extend([0; 8]);
    section.extend(enabled_site(0x2000));
    assert_eq!(parse(&section).unwrap().len(), 2);
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
    assert_eq!(site_table_error(&section), (0, "truncated"));
}

#[test]
fn parse____missing_nul____is_unterminated() {
    let mut section = record(RECORD_VERSION, 0, 0x1000, &["spike", "x", "f"]);
    for b in &mut section[16..] {
        if *b == 0 {
            *b = b'z';
        }
    }
    assert_eq!(site_table_error(&section).1, "unterminated string");
}

#[test]
fn parse____unknown_kind____is_rejected() {
    let mut section = probe_site(0x1000);
    section.extend(record(RECORD_VERSION, 7, 0x1000, &["a", "b", "c"]));
    let offset = probe_site(0).len();
    assert_eq!(site_table_error(&section), (offset, "unknown site kind"));
}

#[test]
fn parse____empty_probe_name____is_rejected() {
    let section = record(RECORD_VERSION, 0, 0x1000, &["p", "", "f"]);
    assert_eq!(site_table_error(&section).1, "empty provider or probe name");
}

/// The DOF probes of `provider` named `name`.
fn dof_probes<'a>(dof: &'a dof::Section, provider: &str, name: &str) -> Vec<&'a dof::Probe> {
    dof.providers[provider]
        .probes
        .values()
        .filter(|p| p.name == name)
        .collect()
}

#[test]
fn to_dof____sites_of_one_probe____share_the_lowest_base() {
    let mut section = probe_site(0x1040);
    section.extend(probe_site(0x1000));
    section.extend(enabled_site(0x1010));
    section.extend(enabled_site(0x1080));
    let dof = to_dof(&parse(&section).unwrap());

    let probes = dof_probes(&dof, "spike", "work-entry");
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].address, 0x1000);
    assert_eq!(probes[0].offsets, vec![0x00, 0x40]);
    assert_eq!(probes[0].enabled_offsets, vec![0x10, 0x80]);
    assert_eq!(probes[0].arguments, vec!["uint64_t", "char *"]);
    assert_eq!(probes[0].function, "work");
}

#[test]
fn to_dof____duplicate_site____is_listed_once() {
    let mut section = probe_site(0x1000);
    section.extend(probe_site(0x1000));
    let dof = to_dof(&parse(&section).unwrap());
    assert_eq!(dof_probes(&dof, "spike", "work-entry")[0].offsets, vec![0]);
}

#[test]
fn to_dof____same_name_different_arguments____are_separate_probes() {
    let mut section = record(
        RECORD_VERSION,
        0,
        0x1000,
        &["p", "new-entry", "new", "uint64_t"],
    );
    section.extend(record(
        RECORD_VERSION,
        1,
        0x0ff0,
        &["p", "new-entry", "new", "uint64_t"],
    ));
    section.extend(record(
        RECORD_VERSION,
        0,
        0x2000,
        &["p", "new-entry", "new"],
    ));
    let dof = to_dof(&parse(&section).unwrap());

    let mut probes = dof_probes(&dof, "p", "new-entry");
    probes.sort_by_key(|p| p.address);
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0].arguments, vec!["uint64_t"]);
    assert_eq!(probes[0].offsets, vec![0x10]);
    assert_eq!(probes[0].enabled_offsets, vec![0]);
    assert!(probes[1].arguments.is_empty());
    assert_eq!(probes[1].offsets, vec![0]);
}

#[test]
fn to_dof____same_name_two_functions____are_separate_probes() {
    let mut section = record(RECORD_VERSION, 0, 0x1000, &["p", "n", "f"]);
    section.extend(record(RECORD_VERSION, 0, 0x2000, &["p", "n", "g"]));
    let dof = to_dof(&parse(&section).unwrap());
    assert_eq!(dof_probes(&dof, "p", "n").len(), 2);
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
    assert_eq!(dof.providers["spike"].probes.len(), 2);
    assert_eq!(dof_probes(&dof, "spike", "work-return")[0].address, 0x2000);
}

#[test]
fn to_dof____serialized____starts_with_dof_magic() {
    let dof = to_dof(&parse(&probe_site(0x1000)).unwrap());
    let bytes = dof::serialize_section(&dof);
    assert_eq!(&bytes[..4], b"\x7fDOF");
}
