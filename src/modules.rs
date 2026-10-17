//! Module assignment and barrels.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Config, Edge, EdgeKind, Language};

pub const TS_INDEX: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.js",
    "index.jsx",
    "index.mjs",
    "index.cjs",
];

/// The file's directory below `source_root`, truncated to `depth` segments. A file directly in
/// the source root is its own module, named by its file name with extension (`main.ts`), so it
/// never merges with another root file or with a directory module; in Go the root's files are
/// one package, module `.`. Files outside the source root count from the scan root and are
/// prefixed `../` (`../lib`, `../tool.ts`), so they never merge with a module inside it.
/// With `config.package_modules` the directory is not truncated: each package is a module.
pub fn module_of(path: &str, config: &Config, language: Language) -> String {
    if !config.legacy_module_classes && config.granularity == crate::model::Granularity::File {
        return path.to_owned();
    }
    folder_of(path, config, language)
}

pub fn assigned(
    path: &str,
    config: &Config,
    language: Language,
    rules: Option<&crate::rules::Compiled>,
) -> String {
    if !config.legacy_module_classes && config.granularity == crate::model::Granularity::File {
        path.to_owned()
    } else {
        rules
            .and_then(|r| r.module_of(path))
            .unwrap_or_else(|| folder_of(path, config, language))
    }
}

/// Folder policy boundaries remain independent of graph granularity and test classes.
pub fn folder_of(path: &str, config: &Config, language: Language) -> String {
    let source_root = config.source_root.as_str();
    let depth = match config.package_modules {
        true => usize::MAX,
        false => config.depth,
    };
    let (rel, outside) = match source_root {
        "." => (path, false),
        root => match path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
            Some(rel) => (rel, false),
            None => (path, true),
        },
    };
    // `--depth 0` keeps every file below the root's top level in `.` (`..` outside it).
    let id = match rel.rsplit_once('/') {
        None if matches!(language, Language::Go | Language::Java) => String::new(),
        None => rel.to_owned(),
        Some(_) if depth == 0 => String::new(),
        Some((dir, _)) => dir.split('/').take(depth).collect::<Vec<_>>().join("/"),
    };
    match (outside, id.is_empty()) {
        (false, false) => id,
        (false, true) => ".".to_owned(),
        (true, false) => format!("../{id}"),
        (true, true) => "..".to_owned(),
    }
}

/// TS: an `index.*` whose imports are all re-exports. Python: an `__init__.py` that imports only
/// its own submodules. Either needs at least one import.
pub fn barrels(edges: &[Edge]) -> BTreeSet<String> {
    let mut outgoing: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    for e in edges {
        outgoing.entry(&e.from).or_default().push(e);
    }
    outgoing
        .into_iter()
        .filter(|(file, out)| {
            let (dir, name) = file.rsplit_once('/').map_or(("", *file), |(d, n)| (d, n));
            if name == "__init__.py" {
                let own = format!("{dir}/");
                out.iter().all(|e| e.to.starts_with(&own))
            } else {
                TS_INDEX.contains(&name)
                    && out.iter().all(|e| {
                        e.is_reexport
                            || matches!(e.kind, EdgeKind::Reexport | EdgeKind::TypeReexport)
                    })
            }
        })
        .map(|(file, _)| file.to_owned())
        .collect()
}

/// File pairs for metrics: an import of a barrel counts as an import of everything it
/// re-exports (through nested barrels), and the barrels' own imports are dropped.
pub fn attribute(edges: &[Edge], barrels: &BTreeSet<String>) -> BTreeSet<(String, String)> {
    let mut targets: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in edges.iter().filter(|e| barrels.contains(&e.from)) {
        targets.entry(&e.from).or_default().push(&e.to);
    }
    let mut pairs = BTreeSet::new();
    for e in edges.iter().filter(|e| !barrels.contains(&e.from)) {
        let mut stack = vec![e.to.as_str()];
        let mut seen = BTreeSet::new();
        while let Some(file) = stack.pop() {
            if !seen.insert(file) {
                continue;
            }
            match targets.get(file) {
                Some(next) => stack.extend(next),
                None if file != e.from => {
                    pairs.insert((e.from.clone(), file.to_owned()));
                }
                None => {}
            }
        }
    }
    pairs
}
