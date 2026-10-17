//! Explicit import accounting is independent of the internal symbol-reference graph.
use crate::model::{Document, Edge, ExternalNode, ImportResolution};
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
    let mut packages = BTreeMap::new();
    let mut edges = BTreeSet::new();
    doc.external_edges.clear();
    for import in imports {
        if let Some(package) = &import.external {
            let id = format!(
                "external:{}:{package}",
                format!("{:?}", doc.source.language).to_lowercase()
            );
            packages.insert(package.clone(), id.clone());
            if edges.insert((import.from.clone(), id.clone(), import.kind)) {
                doc.external_edges.push(Edge {
                    from: import.from.clone(),
                    to: id,
                    kind: import.kind,
                    specifier: Some(import.specifier.clone()),
                    line: import.line,
                    weight: Some(
                        doc.config
                            .rules
                            .as_ref()
                            .and_then(|r| r.doc.edge_weights)
                            .unwrap_or_default()
                            .weight(import.kind),
                    ),
                    ..Default::default()
                });
            }
        }
    }
    doc.external_nodes = packages
        .into_iter()
        .map(|(package, id)| ExternalNode {
            id,
            external: true,
            package,
        })
        .collect();
}

pub fn validate(
    imports: Option<&[crate::model::ImportRecord]>,
    files: &[crate::model::File],
) -> anyhow::Result<()> {
    use anyhow::bail;
    let paths: BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
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
