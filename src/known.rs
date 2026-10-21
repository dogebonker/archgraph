//! `layout.known.yaml`: the forbidden imports a layout already has, so `check` fails only on
//! others. The list only shrinks: an entry no forbidden import matches fails too.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{Document, Edge, ExternalNode, Verdict};

/// The list beside a rules file, as a scan document embeds it in `config.rules.known`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    /// File name only, like `RulesRef.path`.
    pub path: String,
    pub entries: Vec<Entry>,
}

/// A forbidden import as `check` prints it: importer, written target and first-match rule, with
/// no line or kind. Ordered by `(from, to, rule)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Entry {
    pub from: String,
    pub to: String,
    pub rule: String,
}

#[derive(Deserialize)]
struct Disk {
    version: u32,
    #[serde(default)]
    known: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Drop the entries that no longer occur; never add one
    Shrink,
    /// List every forbidden import now, creating the file
    Accept,
}

/// `layout.yaml` → `layout.known.yaml`, beside it.
pub fn path(rules: &Path) -> PathBuf {
    rules.with_extension("known.yaml")
}

/// Reads the list beside `rules`, if there is one. Unknown keys and control characters are an
/// error, as in the rules file.
pub fn load(rules: &Path) -> Result<Option<File>> {
    let path = path(rules);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let parse = || -> Result<Vec<Entry>> {
        let given: serde_json::Value = serde_yaml_ng::from_slice(&bytes)?;
        let keys = serde_json::json!({"version": 1, "known": [{"rule": "", "from": "", "to": ""}]});
        if let Some((key, at)) = crate::rules::unknown_key(&given, &keys, "") {
            match at.as_str() {
                "" => bail!("unknown field `{key}`"),
                at => bail!("unknown field `{key}` in {at}"),
            }
        }
        let disk: Disk = serde_json::from_value(given)?;
        if disk.version != 1 {
            bail!("version must be 1");
        }
        for s in disk.known.iter().flat_map(|e| [&e.rule, &e.from, &e.to]) {
            if s.trim().is_empty() || s.chars().any(char::is_control) {
                bail!("{s:?}: known entries must be one nonempty plain line");
            }
        }
        let mut entries = disk.known;
        entries.sort();
        entries.dedup();
        Ok(entries)
    };
    let entries = parse().with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(File {
        path: path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        entries,
    }))
}

/// `path` as `check` prints a target: `pkg:<package>` for a known external.
pub fn policy_target(doc: &Document, path: &str) -> String {
    target(&doc.external_nodes, path)
}

fn target(externals: &[ExternalNode], path: &str) -> String {
    externals
        .iter()
        .find(|node| node.id == path)
        .map(|node| format!("pkg:{}", node.package))
        .unwrap_or_else(|| path.to_owned())
}

fn entry(edge: &Edge, externals: &[ExternalNode]) -> Entry {
    Entry {
        from: edge.from.clone(),
        to: target(externals, edge.original_to.as_deref().unwrap_or(&edge.to)),
        rule: edge.rule.clone().unwrap_or_default(),
    }
}

/// Sets every edge's `known` from the document's list, replacing any flag read from input, and
/// `stale_known` to the entries no forbidden import matches.
pub fn mark(doc: &mut Document) {
    let listed: BTreeSet<&Entry> = doc
        .config
        .rules
        .iter()
        .flat_map(|r| &r.known)
        .flat_map(|f| &f.entries)
        .collect();
    let mut occurring = BTreeSet::new();
    for edge in doc.edges.iter_mut().chain(&mut doc.external_edges) {
        edge.known = false;
        if edge.verdict == Some(Verdict::Forbidden) {
            let entry = entry(edge, &doc.external_nodes);
            edge.known = listed.contains(&entry);
            occurring.insert(entry);
        }
    }
    doc.stale_known = listed
        .into_iter()
        .filter(|e| !occurring.contains(*e))
        .cloned()
        .collect();
}

/// Whether `check` fails on the policy: a forbidden import not listed, or a stale entry.
pub fn fails(doc: &Document) -> bool {
    !doc.stale_known.is_empty()
        || doc
            .edges
            .iter()
            .chain(&doc.external_edges)
            .any(|e| e.verdict == Some(Verdict::Forbidden) && !e.known)
}

/// The line `check` and `report` print for a stale entry.
pub fn stale_line(entry: &Entry) -> String {
    format!(
        "stale known {} -> {} ({}): no longer forbidden; run archgraph check --update-known shrink",
        entry.from, entry.to, entry.rule
    )
}

/// The entries `mode` writes, sorted: `shrink` keeps the listed ones that still occur, `accept`
/// lists every forbidden import.
pub fn update(doc: &Document, mode: Mode) -> Vec<Entry> {
    let entries: BTreeSet<Entry> = match mode {
        Mode::Shrink => doc
            .config
            .rules
            .iter()
            .flat_map(|r| &r.known)
            .flat_map(|f| &f.entries)
            .filter(|e| !doc.stale_known.contains(e))
            .cloned()
            .collect(),
        Mode::Accept => doc
            .edges
            .iter()
            .chain(&doc.external_edges)
            .filter(|e| e.verdict == Some(Verdict::Forbidden))
            .map(|e| entry(e, &doc.external_nodes))
            .collect(),
    };
    entries.into_iter().collect()
}

/// The file for `entries`, one JSON-quoted flow mapping per line so a diff shows one import.
pub fn render(rules: &str, entries: &[Entry]) -> String {
    let quote = |s: &str| serde_json::Value::from(s).to_string();
    let mut out = format!(
        "# Forbidden imports {rules} already has. check fails on any other, and on an entry that no\n\
         # longer occurs. Shrink with: archgraph check --update-known shrink\n\
         version: 1\nknown:{}\n",
        if entries.is_empty() { " []" } else { "" }
    );
    for e in entries {
        out += &format!(
            "  - {{ rule: {}, from: {}, to: {} }}\n",
            quote(&e.rule),
            quote(&e.from),
            quote(&e.to)
        );
    }
    out
}
