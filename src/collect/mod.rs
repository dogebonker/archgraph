//! Collectors turn a tool's output into files and file edges inside the scanned root.

pub mod barrels;
pub mod depcruise;
pub mod excludes;
pub mod go;
pub mod grimp;
pub mod imports;
pub mod java;
pub mod rust;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::model::{Collector, Edge, Language, NodeMetadata};

pub struct RawGraph {
    pub files: Vec<String>,
    pub edges: Vec<Edge>,
    pub collector: Collector,
    pub metadata: std::collections::BTreeMap<String, NodeMetadata>,
}

/// Extensions dependency-cruiser files must have to count as source; imports of anything else
/// (stylesheets, JSON, images) are dropped.
pub const TS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

// Kept for callers constructing collector-specific discovery globs.
pub use excludes::{GENERATED_EXCLUDES as DEFAULT_EXCLUDES, TEST_EXCLUDES};

/// Files a root marker makes count for Go in language detection.
const GO_MARKERS: &[&str] = &["go.mod", "go.work"];

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

/// The language with the most source files after the default excludes: TypeScript and Python
/// under `source_dir`, Java/Rust/Go under the root. Rust and Go require their root manifests.
/// A tie keeps TypeScript, then Python, Go, Rust and Java.
pub fn detect_language(root: &Path, source_dir: &str) -> Result<Language> {
    detect_language_with_options(root, source_dir, excludes::Options::default())
}

pub fn detect_language_with_options(
    root: &Path,
    source_dir: &str,
    options: excludes::Options,
) -> Result<Language> {
    let generated = options.no_defaults || options.include_generated;
    let has_includes = !options.includes.is_empty();
    let java_output = globset(&["**/target/**".into(), "**/out/**".into()])?;
    let rust_output = globset(&["**/target/**".into()])?;
    let policy = excludes::Policy::new(root, Language::Typescript, options)?;
    let discovery_excludes = globset(policy.patterns())?;
    let cargo = root.join("Cargo.toml").is_file();
    let (mut ts, mut py, mut rust, mut java, mut go) = (0, 0, 0, 0, 0);
    let mut paths = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() {
                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(&entry.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if name != ".git"
                    && (has_includes || !discovery_excludes.is_match(format!("{rel}/_")))
                {
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
            paths.push(rel);
        }
    }
    let has_go = GO_MARKERS.iter().any(|m| root.join(m).is_file());
    for rel in policy.select(&paths)? {
        let path = root.join(&rel);
        match rel.rsplit_once('.').map(|(_, ext)| ext) {
            Some("py") if path.starts_with(root.join(source_dir)) => py += 1,
            Some("rs")
                if cargo && (generated || policy.included(&rel) || !rust_output.is_match(&rel)) =>
            {
                rust += 1
            }
            Some("java") if generated || policy.included(&rel) || !java_output.is_match(&rel) => {
                java += 1
            }
            Some("go")
                if has_go
                    && !rel
                        .split('/')
                        .any(|segment| segment.starts_with('_') || segment == "testdata") =>
            {
                go += 1
            }
            Some(ext)
                if TS_EXTENSIONS.contains(&ext) && path.starts_with(root.join(source_dir)) =>
            {
                ts += 1
            }
            _ => {}
        }
    }
    match (ts, py, go, rust, java) {
        (0, 0, 0, 0, 0) => bail!(
            "no .ts/.js, .py, .java, .rs (with Cargo.toml) or .go (with go.mod) files under {}; pass --language",
            root.join(source_dir).display()
        ),
        _ if java > ts && java > py && java > go && java > rust => Ok(Language::Java),
        _ if rust > ts && rust > py && rust > go => Ok(Language::Rust),
        _ if go > ts && go > py => Ok(Language::Go),
        _ if py > ts => Ok(Language::Python),
        _ => Ok(Language::Typescript),
    }
}
