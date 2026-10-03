//! Grouping registry records by probe, and the `list` command's output.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use anyprobe::registry::{ArgInfo, Origin, ProbeInfo};

use crate::binary::site_key;

/// Every record of one probe name. Several functions can define the same
/// probe name (`Foo::new` and `Bar::new` both have `new__entry`), with the
/// same or different arguments.
#[derive(Debug)]
pub struct Group<'a> {
    /// The provider.
    pub provider: &'a str,
    /// The probe name.
    pub name: &'a str,
    /// The records, in the order the section holds them.
    pub records: Vec<ProbeInfo<'a>>,
    /// Whether the tracer metadata has a site for the probe; `None` where
    /// the binary has no such metadata (Windows).
    pub has_site: Option<bool>,
}

impl<'a> Group<'a> {
    /// The probe's arguments, if every record agrees on them.
    pub fn args(&self) -> Option<&[ArgInfo<'a>]> {
        let first = &self.records.first()?.args;
        self.records
            .iter()
            .all(|r| same_layout(&r.args, first))
            .then_some(first.as_slice())
    }

    /// How many different argument lists the records have.
    pub fn layouts(&self) -> usize {
        let mut seen: Vec<&[ArgInfo<'a>]> = Vec::new();
        for record in &self.records {
            if !seen.iter().any(|s| same_layout(s, &record.args)) {
                seen.push(&record.args);
            }
        }
        seen.len()
    }

    /// `provider:name`.
    pub fn full_name(&self) -> String {
        format!("{}:{}", self.provider, self.name)
    }
}

fn same_layout(a: &[ArgInfo<'_>], b: &[ArgInfo<'_>]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.name == y.name && x.ty == y.ty)
}

/// Which probes a command covers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Filter {
    /// Only this provider.
    pub provider: Option<String>,
    /// Only probe names matching this pattern (`*` and `?`).
    pub probe: Option<String>,
}

impl Filter {
    fn matches(&self, probe: &ProbeInfo<'_>) -> bool {
        self.provider.as_deref().is_none_or(|p| p == probe.provider)
            && self.probe.as_deref().is_none_or(|g| glob(g, probe.name))
    }
}

/// Whether `text` matches `pattern`, where `*` matches any run of
/// characters and `?` any one character.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    // Where the last `*` was, and where in `text` it is matched up to.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Groups `records` by provider and name, in that order, keeping those
/// `filter` matches.
pub fn groups<'a>(
    records: Vec<ProbeInfo<'a>>,
    sites: Option<&HashSet<(String, String)>>,
    filter: &Filter,
) -> Vec<Group<'a>> {
    let mut by_name: BTreeMap<(&'a str, &'a str), Vec<ProbeInfo<'a>>> = BTreeMap::new();
    for record in records.into_iter().filter(|r| filter.matches(r)) {
        by_name
            .entry((record.provider, record.name))
            .or_default()
            .push(record);
    }
    by_name
        .into_iter()
        .map(|((provider, name), records)| Group {
            provider,
            name,
            records,
            has_site: sites.map(|s| s.contains(&site_key(provider, name))),
        })
        .collect()
}

/// `(name: type, ...)`.
fn arg_list(args: &[ArgInfo<'_>]) -> String {
    let args: Vec<String> = args
        .iter()
        .map(|a| format!("{}: {}", a.name, a.ty.as_str()))
        .collect();
    format!("({})", args.join(", "))
}

/// Where a record comes from: `entry of fn get in app::net, src/net.rs:12`.
fn origin(record: &ProbeInfo<'_>) -> String {
    let what = match (record.origin, record.function) {
        (Origin::Probes, _) | (_, None) => "probes!".to_owned(),
        (origin, Some(function)) => format!("{} of fn {function}", origin.as_str()),
    };
    format!(
        "{what} in {}, {}:{}",
        record.module_path, record.file, record.line
    )
}

/// The `list` command's text output.
pub fn list_text(groups: &[Group<'_>]) -> String {
    let mut out = String::new();
    for group in groups {
        let site = match group.has_site {
            Some(false) => "  (no site: its code is not in the binary)",
            _ => "",
        };
        match group.args() {
            Some(args) => {
                let _ = writeln!(out, "{}{}{site}", group.full_name(), arg_list(args));
                for record in &group.records {
                    let _ = writeln!(out, "    {}", origin(record));
                }
            }
            None => {
                let _ = writeln!(out, "{}{site}", group.full_name());
                for record in &group.records {
                    let _ = writeln!(out, "    {}  {}", arg_list(&record.args), origin(record));
                }
            }
        }
    }
    let providers: HashSet<&str> = groups.iter().map(|g| g.provider).collect();
    let _ = writeln!(
        out,
        "{} probe{} in {} provider{}",
        groups.len(),
        if groups.len() == 1 { "" } else { "s" },
        providers.len(),
        if providers.len() == 1 { "" } else { "s" },
    );
    out
}

/// The `list --json` output: one object per record.
pub fn list_json(groups: &[Group<'_>]) -> String {
    let records: Vec<serde_json::Value> = groups
        .iter()
        .flat_map(|g| g.records.iter().map(move |r| (g, r)))
        .map(|(group, r)| {
            let args: Vec<serde_json::Value> = r
                .arg_indices()
                .map(|(index, a)| {
                    serde_json::json!({ "name": a.name, "type": a.ty.as_str(), "index": index })
                })
                .collect();
            serde_json::json!({
                "provider": r.provider,
                "name": r.name,
                "origin": r.origin.as_str(),
                "function": r.function,
                "module_path": r.module_path,
                "file": r.file,
                "line": r.line,
                "args": args,
                "has_site": group.has_site,
            })
        })
        .collect();
    let mut out = serde_json::Value::Array(records).to_string();
    out.push('\n');
    out
}

/// Warnings about the probes, one line each.
pub fn warnings(groups: &[Group<'_>]) -> Vec<String> {
    let mut warnings = Vec::new();
    for group in groups {
        let layouts = group.layouts();
        if layouts > 1 {
            warnings.push(format!(
                "{} has {layouts} different argument lists; DTrace gives each function \
                 its own, so a script has to branch on probefunc. Generated scripts print \
                 no arguments for it",
                group.full_name()
            ));
        }
    }
    warnings
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod report_tests;
