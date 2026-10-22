//! Explicit import accounting is independent of the internal symbol-reference graph.
use crate::model::{Document, Edge, EdgeKind, ExternalNode, ImportResolution};
use std::collections::{BTreeMap, BTreeSet};

pub fn attach(doc: &mut Document) {
    let Some(imports) = &doc.imports else {
        return;
    };
    doc.unresolved = imports
        .iter()
        .filter(|i| i.unresolved.is_some())
        .cloned()
        .collect();
    let resolved = imports.len() - doc.unresolved.len();
    doc.import_resolution = Some(ImportResolution {
        imports: imports.len(),
        resolved,
        unresolved: doc.unresolved.len(),
        resolved_percent: (!imports.is_empty())
            .then(|| (resolved as f64 / imports.len() as f64 * 1000.0).round() / 10.0),
    });
    doc.external_exposure = crate::exposure::report(doc);
}

/// Build package identities and deduplicated edges before applying policy. A host global becomes
/// a node and a runtime edge only when a `global:` rule names it, so other layouts keep their
/// verdicts.
pub fn external_graph(
    source: &crate::model::Source,
    config: &crate::model::Config,
    rules: Option<&crate::rules::Compiled>,
) -> (Vec<ExternalNode>, Vec<Edge>) {
    let mut packages = BTreeMap::new();
    let mut globals = BTreeMap::new();
    let mut edges = BTreeSet::new();
    let mut external_edges = Vec::new();
    let weight = |kind| {
        Some(
            config
                .rules
                .as_ref()
                .and_then(|r| r.doc.edge_weights)
                .unwrap_or_default()
                .weight(kind),
        )
    };
    for import in config.import_records.iter().flatten() {
        if let Some(package) = &import.external {
            let id = external_id(source.language, package);
            packages.insert(package.clone(), id.clone());
            if edges.insert((import.from.clone(), id.clone(), import.kind)) {
                external_edges.push(Edge {
                    from: import.from.clone(),
                    to: id,
                    kind: import.kind,
                    specifier: Some(import.specifier.clone()),
                    line: import.line,
                    weight: weight(import.kind),
                    ..Default::default()
                });
            }
        }
    }
    for global in config.globals.iter().flatten() {
        if !rules.is_some_and(|r| r.names_global(&global.name)) {
            continue;
        }
        let id = global_id(source.language, &global.name);
        globals.insert(global.name.clone(), id.clone());
        if edges.insert((global.from.clone(), id.clone(), EdgeKind::Static)) {
            external_edges.push(Edge {
                from: global.from.clone(),
                to: id,
                kind: EdgeKind::Static,
                specifier: Some(global.name.clone()),
                line: Some(global.line),
                weight: weight(EdgeKind::Static),
                ..Default::default()
            });
        }
    }
    let node = |global: bool| {
        move |(package, id)| ExternalNode {
            id,
            external: true,
            package,
            global,
        }
    };
    let external_nodes = packages
        .into_iter()
        .map(node(false))
        .chain(globals.into_iter().map(node(true)))
        .collect();
    (external_nodes, external_edges)
}

/// The external nodes by id, named as rules select them: `pkg:<package>` or `global:<name>`.
pub fn policy_names(nodes: &[ExternalNode]) -> BTreeMap<String, String> {
    nodes
        .iter()
        .map(|n| {
            let prefix = if n.global { "global" } else { "pkg" };
            (n.id.clone(), format!("{prefix}:{}", n.package))
        })
        .collect()
}

pub fn validate(
    imports: Option<&[crate::model::ImportRecord]>,
    globals: Option<&[crate::model::GlobalUse]>,
    files: &[crate::model::File],
) -> anyhow::Result<()> {
    use anyhow::bail;
    let paths: BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
    if let Some(global) = globals
        .into_iter()
        .flatten()
        .find(|g| !paths.contains(g.from.as_str()) || g.line == 0)
    {
        bail!(
            "global use {} in {} must name a scanned file and a positive source line",
            global.name,
            global.from
        );
    }
    if let Some(imports) = imports {
        for import in imports {
            if !paths.contains(import.from.as_str()) {
                bail!(
                    "import accounting names an unscanned importer {}",
                    import.from
                );
            }
            if import.line == Some(0)
                || (import.unresolved.is_none()
                    && import.targets.is_empty()
                    && import.external.is_none()
                    && import.namespace.is_none())
            {
                bail!(
                    "import accounting must contain a resolution or an unresolved reason, and positive source lines"
                );
            }
            for target in import.targets.iter().chain(import.namespace.iter()) {
                if target.starts_with('/')
                    || target
                        .split('/')
                        .any(|p| p.is_empty() || p == "." || p == "..")
                {
                    bail!("import accounting targets must stay inside the scan root");
                }
            }
            if import
                .external
                .as_ref()
                .is_some_and(|p| p.trim().is_empty())
            {
                bail!("external package names must not be empty");
            }
        }
    }
    Ok(())
}

/// Explicit package or global policy cannot produce an authoritative verdict without collector
/// accounting.
pub fn require_policy_accounting(config: &crate::model::Config) -> anyhow::Result<()> {
    if config.import_records.is_none()
        && config.rules.as_ref().is_some_and(|r| {
            r.doc
                .forbid
                .iter()
                .map(|r| &r.to)
                .chain(r.doc.allow.iter().map(|r| &r.to))
                .any(|to| to.starts_with("pkg:") || to.starts_with("category:"))
        })
    {
        anyhow::bail!(
            "package rules require import accounting; rescan with a collector that records external imports"
        );
    }
    if config.globals.is_none() && config.rules.as_ref().is_some_and(|r| r.doc.names_globals()) {
        anyhow::bail!(
            "global rules require a native JavaScript/TypeScript scan, rescan with --engine native"
        );
    }
    Ok(())
}

fn external_id(language: crate::model::Language, package: &str) -> String {
    format!(
        "external:{}:{package}",
        format!("{language:?}").to_lowercase()
    )
}

fn global_id(language: crate::model::Language, name: &str) -> String {
    external_id(language, &format!("global:{name}"))
}

/// File identities and derived package and global identities must remain disjoint.
pub fn validate_external_identities(
    source: &crate::model::Source,
    imports: Option<&[crate::model::ImportRecord]>,
    globals: Option<&[crate::model::GlobalUse]>,
    files: &[crate::model::File],
) -> anyhow::Result<()> {
    let packages: BTreeSet<_> = imports
        .into_iter()
        .flatten()
        .filter_map(|record| record.external.as_ref())
        .map(|package| external_id(source.language, package))
        .chain(
            globals
                .into_iter()
                .flatten()
                .map(|global| global_id(source.language, &global.name)),
        )
        .collect();
    if let Some(file) = files.iter().find(|file| packages.contains(&file.path)) {
        anyhow::bail!(
            "file {} collides with an external package node identity; rename the file before analyzing package policies",
            file.path
        );
    }
    Ok(())
}
