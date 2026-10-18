//! Go through the embedded go_dump.go helper, run with the user's Go toolchain (1.21 or later).

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use globset::GlobSet;
use serde::Deserialize;

use super::RawGraph;
use crate::model::{Collector, Edge, EdgeKind};

const DUMP: &str = include_str!("go_dump.go");

/// The environment of `go run` and of the helper's `go list`: the local toolchain only (a
/// go.mod `toolchain` line is ignored, a `go` line newer than the toolchain fails), no network,
/// no go.work, no cgo, and GOFLAGS replaced, so no `-toolexec` from the user's go env applies
/// and go.mod and go.sum are never rewritten. tests/fixtures/record.sh uses the same.
pub const ENV: &[(&str, &str)] = &[
    ("GOTOOLCHAIN", "local"),
    ("GOFLAGS", "-mod=readonly"),
    ("CGO_ENABLED", "0"),
    ("GOWORK", "off"),
    ("GOPROXY", "off"),
];

#[derive(Deserialize)]
struct Dump {
    go_version: String,
    files: Vec<DumpFile>,
    edges: Vec<DumpEdge>,
    unresolved: usize,
    #[serde(default)]
    imports: Option<Vec<crate::model::ImportRecord>>,
    #[serde(default)]
    modules: Vec<String>,
    /// Pairs of files of one package that cannot be split across packages and still compile.
    #[serde(default)]
    bound: Vec<DumpEdge>,
}

#[derive(Deserialize)]
struct DumpFile {
    path: String,
    generated: bool,
    #[serde(default)]
    main: bool,
    #[serde(default)]
    test_support: bool,
}

#[derive(Deserialize)]
struct DumpEdge {
    from: String,
    to: String,
}

/// The directories under `root` holding a go.mod with a `module` line, root-relative (`.` for
/// the root) and sorted, outside what the go command skips (`vendor`, `testdata`, names starting
/// with `.` or `_`), node_modules and the `excludes` (matched against `<dir>/go.mod`); and one
/// warning per go.mod skipped for having no module line.
pub fn module_dirs(root: &Path, excludes: &GlobSet) -> Result<(Vec<String>, Vec<String>)> {
    let (mut dirs, mut warnings) = (Vec::new(), Vec::new());
    let mut stack = vec![".".to_owned()];
    while let Some(rel) = stack.pop() {
        let dir = root.join(&rel);
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = crate::modules::join(&rel, &name);
            if entry.file_type()?.is_dir() {
                if !name.starts_with(['.', '_'])
                    && !["vendor", "testdata", "node_modules"].contains(&name.as_str())
                {
                    stack.push(path);
                }
            } else if name == "go.mod" && !excludes.is_match(&path) {
                let text = fs::read_to_string(entry.path()).unwrap_or_default();
                let module = text.lines().any(|l| {
                    l.trim()
                        .strip_prefix("module")
                        .is_some_and(|r| r.starts_with([' ', '\t', '"']))
                });
                if module {
                    dirs.push(rel.clone());
                } else {
                    warnings.push(format!("skipping {path}: it has no module line"));
                }
            }
        }
    }
    dirs.sort();
    warnings.sort();
    Ok((dirs, warnings))
}

/// Runs the helper on the modules in `dirs` (from `module_dirs`) from a temporary directory
/// outside the scanned repository, which `go run` compiles alone: no repository code is compiled
/// or run. The helper's warnings (a module read with go/build) are printed.
pub fn run(root: &Path, go: &str, dirs: &[String]) -> Result<Vec<u8>> {
    let root = fs::canonicalize(root)?;
    // A path with a separator is resolved here, not against the helper's working directory.
    let go = if go.contains(std::path::MAIN_SEPARATOR) || go.contains('/') {
        fs::canonicalize(go)
            .with_context(|| format!("--go {go}: no such executable"))?
            .to_string_lossy()
            .into_owned()
    } else {
        go.to_owned()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("archgraph-go-{}-{nanos}", std::process::id()));
    fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let out = fs::write(dir.join("go_dump.go"), DUMP).and_then(|()| {
        Command::new(&go)
            .args(["run", "go_dump.go"])
            .arg(&root)
            .arg(&go)
            .args(dirs)
            .current_dir(&dir)
            .env("PWD", &dir)
            .envs(ENV.iter().copied())
            .output()
    });
    let _ = fs::remove_dir_all(&dir);
    let out = out.with_context(|| {
        format!("running {go} (install Go from https://go.dev/dl, or pass --go)")
    })?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        // The helper compiles with Go 1.21 or later; a compile error names its own file.
        let hint = if stderr.contains("requires go >=") {
            "\na go.mod asks for a newer Go than this toolchain, and archgraph never downloads one \
             (GOTOOLCHAIN=local): install that version from https://go.dev/dl and pass --go <path>"
        } else if stderr.contains("go_dump.go:") {
            "\narchgraph's Go helper did not compile with this Go: it needs Go 1.21 or later \
             (tested with 1.27.1, https://go.dev/dl); pass --go <path> to pick one"
        } else {
            ""
        };
        bail!("Go helper failed: {}{hint}", stderr.trim());
    }
    for line in stderr.lines().filter(|l| l.starts_with("warning:")) {
        eprintln!("{line}");
    }
    Ok(out.stdout)
}

/// What `parse` reads besides the graph.
pub struct Parsed {
    pub raw: RawGraph,
    pub generated: Vec<String>,
    pub test_support: Vec<String>,
    /// Pairs of files that cannot be split across packages, for `bound_files` once excludes ran.
    pub bound: Vec<(String, String)>,
    /// The test support left out without --include-tests, sorted: `dir/` for a package left out
    /// whole, else each file.
    pub left_out: Vec<String>,
}

/// Files and file references, generated files (a `Code generated ... DO NOT EDIT.` header) and
/// test support (see the helper's `testSupport`) included. Every edge is `static`: Go has no
/// load order, and the edges are references, not import statements.
pub fn parse(json: &[u8]) -> Result<Parsed> {
    let dump: Dump = serde_json::from_slice(json).context("reading Go helper JSON")?;
    let generated = dump
        .files
        .iter()
        .filter(|f| f.generated)
        .map(|f| f.path.clone())
        .collect();
    let test_support = dump
        .files
        .iter()
        .filter(|f| f.test_support)
        .map(|f| f.path.clone())
        .collect();
    // Per directory: its non-test files, and those that are test support.
    let mut by_dir: std::collections::BTreeMap<&str, (usize, Vec<&str>)> = Default::default();
    for f in dump.files.iter().filter(|f| !f.path.ends_with("_test.go")) {
        let dir = by_dir
            .entry(f.path.rsplit_once('/').map_or(".", |(d, _)| d))
            .or_default();
        dir.0 += 1;
        if f.test_support {
            dir.1.push(&f.path);
        }
    }
    let mut left_out: Vec<String> = Vec::new();
    for (dir, (total, support)) in by_dir.iter().filter(|(_, (_, s))| !s.is_empty()) {
        if support.len() == *total {
            left_out.push(format!("{dir}/"));
        } else {
            left_out.extend(support.iter().map(|f| f.to_string()));
        }
    }
    left_out.sort();
    let mut main_packages: Vec<String> = dump
        .files
        .iter()
        .filter(|f| f.main)
        .map(|f| f.path.rsplit_once('/').map_or(".", |(d, _)| d).to_owned())
        .collect();
    main_packages.sort();
    main_packages.dedup();
    let collector = Collector {
        name: "go-types".to_owned(),
        version: Some(dump.go_version),
        cruised: dump.files.len(),
        unresolved: Some(dump.unresolved),
        main_packages,
        go_modules: dump.modules,
        ..Default::default()
    };
    let edges = dump
        .edges
        .into_iter()
        .map(|e| Edge {
            from: e.from,
            to: e.to,
            kind: EdgeKind::Static,
            ..crate::model::Edge::default()
        })
        .collect();
    let raw = RawGraph {
        imports: dump.imports,
        metadata: dump
            .files
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    crate::model::NodeMetadata {
                        test: f.test_support,
                        ..Default::default()
                    },
                )
            })
            .collect(),
        files: dump.files.into_iter().map(|f| f.path).collect(),
        edges,
        collector,
    };
    let bound = dump.bound.into_iter().map(|e| (e.from, e.to)).collect();
    Ok(Parsed {
        raw,
        generated,
        test_support,
        bound,
        left_out,
    })
}

/// The files of the `bound` pairs whose two files are both in `files`, sorted: a file bound only
/// to an excluded one (a test, generated code) moves freely in the graph that is analyzed.
pub fn bound_files(bound: &[(String, String)], files: &[String]) -> Vec<String> {
    let kept: std::collections::HashSet<&str> = files.iter().map(String::as_str).collect();
    let mut out: Vec<String> = bound
        .iter()
        .filter(|(a, b)| kept.contains(a.as_str()) && kept.contains(b.as_str()))
        .flat_map(|(a, b)| [a.clone(), b.clone()])
        .collect();
    out.sort();
    out.dedup();
    out
}
