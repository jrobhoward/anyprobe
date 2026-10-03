//! Registry sections for tests, built as anyprobe's macros build them.

#![allow(clippy::unwrap_used)]

/// One record: provider, name, origin, function, line and `(name, type)`
/// arguments. The module path is the one `#[probe]` records for `app::net`.
pub struct Rec<'a> {
    pub provider: &'a str,
    pub name: &'a str,
    pub origin: &'a str,
    pub function: &'a str,
    pub line: u32,
    pub args: &'a [(&'a str, &'a str)],
}

/// A registry section holding `records`, with zero padding between them.
pub fn section(records: &[Rec<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        let module = if r.origin == "probes" {
            format!("app::net::{}", r.name)
        } else {
            "app::net::__anyprobe::__anyprobe_entry".to_owned()
        };
        let mut fields = vec![
            "1".to_owned(),
            r.provider.to_owned(),
            r.name.to_owned(),
            r.origin.to_owned(),
            r.function.to_owned(),
            module,
            "src/net.rs".to_owned(),
            r.line.to_string(),
            r.args.len().to_string(),
        ];
        for (name, ty) in r.args {
            fields.push((*name).to_owned());
            fields.push((*ty).to_owned());
        }
        let body: String = fields.iter().map(|f| format!("{f}\0")).collect();
        out.extend_from_slice(b"APRB");
        out.extend_from_slice(&u16::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(body.as_bytes());
        out.extend_from_slice(&[0, 0, 0]);
    }
    out
}

/// A record of `#[probe]`'s entry probe.
pub fn entry<'a>(name: &'a str, function: &'a str, args: &'a [(&'a str, &'a str)]) -> Rec<'a> {
    Rec {
        provider: "app",
        name,
        origin: "entry",
        function,
        line: 10,
        args,
    }
}

/// Every probe in `section`.
pub fn parse(section: &[u8]) -> Vec<anyprobe::registry::ProbeInfo<'_>> {
    anyprobe::registry::parse(section)
        .collect::<Result<_, _>>()
        .unwrap()
}
