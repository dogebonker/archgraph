//! Evidence-only explanations shared by standalone and daemon command execution.
use anyhow::{Result, bail};
use archgraph::model::Document;
use serde_json::{Value, json};

pub(super) fn explain(doc: &Document, subject: &str) -> Result<Value> {
    let file = doc.files.iter().find(|file| file.path == subject);
    let module = doc
        .modules
        .iter()
        .find(|module| module.id == subject || file.is_some_and(|file| module.id == file.module));
    let findings: Vec<_> = doc
        .findings
        .iter()
        .filter(|finding| finding.subject == subject)
        .collect();
    if file.is_none() && module.is_none() && findings.is_empty() {
        bail!(
            "no file, module or finding matches {subject:?}; use report to list module IDs, or pass an exact repository-relative file path"
        );
    }
    let member_files: std::collections::BTreeSet<_> = doc
        .files
        .iter()
        .filter(|file| file.module == subject)
        .map(|file| file.path.as_str())
        .collect();
    let matches = |path: &str| path == subject || member_files.contains(path);
    let imports: Vec<_> = doc
        .edges
        .iter()
        .filter(|edge| matches(&edge.from))
        .collect();
    let imported_by: Vec<_> = doc.edges.iter().filter(|edge| matches(&edge.to)).collect();
    let cycles: Vec<_> =
        doc.cycles
            .iter()
            .filter(|cycle| {
                cycle.members.iter().any(|member| {
                    member == subject || file.is_some_and(|file| member == &file.module)
                })
            })
            .collect();
    Ok(json!({
        "schema": "archgraph/explain/v1", "kind": "explain", "subject": subject,
        "file": file, "module": module, "imports": imports, "imported_by": imported_by,
        "cycles": cycles, "findings": findings,
        "limitations": doc.source.collector.limitations,
        "hint": "Static imports establish at least these relationships; dynamic dispatch may add others."
    }))
}
