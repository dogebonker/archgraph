//! Java source references through the JDK compiler tree API; no project build is run.

use std::fs;
use std::io::Write;
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use globset::GlobSet;
use serde::Deserialize;

use super::RawGraph;
use crate::model::{Collector, Edge, EdgeKind, ImportRecord};

/// JDK 11 or later, using source-file launch from an isolated temporary directory.
pub fn run(root: &Path, java: &str, excludes: &GlobSet) -> Result<Vec<u8>> {
    let root = fs::canonicalize(root)?;
    let java = if java.contains(['/', '\\']) {
        fs::canonicalize(java)
            .with_context(|| format!("--java {java}: no such executable"))?
            .to_string_lossy()
            .into_owned()
    } else {
        java.to_owned()
    };
    let mut dirs = vec![root.clone()];
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = entry
                .path()
                .strip_prefix(&root)?
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') && name != "node_modules" && !excludes.is_match(&rel) {
                    dirs.push(entry.path());
                }
            } else if entry.file_type()?.is_file()
                && name.ends_with(".java")
                && !excludes.is_match(&rel)
            {
                files.push(rel);
            }
        }
    }
    files.sort();
    ensure!(!files.is_empty(), "no Java source files after excludes");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("archgraph-java-{}-{nanos}", std::process::id()));
    fs::create_dir(&dir)?;
    let result = (|| -> Result<_> {
        fs::write(
            dir.join("ArchgraphJava.java"),
            include_str!("ArchgraphJava.java"),
        )?;
        let mut child = Command::new(&java)
            .args(["--class-path", ".", "ArchgraphJava.java"])
            .arg(&root)
            .current_dir(&dir)
            .env_remove("JDK_JAVA_OPTIONS")
            .env_remove("JAVA_TOOL_OPTIONS")
            .env_remove("_JAVA_OPTIONS")
            .env_remove("CLASSPATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("running {java}: install JDK 11 or later, or pass --java"))?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(files.join("\0").as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "Java helper failed (requires JDK 11 or later): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    })();
    let _ = fs::remove_dir_all(&dir);
    result
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
            !path.is_empty()
                && !path.contains(['\\', '\0', ':'])
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
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
            type_marking: None,
            main_packages: Vec::new(),
            bound_files: Vec::new(),
            go_modules: Vec::new(),
            limitations: Vec::new(),
        },
        files: dump.files,
        edges: dump
            .edges
            .into_iter()
            .map(|e| Edge {
                from: e.from,
                to: e.to,
                kind: EdgeKind::Static,
                verdict: None,
                rule: None,
                reach: None,
                ..crate::model::Edge::default()
            })
            .collect(),
    })
}
