//! Collectors turn a tool's output into files and file edges inside the scanned root.

pub mod depcruise;
pub mod grimp;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::model::{Collector, Edge, Language};

pub struct RawGraph {
    pub files: Vec<String>,
    pub edges: Vec<Edge>,
    pub collector: Collector,
}

/// Extensions dependency-cruiser files must have to count as source; imports of anything else
/// (stylesheets, JSON, images) are dropped.
pub const TS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

pub const DEFAULT_EXCLUDES: &[&str] = &[
    "**/generated/**",
    "**/__generated__/**",
    "**/*.generated.*",
    "**/*.gen.*",
    "**/*_pb2.py",
    "**/*_pb2_grpc.py",
    "**/*.d.ts",
    "**/node_modules/**",
    "**/dist/**",
    "**/build/**",
];

pub const TEST_EXCLUDES: &[&str] = &[
    "**/*.test.*",
    "**/*.spec.*",
    "**/__tests__/**",
    "**/test_*.py",
    "**/*_test.py",
    "**/conftest.py",
    "**/tests/**",
];

impl RawGraph {
    /// Drops excluded files and every edge that does not join two kept files.
    pub fn exclude(mut self, excludes: &GlobSet) -> RawGraph {
        self.files.retain(|f| !excludes.is_match(f));
        let kept: HashSet<&str> = self.files.iter().map(String::as_str).collect();
        self.edges
            .retain(|e| kept.contains(e.from.as_str()) && kept.contains(e.to.as_str()));
        self
    }
}

/// `*` stays within one path segment, `**` crosses any number.
pub fn globset(globs: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for glob in globs {
        set.add(
            GlobBuilder::new(glob)
                .literal_separator(true)
                .build()
                .with_context(|| format!("glob {glob}"))?,
        );
    }
    Ok(set.build()?)
}

/// `src` when the root has one, else the root itself.
pub fn source_dir(root: &Path) -> &'static str {
    if root.join("src").is_dir() {
        "src"
    } else {
        "."
    }
}

pub fn join(dir: &str, rest: &str) -> String {
    if dir == "." {
        rest.to_owned()
    } else {
        format!("{dir}/{rest}")
    }
}

/// The language with more source files under `source_dir`, after the default excludes.
// ponytail: hidden directories and node_modules are not descended; add a flag if a repository
// keeps sources in a dot-directory.
pub fn detect_language(root: &Path, source_dir: &str) -> Result<Language> {
    let defaults: Vec<String> = DEFAULT_EXCLUDES
        .iter()
        .chain(TEST_EXCLUDES)
        .map(|g| g.to_string())
        .collect();
    let defaults = globset(&defaults)?;
    let (mut ts, mut py) = (0, 0);
    let mut dirs = vec![root.join(source_dir)];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() {
                if !name.starts_with('.') && name != "node_modules" {
                    dirs.push(entry.path());
                }
                continue;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if defaults.is_match(&rel) {
                continue;
            }
            match name.rsplit_once('.').map(|(_, ext)| ext) {
                Some("py") => py += 1,
                Some(ext) if TS_EXTENSIONS.contains(&ext) => ts += 1,
                _ => {}
            }
        }
    }
    match (ts, py) {
        (0, 0) => bail!(
            "no .ts/.js or .py files under {}; pass --language",
            root.join(source_dir).display()
        ),
        _ if py > ts => Ok(Language::Python),
        _ => Ok(Language::Typescript),
    }
}
