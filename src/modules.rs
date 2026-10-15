//! Module assignment and barrels.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Edge, EdgeKind};

const TS_INDEX: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.js",
    "index.jsx",
    "index.mjs",
    "index.cjs",
];

/// The file's directory below `source_root`, truncated to `depth` segments; `.` for a file
/// directly in the source root. Files outside the source root count from the scan root and are
/// prefixed `../` (`..` for a file directly in the scan root), so they never merge with a module
/// inside it.
pub fn module_of(path: &str, source_root: &str, depth: usize) -> String {
    let (rel, outside) = match source_root {
        "." => (path, false),
        root => match path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
            Some(rel) => (rel, false),
            None => (path, true),
        },
    };
    let dirs: Vec<&str> = rel.split('/').collect();
    let dirs = &dirs[..dirs.len() - 1];
    let id = match &dirs[..dirs.len().min(depth)] {
        [] => ".".to_owned(),
        kept => kept.join("/"),
    };
    match (outside, id.as_str()) {
        (false, _) => id,
        (true, ".") => "..".to_owned(),
        (true, _) => format!("../{id}"),
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
                TS_INDEX.contains(&name) && out.iter().all(|e| e.kind == EdgeKind::Reexport)
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
