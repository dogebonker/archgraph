//! grimp 3.17 through the embedded grimp_dump.py helper.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use globset::GlobSet;
use serde::Deserialize;

use super::RawGraph;
use crate::model::{Collector, Edge, EdgeKind};
use crate::modules::join;

const DUMP: &str = include_str!("grimp_dump.py");

#[derive(Deserialize)]
struct Dump {
    grimp_version: String,
    modules: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    imports: Vec<Import>,
    #[serde(default)]
    import_records: Option<Vec<crate::model::ImportRecord>>,
    #[serde(default)]
    module_paths: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    barrel_modules: Vec<super::barrels::BarrelModule>,
}

#[derive(Deserialize)]
struct Import {
    importer: String,
    imported: String,
    /// `type` (only under `if TYPE_CHECKING:`), `dynamic` (only inside function bodies) or
    /// `static`; recordings older than the field are all `static`.
    #[serde(default = "static_kind")]
    kind: EdgeKind,
}

fn static_kind() -> EdgeKind {
    EdgeKind::Static
}

/// `explicit`, else the single package directly under `source_dir`: a directory with an
/// `__init__.py` or, for a namespace package, with `.py` files, and not excluded.
pub fn package(
    root: &Path,
    source_dir: &str,
    explicit: Option<&str>,
    excludes: &GlobSet,
) -> Result<String> {
    if let Some(package) = explicit {
        return Ok(package.to_owned());
    }
    let dir = root.join(source_dir);
    let mut found = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // An excluded directory excludes every file in it, so a probe file stands for it.
        if !path.is_dir()
            || name.starts_with('.')
            || excludes.is_match(join(source_dir, &format!("{name}/x.py")))
        {
            continue;
        }
        let mut pending = vec![path.clone()];
        let mut has_python = false;
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)?.filter_map(|entry| entry.ok()) {
                let kind = entry.file_type()?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if kind.is_file() && entry.path().extension().is_some_and(|x| x == "py") {
                    has_python = true;
                    break;
                }
                if kind.is_dir()
                    && !name.starts_with('.')
                    && !["node_modules", "vendor", "__pycache__"].contains(&name.as_ref())
                {
                    pending.push(entry.path());
                }
            }
            if has_python {
                break;
            }
        }
        if has_python {
            found.push(name);
        }
    }
    found.sort();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => bail!(
            "no Python package (directory with .py files) in {}; pass --package",
            dir.display()
        ),
        many => bail!(
            "several Python packages in {}: {}; pass --package",
            dir.display(),
            many.join(", ")
        ),
    }
}

/// `-I` keeps the environment, the working directory and user site-packages off `sys.path`; the
/// helper appends the source root itself after importing what it needs. `-B` writes no bytecode.
pub fn run(
    root: &Path,
    source_dir: &str,
    package: &str,
    python: &str,
    include_tests: bool,
    python_paths: &[PathBuf],
) -> Result<Vec<u8>> {
    let src = fs::canonicalize(root.join(source_dir))?;
    let python_paths = python_paths
        .iter()
        .map(|path| {
            let path = if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            };
            fs::canonicalize(&path).with_context(|| format!("--python-path {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let out = super::run(
        Command::new(python)
            .args(["-I", "-B", "-c", DUMP])
            .arg(src)
            .arg(package)
            .arg(fs::canonicalize(root)?)
            .args(include_tests.then_some("--include-tests"))
            .args(
                python_paths
                    .iter()
                    .flat_map(|path| [std::ffi::OsStr::new("--python-path"), path.as_os_str()]),
            ),
    )
    .with_context(|| format!("running {python}"))?;
    if !out.status.success() {
        bail!(
            "grimp helper failed: {}\ngrimp 3.17 is required: pip install grimp==3.17, \
             or run archgraph under uv run --with grimp==3.17",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Maps dotted module names to `a/b.py` or, for packages, `a/b/__init__.py`. A namespace
/// package (a directory without `__init__.py`) is no file and is left out.
pub fn parse(json: &[u8], root: &Path, source_dir: &str) -> Result<RawGraph> {
    let dump: Dump = serde_json::from_slice(json).context("reading grimp JSON")?;
    let path = |module: &str| {
        if let Some(path) = dump.module_paths.get(module) {
            return Some(path.clone());
        }
        let base = join(source_dir, &module.replace('.', "/"));
        let init = format!("{base}/__init__.py");
        let file = format!("{base}.py");
        if root.join(&init).is_file() {
            Some(init)
        } else if root.join(&base).is_dir() && !root.join(&file).is_file() {
            None
        } else {
            Some(file)
        }
    };
    let mut edges: Vec<Edge> = dump
        .imports
        .iter()
        .filter_map(|i| {
            Some(Edge {
                from: path(&i.importer)?,
                to: path(&i.imported)?,
                kind: i.kind,
                ..crate::model::Edge::default()
            })
        })
        .collect();
    if !dump.barrel_modules.is_empty() {
        let analyzed: std::collections::BTreeSet<_> = dump
            .barrel_modules
            .iter()
            .map(|module| module.file.as_str())
            .collect();
        edges.retain(|edge| !analyzed.contains(edge.from.as_str()));
        for module in &dump.barrel_modules {
            for route in &module.imports {
                edges.push(Edge {
                    from: module.file.clone(),
                    to: route.to.clone(),
                    kind: route.kind,
                    ..Edge::default()
                });
            }
        }
    }
    let mut files: Vec<String> = dump
        .modules
        .iter()
        .filter_map(|m| path(m))
        .chain(dump.files)
        .collect();
    files.sort();
    files.dedup();
    let collector = Collector {
        name: "grimp".to_owned(),
        version: Some(dump.grimp_version),
        cruised: files.len(),
        ..Default::default()
    };
    let mut graph = RawGraph {
        imports: dump.import_records,
        files,
        edges,
        collector,
        metadata: Default::default(),
    };
    for warning in super::barrels::resolve(&mut graph, &dump.barrel_modules) {
        eprintln!("warning: {warning}");
    }
    Ok(graph)
}
