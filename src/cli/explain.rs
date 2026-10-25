//! Evidence-only explanations shared by standalone and daemon command execution.
use anyhow::{Result, bail};
use boundrail::model::Document;
use serde_json::{Value, json};

/// A bounded evidence summary for the entire pending edit, including not-yet-scanned files.
pub(super) fn for_agent(doc: &Document, subjects: &[String]) -> Result<Value> {
    use std::collections::BTreeSet;

    let paths: BTreeSet<_> = subjects.iter().cloned().collect();
    if paths.is_empty() {
        bail!("supply at least one repository-relative file path");
    }
    for path in &paths {
        if !boundrail::collect::is_relative_path(path, false)
            || path.contains(['*', '?', '[', ']', '{', '}', '<', '>'])
        {
            bail!("invalid edit path {path:?}: supply an exact repository-relative file path");
        }
    }
    let compiled = boundrail::rules::compile(&doc.config)?;
    let mut modules = BTreeSet::new();
    let mut layers = BTreeSet::new();
    let mut unscanned = 0;
    let mut unclassified = 0;
    let mut unknown_modules = 0;
    for path in &paths {
        let file = doc.files.iter().find(|file| &file.path == path);
        unscanned += usize::from(file.is_none());
        let module = file
            .map(|file| file.module.clone())
            .or_else(|| compiled.as_ref().and_then(|c| c.module_of(path)));
        match module {
            Some(module) => {
                modules.insert(module);
            }
            None => unknown_modules += 1,
        }
        match compiled.as_ref().and_then(|c| c.layer_for(path)) {
            Some((_, layer)) => {
                layers.insert(layer);
            }
            None => unclassified += 1,
        }
    }
    let imports = doc
        .edges
        .iter()
        .chain(&doc.external_edges)
        .filter(|edge| paths.contains(&edge.from))
        .count();
    let importers = doc
        .edges
        .iter()
        .filter(|edge| paths.contains(&edge.to))
        .count();
    let relevant = match (&compiled, &doc.config.rules) {
        (Some(compiled), Some(rules)) => {
            compiled.relevant(&rules.doc, &paths.iter().cloned().collect::<Vec<_>>())?
        }
        _ => Value::Null,
    };
    let rule_count: usize = ["layers", "forbid", "allow", "public", "entrypoints"]
        .iter()
        .map(|key| relevant[key].as_array().map_or(0, Vec::len))
        .sum();
    let rules = if compiled.is_some() {
        rule_count.to_string()
    } else {
        "unavailable".into()
    };
    let base = format!(
        "Static evidence;files:{};unscanned:{unscanned};imports:{imports};importers:{importers};rules:{rules};unknown_modules:{unknown_modules};unclassified:{unclassified}",
        paths.len()
    );
    let mut detail = format!(
        ";modules:{};layers:{}",
        modules.into_iter().collect::<Vec<_>>().join(","),
        layers.into_iter().collect::<Vec<_>>().join(",")
    );
    if let Some(forbids) = relevant["forbid"].as_array() {
        for rule in forbids {
            detail.push_str(&format!(
                ";forbid:{}->{}",
                rule["from"].as_str().unwrap_or("?"),
                rule["to"].as_str().unwrap_or("?")
            ));
        }
    }
    let mut detail: String = detail.chars().take(400).collect();
    loop {
        let value = json!({"schema":"archgraph/explain-agent/v1", "summary":format!("{base}{detail}"), "omitted":true});
        if serde_json::to_vec(&value)?.len() <= 400 {
            return Ok(value);
        }
        // Remove whole Unicode scalars; JSON escaping is included in the byte budget.
        if detail.pop().is_none() {
            bail!("edit summary exceeds the 400-byte budget");
        }
    }
}

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
        "grouping": boundrail::modules::grouping(&doc.config),
        "source_semantics": boundrail::modules::dependency_semantics(doc.source.language),
        "limitations": doc.source.collector.limitations,
        "hint": "Static imports establish at least these relationships; dynamic dispatch may add others."
    }))
}
