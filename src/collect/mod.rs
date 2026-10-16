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
    // Build output at the scanned root. Deeper `build/` and `dist/` folders are often source
    // (a bundler's build step); they are dropped only when git ignores them (OUTPUT_DIRS).
    "dist/**",
    "build/**",
];

/// Build output folders below the root, dropped with the default excludes only when git
/// ignores them.
pub const OUTPUT_DIRS: &[&str] = &["**/dist/**", "**/build/**"];

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
    /// Drops files under `OUTPUT_DIRS` that git ignores in `root`'s repository (nothing outside
    /// a repository, or when git fails). Tracked files are never ignored.
    pub fn exclude_ignored_output(self, root: &Path) -> Result<RawGraph> {
        let output = globset(
            &OUTPUT_DIRS
                .iter()
                .map(|g| g.to_string())
                .collect::<Vec<_>>(),
        )?;
        let candidates: Vec<&str> = self
            .files
            .iter()
            .map(String::as_str)
            .filter(|f| output.is_match(f))
            .collect();
        if candidates.is_empty() {
            return Ok(self);
        }
        let ignored: HashSet<String> = git_ignored(root, &candidates).into_iter().collect();
        let mut kept = self;
        kept.files.retain(|f| !ignored.contains(f));
        kept.edges
            .retain(|e| !ignored.contains(&e.from) && !ignored.contains(&e.to));
        Ok(kept)
    }

    /// Drops excluded files and every edge that does not join two kept files.
    pub fn exclude(mut self, excludes: &GlobSet) -> RawGraph {
        self.files.retain(|f| !excludes.is_match(f));
        let kept: HashSet<&str> = self.files.iter().map(String::as_str).collect();
        self.edges
            .retain(|e| kept.contains(e.from.as_str()) && kept.contains(e.to.as_str()));
        self
    }
}

/// The `paths` (relative to `root`) that git ignores, by `git check-ignore`: empty outside a
/// repository or when git fails.
fn git_ignored(root: &Path, paths: &[&str]) -> Vec<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let child = Command::new("git")
        .arg("-C")
        .arg(root)
        // A repository's core.fsmonitor names a program git would run.
        .args([
            "-c",
            "core.fsmonitor=false",
            "check-ignore",
            "--stdin",
            "-z",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(paths.join("\0").as_bytes());
        let _ = stdin.write_all(b"\0");
    }
    // Exit 1 means none is ignored, 128 not a repository.
    let Ok(out) = child.wait_with_output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
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
