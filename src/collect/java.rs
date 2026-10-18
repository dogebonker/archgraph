//! Java source references through the JDK compiler tree API; no project build is run.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use globset::GlobSet;
use serde::Deserialize;

use super::RawGraph;
use crate::model::{Collector, Edge, EdgeKind, ImportRecord};

/// JDK 11 or later, using source-file launch from an isolated temporary directory.
pub fn run(root: &Path, java: &str, excludes: &GlobSet) -> Result<Vec<u8>> {
    let root = fs::canonicalize(root)?;
    let java = super::resolve_executable("java", java)?;
    let mut files = super::native::source_files(&root, &["java"])?;
    files.retain(|file| {
        !excludes.is_match(file)
            && !file
                .split('/')
                .rev()
                .skip(1)
                .any(|dir| dir.starts_with('.'))
    });
    ensure!(!files.is_empty(), "no Java source files after excludes");
    let dir = super::TempDir::new("java")?;
    fs::write(
        dir.path().join("ArchgraphJava.java"),
        include_str!("ArchgraphJava.java"),
    )?;
    let out = super::run_with_input(
        Command::new(&java)
            .args(["--class-path", ".", "ArchgraphJava.java"])
            .arg(&root)
            .current_dir(dir.path())
            .env_remove("JDK_JAVA_OPTIONS")
            .env_remove("JAVA_TOOL_OPTIONS")
            .env_remove("_JAVA_OPTIONS")
            .env_remove("CLASSPATH"),
        files.join("\0").into_bytes(),
    )
    .with_context(|| {
        format!(
            "running {}: install JDK 11 or later, or pass --java",
            java.display()
        )
    })?;
    if !out.status.success() {
        bail!(
            "Java helper failed (requires JDK 11 or later): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

#[derive(Deserialize)]
struct Dump {
    java_version: String,
    files: Vec<String>,
    edges: Vec<DumpEdge>,
    unresolved: usize,
    #[serde(default)]
    imports: Option<Vec<ImportRecord>>,
}

#[derive(Deserialize)]
struct DumpEdge {
    from: String,
    to: String,
}

/// `unresolved` counts compiler error diagnostics: unavailable libraries can leave a partial graph.
pub fn parse(json: &[u8]) -> Result<RawGraph> {
    let dump: Dump = serde_json::from_slice(json).context("reading Java helper JSON")?;
    for path in dump
        .files
        .iter()
        .chain(dump.edges.iter().flat_map(|e| [&e.from, &e.to]))
    {
        ensure!(
            super::is_relative_path(path, false),
            "Java helper path must be root-relative: {path:?}"
        );
    }
    Ok(RawGraph {
        imports: dump.imports,
        metadata: Default::default(),
        collector: Collector {
            name: "java-javac".to_owned(),
            version: Some(dump.java_version),
            cruised: dump.files.len(),
            unresolved: Some(dump.unresolved),
            ..Default::default()
        },
        files: dump.files,
        edges: dump
            .edges
            .into_iter()
            .map(|e| Edge {
                from: e.from,
                to: e.to,
                kind: EdgeKind::Static,
                ..crate::model::Edge::default()
            })
            .collect(),
    })
}
