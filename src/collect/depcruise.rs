//! dependency-cruiser 18.4.0 (with typescript 6.0.3).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::{RawGraph, TS_EXTENSIONS};
use crate::model::{Collector, Edge, EdgeKind};

#[derive(Deserialize)]
struct Output {
    modules: Vec<Module>,
    summary: Summary,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Module {
    source: String,
    #[serde(default)]
    core_module: bool,
    #[serde(default)]
    could_not_resolve: bool,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dependency {
    resolved: String,
    #[serde(default)]
    core_module: bool,
    #[serde(default)]
    could_not_resolve: bool,
    #[serde(default)]
    dependency_types: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    total_cruised: usize,
    environment: Option<Environment>,
}

#[derive(Deserialize)]
struct Environment {
    version: String,
}

/// Runs depcruise in `root`. `--ts-pre-compilation-deps` keeps `import type` edges and
/// `--ts-config` resolves tsconfig path aliases such as `@/`.
pub fn run(root: &Path, source_dir: &str) -> Result<Vec<u8>> {
    let mut cmd = Command::new("npx");
    cmd.current_dir(root).args([
        "--no-install",
        "depcruise",
        "--output-type",
        "json",
        "--no-config",
        "--do-not-follow",
        "node_modules",
        "--ts-pre-compilation-deps",
    ]);
    if root.join("tsconfig.json").is_file() {
        cmd.args(["--ts-config", "tsconfig.json"]);
    }
    let out = cmd
        .arg(source_dir)
        .output()
        .context("running npx (is Node.js installed?)")?;
    if !out.status.success() {
        bail!(
            "depcruise failed: {}\ninstall the pinned tools in the scanned repository: \
             npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

pub fn parse(json: &[u8]) -> Result<RawGraph> {
    let out: Output = serde_json::from_slice(json).context("reading dependency-cruiser JSON")?;
    if out.summary.total_cruised == 0 {
        bail!(
            "dependency-cruiser cruised 0 files (it does not support TypeScript 7; pin typescript@6.0.3)"
        );
    }
    let inside = |path: &str| {
        !path.starts_with("../")
            && !path.starts_with('/')
            && path
                .rsplit_once('.')
                .is_some_and(|(_, ext)| TS_EXTENSIONS.contains(&ext))
    };
    let mut files = Vec::new();
    let mut kinds: BTreeMap<(&str, &str), EdgeKind> = BTreeMap::new();
    let mut unresolved = 0;
    for m in &out.modules {
        unresolved += m
            .dependencies
            .iter()
            .filter(|d| d.could_not_resolve)
            .count();
        if m.core_module || m.could_not_resolve || !inside(&m.source) {
            continue;
        }
        files.push(m.source.clone());
        for d in &m.dependencies {
            if d.core_module || d.could_not_resolve || !inside(&d.resolved) {
                continue;
            }
            let has = |t: &str| d.dependency_types.iter().any(|x| x == t);
            // `export type … from` is reported as type-only and export: still a re-export.
            let kind = if has("export") {
                EdgeKind::Reexport
            } else if has("type-only") {
                EdgeKind::Type
            } else if has("dynamic-import") {
                EdgeKind::Dynamic
            } else {
                EdgeKind::Static
            };
            let slot = kinds.entry((&m.source, &d.resolved)).or_insert(kind);
            *slot = (*slot).max(kind);
        }
    }
    let edges = kinds
        .into_iter()
        .map(|((from, to), kind)| Edge {
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            verdict: None,
            rule: None,
            reach: None,
        })
        .collect();
    let collector = Collector {
        name: "dependency-cruiser".to_owned(),
        version: out.summary.environment.map(|e| e.version),
        cruised: out.summary.total_cruised,
        unresolved: Some(unresolved),
    };
    Ok(RawGraph {
        files,
        edges,
        collector,
    })
}
