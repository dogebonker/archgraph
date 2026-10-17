//! Refine barrel consumers with TypeScript's symbol resolver without executing source code.
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use archgraph::model::{Document, EdgeKind};
use serde::Deserialize;

const HELPER: &str = include_str!("../impact_typescript.cjs");

#[derive(Deserialize)]
struct Replacement {
    from: String,
    barrel: String,
    targets: Vec<String>,
}

pub(super) fn resolve_barrels(
    root: &Path,
    depcruise: Option<&Path>,
    doc: &mut Document,
    changed: &[String],
) -> Result<usize> {
    if !doc.files.iter().any(|file| file.barrel)
        || doc
            .files
            .iter()
            .any(|file| file.barrel && changed.contains(&file.path))
    {
        return Ok(0);
    }
    let input = serde_json::to_vec(&serde_json::json!({
        "root": root.canonicalize().context("resolving TypeScript impact root")?,
        "depcruise": depcruise,
        "files": doc.files.iter().map(|file| serde_json::json!({"path":file.path,"barrel":file.barrel})).collect::<Vec<_>>()
    }))?;
    let mut child = Command::new("node")
        .args(["-e", HELPER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running TypeScript impact symbol resolver")?;
    child
        .stdin
        .take()
        .context("opening TypeScript resolver input")?
        .write_all(&input)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "TypeScript barrel resolution failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let replacements: Vec<Replacement> =
        serde_json::from_slice(&output.stdout).context("reading TypeScript barrel replacements")?;
    let files: BTreeSet<_> = doc.files.iter().map(|file| file.path.as_str()).collect();
    let barrels: BTreeSet<_> = doc
        .files
        .iter()
        .filter(|file| file.barrel)
        .map(|file| file.path.as_str())
        .collect();
    for replacement in &replacements {
        if !files.contains(replacement.from.as_str())
            || !barrels.contains(replacement.barrel.as_str())
            || replacement.targets.is_empty()
            || replacement
                .targets
                .iter()
                .any(|target| !files.contains(target.as_str()) || barrels.contains(target.as_str()))
        {
            bail!("invalid TypeScript barrel replacement");
        }
    }
    let mut count = 0;
    for replacement in replacements {
        let mut added = Vec::new();
        doc.edges.retain(|edge| {
            if edge.from != replacement.from
                || edge.to != replacement.barrel
                || !matches!(edge.kind, EdgeKind::Static | EdgeKind::Type)
            {
                return true;
            }
            for target in &replacement.targets {
                let mut edge = edge.clone();
                edge.to.clone_from(target);
                added.push(edge);
            }
            count += 1;
            false
        });
        doc.edges.extend(added);
    }
    doc.edges
        .sort_by(|a, b| (&a.from, &a.to, a.kind).cmp(&(&b.from, &b.to, b.kind)));
    doc.edges
        .dedup_by(|a, b| a.from == b.from && a.to == b.to && a.kind == b.kind);
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archgraph::model::{Config, Edge, Source};
    use serde_json::json;

    #[test]
    fn changed_nested_barrel_preserves_outer_consumer_without_running_node() {
        let source: Source = serde_json::from_value(json!({
            "language":"typescript", "collector":{"name":"fixture", "cruised":4},
            "git":null, "generated_at":"fixture"
        }))
        .unwrap();
        let config: Config = serde_json::from_value(json!({
            "depth":1, "source_root":".", "excludes":[], "include_tests":true, "rules":null
        }))
        .unwrap();
        let edge = |from: &str, to: &str, kind| Edge {
            from: from.into(),
            to: to.into(),
            kind,
            verdict: None,
            rule: None,
            reach: None,
        };
        let mut doc = archgraph::analyze(
            source,
            config,
            ["consumer.ts", "index.ts", "nested/index.ts", "a.ts"]
                .map(String::from)
                .to_vec(),
            vec![
                edge("consumer.ts", "index.ts", EdgeKind::Static),
                edge("index.ts", "nested/index.ts", EdgeKind::Reexport),
                edge("nested/index.ts", "a.ts", EdgeKind::Reexport),
            ],
            vec![],
            None,
        )
        .unwrap();
        let before = doc.edges.clone();
        assert_eq!(
            resolve_barrels(
                Path::new("/nonexistent-impact-root"),
                None,
                &mut doc,
                &["nested/index.ts".into()]
            )
            .unwrap(),
            0
        );
        assert_eq!(doc.edges, before);
    }
}
