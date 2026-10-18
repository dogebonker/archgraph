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

/// Whether a file name is a module's index: `__init__.py` or one of `TS_INDEX`.
pub fn is_index(name: &str) -> bool {
    name == "__init__.py" || TS_INDEX.contains(&name)
}

/// Human-readable grouping evidence shared by reports and agent explanations.
pub fn grouping(config: &Config) -> String {
    let mode = if config.granularity == crate::model::Granularity::File {
        "file granularity".to_owned()
    } else {
        let fallback = if config.package_modules {
            "package directories".to_owned()
        } else {
            format!("folder depth {}", config.depth)
        };
        if config
            .rules
            .as_ref()
            .is_some_and(|r| !r.doc.modules.is_empty())
        {
            format!("layout modules; fallback {fallback}")
        } else {
            fallback
        }
    };
    format!("{mode} under {}", config.source_root)
}

pub fn dependency_semantics(language: Language) -> &'static str {
    if language == Language::Rust {
        "rust-source-references"
    } else {
        "import-dependencies"
    }
}

/// Whether `path` is a test file, by the collector's metadata or by its path.
pub fn is_test(path: &str, config: &Config) -> bool {
    config.file_metadata.get(path).is_some_and(|m| m.test)
        || crate::collect::excludes::is_test(path)
}

/// Public entries for cycle cuts: what a package.json publishes and the source root's index
/// (`index.ts`, a Python package's `__init__.py`). Other barrels are internal.
pub fn cycle_entries<'a>(
    paths: impl IntoIterator<Item = &'a String>,
    source_root: &str,
    package_entries: &'a [String],
) -> BTreeSet<String> {
    let root_index = |f: &str| {
        let name = match source_root {
            "." => Some(f),
            root => f.strip_prefix(root).and_then(|r| r.strip_prefix('/')),
        };
        name.is_some_and(is_index)
    };
    paths
        .into_iter()
        .filter(|f| root_index(f))
        .chain(package_entries)
        .cloned()
        .collect()
}

/// The directory of `path`, `""` for a file in the scan root.
pub fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `name` in `dir`, where `""` and `.` both mean the scan root.
pub fn join(dir: &str, name: &str) -> String {
    match dir {
        "" | "." => name.to_owned(),
        dir => format!("{dir}/{name}"),
    }
}

/// The file's directory below `source_root`, truncated to `depth` segments. A file directly in
/// the source root is its own module, named by its file name with extension (`main.ts`), so it
/// never merges with another root file or with a directory module; in Go the root's files are
/// one package, module `.`. Files outside the source root count from the scan root and are
/// prefixed `../` (`../lib`, `../tool.ts`), so they never merge with a module inside it.
/// With `config.package_modules` the directory is not truncated: each package is a module.
/// File granularity makes each file its module, and `rules` modules take precedence.
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

/// `assigned` for a scanned file, with test files in their own `tests::` module.
pub fn assigned_file(
    file: &crate::model::File,
    config: &Config,
    language: Language,
    rules: Option<&crate::rules::Compiled>,
) -> String {
    let module = assigned(&file.path, config, language, rules);
    if file.class == Some(crate::model::ModuleClass::Tests) {
        format!("tests::{module}")
    } else {
        module
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
pub fn attribute<'a>(
    edges: impl IntoIterator<Item = &'a Edge>,
    barrels: &BTreeSet<String>,
) -> BTreeSet<(String, String)> {
    weighted(edges, barrels).into_keys().collect()
}

/// `attribute`'s pairs, each with the heaviest weight of the imports it stands for.
pub fn weighted<'a>(
    edges: impl IntoIterator<Item = &'a Edge>,
    barrels: &BTreeSet<String>,
) -> BTreeMap<(String, String), f64> {
    let (from_barrels, others): (Vec<&Edge>, Vec<&Edge>) =
        edges.into_iter().partition(|e| barrels.contains(&e.from));
    let mut targets: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in from_barrels {
        targets.entry(&e.from).or_default().push(&e.to);
    }
    let mut pairs: BTreeMap<(String, String), f64> = BTreeMap::new();
    for e in others {
        let weight = e
            .weight
            .unwrap_or_else(|| crate::rules::EdgeWeights::default().weight(e.kind));
        let mut stack = vec![e.to.as_str()];
        let mut seen = BTreeSet::new();
        while let Some(file) = stack.pop() {
            if !seen.insert(file) {
                continue;
            }
            match targets.get(file) {
                Some(next) => stack.extend(next),
                None if file != e.from => {
                    let pair = pairs.entry((e.from.clone(), file.to_owned())).or_default();
                    *pair = pair.max(weight);
                }
                None => {}
            }
        }
    }
    pairs
}
