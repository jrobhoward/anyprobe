#![allow(non_snake_case)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use super::*;

#[test]
fn raw_range____within_the_file____returns_the_bytes() {
    assert_eq!(raw_range(&[1, 2, 3, 4], 1, 2), Some(&[2, 3][..]));
}

#[test]
fn raw_range____empty_at_the_end____returns_nothing_to_read() {
    assert_eq!(raw_range(&[1, 2], 2, 0), Some(&[][..]));
}

#[test]
fn raw_range____past_the_end____is_none() {
    assert_eq!(raw_range(&[1, 2, 3], 2, 2), None);
    assert_eq!(raw_range(&[1, 2, 3], 4, 0), None);
}

#[test]
fn raw_range____start_plus_length_overflows____is_none() {
    assert_eq!(raw_range(&[1, 2, 3], u32::MAX, u32::MAX), None);
}

#[test]
fn read_dof____section_at_an_odd_address____lists_its_probes() {
    let probe = dof::Probe {
        name: "lookup-entry".to_owned(),
        function: "lookup".to_owned(),
        address: 0x1000,
        offsets: vec![0],
        enabled_offsets: vec![],
        arguments: vec!["uint64_t".to_owned()],
    };
    let provider = dof::Provider {
        name: "attr".to_owned(),
        probes: [(probe.name.clone(), probe)].into(),
    };
    let section = dof::Section {
        providers: [(provider.name.clone(), provider)].into(),
        ..Default::default()
    };
    let dof = dof::serialize_section(&section);
    // ld64 can place a DOF section at any address; put this one at an odd
    // one, which `dof` cannot read in place.
    let mut file = vec![0; dof.len() + 8];
    let start = 1 + file.as_ptr().addr().wrapping_neg() % 8;
    file[start..start + dof.len()].copy_from_slice(&dof);
    let mut sites = HashSet::new();
    read_dof(&file[start..start + dof.len()], &mut sites).unwrap();
    assert_eq!(sites, HashSet::from([site_key("attr", "lookup-entry")]));
}
