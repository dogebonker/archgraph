//! Before/after evidence for the offline import comparison, separate from gate deltas.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{Config, Document, EdgeKind, Language, ScanInput, Source, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Added,
    Removed,
    Changed,
    Unchanged,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Module,
    Package,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Files {
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub status: Status,
    pub before: Option<Files>,
    pub after: Option<Files>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub line: Option<usize>,
    pub specifier: Option<String>,
    pub original_to: Option<String>,
    pub is_reexport: bool,
    pub verdict: Option<Verdict>,
    pub rule: Option<String>,
    pub reasons: Vec<String>,
    pub sources: Vec<String>,
    pub known: bool,
}

#[derive(Debug, Serialize)]
pub struct ImportState {
    pub from_node: String,
    pub to_node: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Serialize)]
pub struct Import {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub status: Status,
    pub changes: Vec<&'static str>,
    pub before: Option<ImportState>,
    pub after: Option<ImportState>,
}

#[derive(Debug, Serialize)]
pub struct Collector {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Policy {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub label: String,
    pub commit: Option<String>,
    pub dirty: Option<bool>,
    pub tool_version: Option<String>,
    pub language: Language,
    pub collector: Collector,
    pub scan_root: Option<String>,
    pub policy: Option<Policy>,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub status: &'static str,
    pub policy_changed: bool,
    pub warnings: Vec<String>,
    pub before: Snapshot,
    pub after: Snapshot,
}

#[derive(Debug, Serialize)]
pub struct View {
    pub comparison: Comparison,
    pub nodes: Vec<Node>,
    pub imports: Vec<Import>,
}

#[derive(Debug, Deserialize)]
struct Tool {
    name: Option<String>,
    version: Option<String>,
}

/// Original recorded settings, captured before restoration replaces analyzer metadata.
pub struct Provenance {
    label: String,
    source: Source,
    config: Config,
    tool: Option<Tool>,
}

/// Read provenance and graph input from the same bytes. The caller validates and restores input.
pub fn read(text: &str, label: &str) -> Result<(ScanInput, Provenance)> {
    #[derive(Deserialize)]
    struct Header {
        tool: Option<Tool>,
    }
    let header: Header =
        serde_json::from_str(text).context("reading recorded analyzer metadata")?;
    let input: ScanInput = serde_json::from_str(text).context("reading comparison scan")?;
    let provenance = Provenance {
        label: label.into(),
        source: input.source.clone(),
        config: input.config.clone(),
        tool: header.tool,
    };
    Ok((input, provenance))
}

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

impl Provenance {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            label: self.label.clone(),
            commit: self.source.git.as_ref().map(|git| git.commit.clone()),
            dirty: self.source.git.as_ref().map(|git| git.dirty),
            tool_version: present(self.tool.as_ref().and_then(|tool| tool.version.as_deref()))
                .map(str::to_owned),
            language: self.source.language,
            collector: Collector {
                name: self.source.collector.name.clone(),
                version: present(self.source.collector.version.as_deref()).map(str::to_owned),
            },
            scan_root: self.source.scan_root.clone(),
            policy: self.config.rules.as_ref().map(|rules| Policy {
                path: rules.path.clone(),
                sha256: rules.sha256.clone(),
            }),
        }
    }

    fn selection(&self) -> Value {
        let config = &self.config;
        let policy = config.rules.as_ref().map(|rules| &rules.doc);
        json!({
            "source_root": config.source_root,
            "depth": config.depth,
            "granularity": config.granularity,
            "package_modules": config.package_modules,
            "excludes": config.excludes,
            "includes": config.includes,
            "explicit_excludes": config.explicit_excludes,
            "include_tests": config.include_tests,
            "include_generated": config.include_generated,
            "no_default_excludes": config.no_default_excludes,
            "all_kinds": config.all_kinds,
            "policy_excludes": policy.map(|rules| rules.exclude.as_slice()).unwrap_or_default(),
            "policy_includes": policy.map(|rules| rules.include.as_slice()).unwrap_or_default(),
            "global_capture": policy.is_some_and(|rules| rules.names_globals()),
        })
    }
}

fn compatible(before: &Provenance, after: &Provenance) -> Result<(bool, Vec<String>)> {
    ensure!(
        before.source.language == after.source.language,
        "incompatible comparison languages"
    );
    ensure!(
        before.selection() == after.selection(),
        "incompatible comparison collection selection or grouping settings"
    );
    let mut warnings = Vec::new();
    for (name, old, new) in [
        (
            "collector name",
            present(Some(before.source.collector.name.as_str())),
            present(Some(after.source.collector.name.as_str())),
        ),
        (
            "collector version",
            present(before.source.collector.version.as_deref()),
            present(after.source.collector.version.as_deref()),
        ),
        (
            "scan root",
            present(before.source.scan_root.as_deref()),
            present(after.source.scan_root.as_deref()),
        ),
    ] {
        match (old, new) {
            (Some(old), Some(new)) => ensure!(old == new, "incompatible recorded {name}"),
            _ => warnings.push(format!(
                "Recorded {name} is unavailable on one or both snapshots."
            )),
        }
    }
    ensure!(
        before.source.collector.type_marking == after.source.collector.type_marking,
        "incompatible collector type-marking capabilities"
    );
    match (&before.config.collection, &after.config.collection) {
        (Some(old), Some(new)) => ensure!(
            serde_json::to_value(old)? == serde_json::to_value(new)?,
            "incompatible recorded collection settings"
        ),
        _ => warnings
            .push("Recorded collection settings are unavailable on one or both snapshots.".into()),
    }
    for (name, old, new) in [
        (
            "analyzer name",
            present(before.tool.as_ref().and_then(|tool| tool.name.as_deref())),
            present(after.tool.as_ref().and_then(|tool| tool.name.as_deref())),
        ),
        (
            "analyzer version",
            present(
                before
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.version.as_deref()),
            ),
            present(after.tool.as_ref().and_then(|tool| tool.version.as_deref())),
        ),
    ] {
        match (old, new) {
            (Some(old), Some(new)) if old != new => warnings.push(format!(
                "Recorded {name} differs ({old} versus {new}); original analysis parity is unverified."
            )),
            (Some(_), Some(_)) => (),
            _ => warnings.push(format!("Recorded {name} is unavailable on one or both snapshots.")),
        }
    }
    Ok((warnings.is_empty(), warnings))
}

struct NodeState {
    kind: NodeKind,
    label: String,
    files: Files,
}

fn nodes(doc: &Document) -> BTreeMap<String, NodeState> {
    let mut nodes: BTreeMap<_, _> = doc
        .modules
        .iter()
        .map(|module| {
            (
                format!("module:{}", module.id),
                NodeState {
                    kind: NodeKind::Module,
                    label: module.id.clone(),
                    files: Files { files: Vec::new() },
                },
            )
        })
        .collect();
    for file in &doc.files {
        nodes
            .get_mut(&format!("module:{}", file.module))
            .expect("analyzed module exists")
            .files
            .files
            .push(file.path.clone());
    }
    for node in nodes.values_mut() {
        node.files.files.sort();
    }
    for external in &doc.external_nodes {
        let (kind, prefix) = if external.global {
            (NodeKind::Global, "global")
        } else {
            (NodeKind::Package, "pkg")
        };
        nodes.insert(
            format!("external:{}", external.id),
            NodeState {
                kind,
                label: format!("{prefix}:{}", external.package),
                files: Files { files: Vec::new() },
            },
        );
    }
    nodes
}

type ImportKey = (String, String, &'static str);

fn imports(doc: &Document) -> Result<BTreeMap<ImportKey, ImportState>> {
    let files: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|file| (&file.path, &file.module))
        .collect();
    let externals: BTreeSet<_> = doc.external_nodes.iter().map(|node| &node.id).collect();
    let mut imports: BTreeMap<ImportKey, ImportState> = BTreeMap::new();
    for edge in doc.edges.iter().chain(&doc.external_edges) {
        let from_node = format!(
            "module:{}",
            files
                .get(&edge.from)
                .context("import source has no module")?
        );
        let to_node = if externals.contains(&edge.to) {
            format!("external:{}", edge.to)
        } else {
            format!(
                "module:{}",
                files
                    .get(&edge.to)
                    .context("import destination has no module")?
            )
        };
        let decision = edge.rule.as_deref().unwrap_or_default();
        let policy = doc.config.rules.as_ref().map(|rules| &rules.doc);
        let evidence = Evidence {
            line: edge.line,
            specifier: edge.specifier.clone(),
            original_to: edge.original_to.clone(),
            is_reexport: edge.is_reexport
                || matches!(edge.kind, EdgeKind::Reexport | EdgeKind::TypeReexport),
            verdict: edge.verdict,
            rule: edge.rule.clone(),
            reasons: policy
                .map(|rules| {
                    rules
                        .reasons(decision)
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            sources: policy
                .map(|rules| {
                    rules
                        .sources(decision)
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            known: edge.known,
        };
        imports
            .entry((edge.from.clone(), edge.to.clone(), edge.kind.as_str()))
            .or_insert_with(|| ImportState {
                from_node,
                to_node,
                evidence: Vec::new(),
            })
            .evidence
            .push(evidence);
    }
    for state in imports.values_mut() {
        state.evidence.sort_by_cached_key(|evidence| {
            serde_json::to_string(evidence).expect("evidence serializes")
        });
        state.evidence.dedup();
    }
    Ok(imports)
}

fn signatures<T: Serialize>(values: impl Iterator<Item = T>) -> BTreeSet<String> {
    values
        .map(|value| serde_json::to_string(&value).expect("evidence signatures serialize"))
        .collect()
}

fn decisions_changed(
    before: &[Evidence],
    after: &[Evidence],
    decide: impl Fn(&Evidence) -> String,
) -> bool {
    type Occurrence<'a> = (Option<&'a str>, Option<&'a str>, bool);
    fn group<'a>(
        evidence: &'a [Evidence],
        decide: &impl Fn(&Evidence) -> String,
    ) -> BTreeMap<Occurrence<'a>, BTreeSet<String>> {
        let mut groups: BTreeMap<_, BTreeSet<String>> = BTreeMap::new();
        for e in evidence {
            groups
                .entry((
                    e.original_to.as_deref(),
                    e.specifier.as_deref(),
                    e.is_reexport,
                ))
                .or_default()
                .insert(decide(e));
        }
        groups
    }
    let before = group(before, &decide);
    let after = group(after, &decide);
    before.values().flatten().collect::<BTreeSet<_>>()
        != after.values().flatten().collect::<BTreeSet<_>>()
        || before
            .iter()
            .any(|(occurrence, old)| after.get(occurrence).is_some_and(|new| new != old))
}

fn changes(before: &ImportState, after: &ImportState) -> Vec<&'static str> {
    let mut changes = Vec::new();
    if decisions_changed(&before.evidence, &after.evidence, |e| {
        serde_json::to_string(&e.verdict).expect("verdict serializes")
    }) {
        changes.push("verdict");
    }
    if decisions_changed(&before.evidence, &after.evidence, |e| {
        serde_json::to_string(&(&e.rule, &e.reasons, &e.sources, e.known))
            .expect("rule evidence serializes")
    }) {
        changes.push("rule");
    }
    if before.from_node != after.from_node || before.to_node != after.to_node {
        changes.push("module");
    }
    fn source(e: &Evidence) -> (Option<usize>, Option<&str>, Option<&str>, bool) {
        (
            e.line,
            e.specifier.as_deref(),
            e.original_to.as_deref(),
            e.is_reexport,
        )
    }
    if signatures(before.evidence.iter().map(source))
        != signatures(after.evidence.iter().map(source))
    {
        changes.push("evidence");
    }
    changes
}

/// Project validated, recomputed scans while retaining their original recorded provenance.
pub fn project(
    before: &Document,
    old: &Provenance,
    after: &Document,
    new: &Provenance,
) -> Result<View> {
    let (recorded_compatible, mut warnings) = compatible(old, new)?;
    let policy_changed =
        serde_json::to_value(&old.config.rules)? != serde_json::to_value(&new.config.rules)?;
    warnings.push("Matching recorded settings do not establish repository identity, identical analyzer binaries, or complete dependency coverage.".into());
    warnings.push("This view compares supplied static evidence. Missing or dynamically resolved relationships can remain unobserved.".into());
    for (label, doc) in [("Before", before), ("After", after)] {
        warnings.extend(
            doc.source
                .collector
                .limitations
                .iter()
                .map(|note| format!("{label}: {note}")),
        );
        if let Some(resolution) = &doc.import_resolution
            && resolution.unresolved > 0
        {
            warnings.push(format!(
                "{label}: {} import requests remain unresolved.",
                resolution.unresolved
            ));
        }
    }
    if before.source.language == Language::Rust {
        warnings.push("Rust runtime-kind relationships describe source coupling, not proved runtime initialization dependencies.".into());
    }
    let mut was = nodes(before);
    let mut is = nodes(after);
    let keys: BTreeSet<_> = was.keys().chain(is.keys()).cloned().collect();
    let nodes = keys
        .into_iter()
        .map(|id| {
            let before = was.remove(&id);
            let after = is.remove(&id);
            let status = match (&before, &after) {
                (None, _) => Status::Added,
                (_, None) => Status::Removed,
                (Some(before), Some(after)) if before.files != after.files => Status::Changed,
                _ => Status::Unchanged,
            };
            let present = after
                .as_ref()
                .or(before.as_ref())
                .expect("one node side exists");
            Node {
                id,
                kind: present.kind,
                label: present.label.clone(),
                status,
                before: before.map(|state| state.files),
                after: after.map(|state| state.files),
            }
        })
        .collect();
    let mut was = imports(before)?;
    let mut is = imports(after)?;
    let keys: BTreeSet<_> = was.keys().chain(is.keys()).cloned().collect();
    let imports = keys
        .into_iter()
        .map(|(from, to, kind)| {
            let before = was.remove(&(from.clone(), to.clone(), kind));
            let after = is.remove(&(from.clone(), to.clone(), kind));
            let differences = match (&before, &after) {
                (Some(before), Some(after)) => changes(before, after),
                _ => Vec::new(),
            };
            let status = match (&before, &after) {
                (None, _) => Status::Added,
                (_, None) => Status::Removed,
                _ if !differences.is_empty() => Status::Changed,
                _ => Status::Unchanged,
            };
            Import {
                from,
                to,
                kind,
                status,
                changes: differences,
                before,
                after,
            }
        })
        .collect();
    Ok(View {
        comparison: Comparison {
            status: if recorded_compatible {
                "recorded-compatible"
            } else {
                "unverified"
            },
            policy_changed,
            warnings,
            before: old.snapshot(),
            after: new.snapshot(),
        },
        nodes,
        imports,
    })
}
