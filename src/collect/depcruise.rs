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
    #[serde(default)]
    module: Option<String>,
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

/// Runs depcruise in `root`: `program`, else `npx --no-install depcruise`.
/// The options `run` passes, as a dependency-cruiser config (the command line cannot set
/// `tsPreCompilationDeps: "specify"`). `"specify"` keeps imports the TypeScript compiler drops
/// (`import type`, and imports of types and interfaces only) and marks them
/// `pre-compilation-only`; `tsConfig` resolves path aliases such as `@/`. The scanned
/// repository's own config is never read. tests/fixtures/record.sh writes the same options.
pub fn options(ts_config: bool, pre_compilation: &str) -> String {
    let ts = if ts_config {
        r#", "tsConfig": { "fileName": "tsconfig.json" }"#
    } else {
        ""
    };
    format!(
        r#"{{ "options": {{ "doNotFollow": {{ "path": "node_modules" }}, "tsPreCompilationDeps": {pre_compilation}{ts} }} }}"#
    )
}

/// One depcruise run with `options(…, pre_compilation)`: its JSON, or its error output.
fn cruise(
    root: &Path,
    source_dir: &str,
    program: Option<&Path>,
    pre_compilation: &str,
) -> Result<std::result::Result<Vec<u8>, String>> {
    let mut cmd = match program {
        Some(program) => Command::new(program),
        None => {
            let mut npx = Command::new("npx");
            npx.args(["--no-install", "depcruise"]);
            npx
        }
    };
    // Written outside the scanned repository, removed after the run.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let config = std::env::temp_dir().join(format!(
        "archgraph-depcruise-{}-{nanos}.json",
        std::process::id()
    ));
    let ts_config = root.join("tsconfig.json").is_file();
    std::fs::write(&config, options(ts_config, pre_compilation))
        .with_context(|| format!("writing {}", config.display()))?;
    cmd.current_dir(root)
        .args(["--output-type", "json", "--config"])
        .arg(&config);
    let running = program.map_or("npx (is Node.js installed?)".into(), |p| {
        p.display().to_string()
    });
    let out = cmd.arg(source_dir).output();
    let _ = std::fs::remove_file(&config);
    let out = out.with_context(|| format!("running {running}"))?;
    Ok(if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    })
}

/// dependency-cruiser's JSON with every dependency the compiler drops marked
/// `pre-compilation-only`. One run with `"specify"`; when that run fails (dependency-cruiser
/// 18.4.0 crashes on some sources in that mode), two: with and without pre-compilation
/// dependencies, the ones only the first has marked; when the second fails too, the first
/// unmarked, with a warning, and `Some("unavailable")` for `source.collector.type_marking`.
pub fn run(
    root: &Path,
    source_dir: &str,
    program: Option<&Path>,
) -> Result<(Vec<u8>, Option<&'static str>)> {
    let failed = |stderr: String| {
        let hint = match program {
            Some(_) => {
                "--depcruise must be dependency-cruiser 18.4.0 with typescript 6.0.3 \
                        installed beside it"
            }
            None => {
                "install the pinned tools in the scanned repository: \
                     npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3"
            }
        };
        anyhow::anyhow!("depcruise failed: {stderr}\n{hint}")
    };
    if let Ok(json) = cruise(root, source_dir, program, r#""specify""#)? {
        return Ok((json, None));
    }
    let full = cruise(root, source_dir, program, "true")?.map_err(failed)?;
    match cruise(root, source_dir, program, "false")? {
        Ok(compiled) => Ok((mark_pre_compilation(&full, &compiled)?, None)),
        Err(stderr) => {
            eprintln!(
                "warning: dependency-cruiser failed without pre-compilation dependencies ({}); \
                 imports of types only, written without `import type`, count as `static`",
                stderr
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
            );
            Ok((full, Some("unavailable")))
        }
    }
}

/// `full` with each dependency absent from `compiled` marked `pre-compilation-only`.
fn mark_pre_compilation(full: &[u8], compiled: &[u8]) -> Result<Vec<u8>> {
    let pairs = |doc: &serde_json::Value| -> std::collections::BTreeSet<(String, String)> {
        let mut set = std::collections::BTreeSet::new();
        for m in doc["modules"].as_array().into_iter().flatten() {
            for d in m["dependencies"].as_array().into_iter().flatten() {
                if let (Some(s), Some(r)) = (m["source"].as_str(), d["resolved"].as_str()) {
                    set.insert((s.to_owned(), r.to_owned()));
                }
            }
        }
        set
    };
    let mut doc: serde_json::Value =
        serde_json::from_slice(full).context("reading dependency-cruiser JSON")?;
    let kept = pairs(&serde_json::from_slice(compiled).context("reading dependency-cruiser JSON")?);
    for m in doc["modules"].as_array_mut().into_iter().flatten() {
        let source = m["source"].as_str().unwrap_or_default().to_owned();
        for d in m["dependencies"].as_array_mut().into_iter().flatten() {
            let resolved = d["resolved"].as_str().unwrap_or_default().to_owned();
            if !kept.contains(&(source.clone(), resolved))
                && let Some(types) = d["dependencyTypes"].as_array_mut()
            {
                types.push("pre-compilation-only".into());
            }
        }
    }
    Ok(serde_json::to_vec(&doc)?)
}

/// Refine load timing only for dependencies confirmed by dependency-cruiser.
pub fn parse_at(json: &[u8], root: &Path) -> Result<RawGraph> {
    let mut graph = parse(json)?;
    let out: Output = serde_json::from_slice(json)?;
    let Ok(root) = root.canonicalize() else {
        return Ok(graph);
    };
    let mut lazy = BTreeMap::new();
    for module in &out.modules {
        let Ok(path) = root.join(&module.source).canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let imports = super::imports::typescript(&source);
        for dependency in &module.dependencies {
            let all_lazy = dependency.module.as_deref().is_some_and(|specifier| {
                let mut matches = imports
                    .iter()
                    .filter(|i| i.specifier.as_deref() == Some(specifier))
                    .peekable();
                matches.peek().is_some() && matches.all(|i| i.kind == EdgeKind::Dynamic)
            });
            let slot = lazy
                .entry((module.source.as_str(), dependency.resolved.as_str()))
                .or_insert(true);
            *slot &= all_lazy;
        }
    }
    for edge in &mut graph.edges {
        if edge.kind == EdgeKind::Static
            && lazy.get(&(edge.from.as_str(), edge.to.as_str())) == Some(&true)
        {
            edge.kind = EdgeKind::Dynamic;
        }
    }
    Ok(graph)
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
            // `export type … from` is reported as type-only and export: a re-export, erased.
            // `pre-compilation-only` is an import of types only, without `import type`: the
            // compiler drops it just the same.
            let erased = has("type-only") || has("pre-compilation-only");
            let kind = if has("export") && erased {
                EdgeKind::TypeReexport
            } else if has("export") {
                EdgeKind::Reexport
            } else if erased {
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
            ..crate::model::Edge::default()
        })
        .collect();
    let collector = Collector {
        name: "dependency-cruiser".to_owned(),
        version: out.summary.environment.map(|e| e.version),
        cruised: out.summary.total_cruised,
        unresolved: Some(unresolved),
        type_marking: None,
        main_packages: Vec::new(),
        bound_files: Vec::new(),
        go_modules: Vec::new(),
    };
    Ok(RawGraph {
        files,
        edges,
        collector,
    })
}

/// Build output folders a package.json points into; the sources sit without them.
const OUTPUT_DIRS: &[&str] = &["dist", "build", "lib", "out", "esm", "cjs"];
/// Output extensions, longest first, stripped to find the source a package.json path is built
/// from.
const OUTPUT_EXTENSIONS: &[&str] = &[
    ".d.mts", ".d.cts", ".d.ts", ".mjs", ".cjs", ".mts", ".cts", ".jsx", ".tsx", ".js", ".ts",
];

/// The scanned files that the package.json files under `root` publish through `exports`, `main`,
/// `module` and `types`: each path mapped from build output back to its source (output folder,
/// extension and a `src/` prefix tried), else to the one scanned file of the package with the
/// same path stem. node_modules and hidden folders are skipped, symbolic links not followed.
pub fn package_entries(root: &Path, files: &[String]) -> Vec<String> {
    let scanned: std::collections::BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let mut found = std::collections::BTreeSet::new();
    let mut dirs = vec![".".to_owned()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && name != "node_modules" && !name.starts_with('.') {
                dirs.push(super::join(&dir, &name));
            } else if kind.is_file() && name == "package.json" {
                let Some(json) = std::fs::read(entry.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                else {
                    continue;
                };
                let mut targets = Vec::new();
                for key in ["main", "module", "types", "typings", "exports"] {
                    strings(&json[key], &mut targets);
                }
                for t in targets {
                    if let Some(file) = source_of(&dir, &t, &scanned) {
                        found.insert(file.to_owned());
                    }
                }
            }
        }
    }
    found.into_iter().collect()
}

/// Every string in a package.json field: `exports` nests subpaths and conditions.
fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(a) => a.iter().for_each(|v| strings(v, out)),
        serde_json::Value::Object(o) => o.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

/// The scanned file a package path (relative to the package folder `dir`) is built from.
fn source_of<'a>(
    dir: &str,
    target: &str,
    scanned: &std::collections::BTreeSet<&'a str>,
) -> Option<&'a str> {
    let target = target.strip_prefix("./").unwrap_or(target);
    if target.contains('*') || target.ends_with(".json") || target.starts_with(['/', '.']) {
        return None;
    }
    let mut stem = OUTPUT_EXTENSIONS
        .iter()
        .find_map(|ext| target.strip_suffix(ext))
        .unwrap_or(target);
    while let Some((first, rest)) = stem.split_once('/') {
        if !OUTPUT_DIRS.contains(&first) {
            break;
        }
        stem = rest;
    }
    for base in [
        super::join(dir, stem),
        super::join(dir, &format!("src/{stem}")),
    ] {
        for ext in TS_EXTENSIONS {
            if let Some(file) = scanned.get(format!("{base}.{ext}").as_str()) {
                return Some(file);
            }
        }
    }
    // The same stem anywhere in the package, when only one file has it.
    let tail = format!("/{stem}");
    let prefix = if dir == "." {
        String::new()
    } else {
        format!("{dir}/")
    };
    let mut same = scanned.iter().filter(|f| {
        f.strip_prefix(prefix.as_str()).is_some_and(|rel| {
            let path = format!("/{rel}");
            path.rsplit_once('.')
                .is_some_and(|(p, ext)| TS_EXTENSIONS.contains(&ext) && p.ends_with(&tail))
        })
    });
    match (same.next(), same.next()) {
        (Some(file), None) => Some(file),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::mark_pre_compilation;

    #[test]
    fn a_dependency_only_the_pre_compilation_run_has_is_marked() {
        let dep =
            |r: &str| serde_json::json!({ "resolved": r, "dependencyTypes": ["local", "import"] });
        let doc = |deps: Vec<serde_json::Value>| {
            serde_json::json!({ "modules": [{ "source": "a.ts", "dependencies": deps }] })
                .to_string()
        };
        let full = doc(vec![dep("t.ts"), dep("v.ts")]);
        let compiled = doc(vec![dep("v.ts")]);
        let marked: serde_json::Value = serde_json::from_slice(
            &mark_pre_compilation(full.as_bytes(), compiled.as_bytes()).unwrap(),
        )
        .unwrap();
        let types = |i: usize| marked["modules"][0]["dependencies"][i]["dependencyTypes"].clone();
        assert_eq!(
            types(0),
            serde_json::json!(["local", "import", "pre-compilation-only"])
        );
        assert_eq!(types(1), serde_json::json!(["local", "import"]));
    }
}
