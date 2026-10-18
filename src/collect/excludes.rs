//! Shared source-selection policy. Explicit excludes win; includes restore defaults/ignore matches.
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use globset::GlobSet;

use super::globset;
use crate::model::Language;

pub const TEST_EXCLUDES: &[&str] = &[
    "**/*_test.go",
    "**/*_test.exs",
    "**/test_helper.exs",
    "**/*.test.*",
    "**/*.spec.*",
    "**/tests/**",
    "**/test/**",
    "**/__tests__/**",
    "**/test_*.py",
    "**/*_test.py",
    "**/conftest.py",
    "**/*Test.java",
    "**/*Tests.java",
];
pub const GENERATED_EXCLUDES: &[&str] = &[
    "**/generated/**",
    "**/__generated__/**",
    "**/*.generated.*",
    "**/*.gen.*",
    "**/*_pb.*",
    "**/*_pb2.py",
    "**/*_pb2_grpc.py",
    "**/*.pb.go",
    "**/*.pb.gw.go",
    "**/zz_generated*.go",
    "**/*.d.ts",
    "**/dist/**",
    "**/build/**",
    "**/node_modules/**",
    "**/vendor/**",
    "**/.venv/**",
];
pub const SUPPORT_EXCLUDES: &[&str] = &[
    "**/testdata/**",
    "**/fixtures/**",
    "**/fixture/**",
    "**/__fixtures__/**",
    "**/stories/**",
    "**/story/**",
    "**/*.stories.*",
    "**/*.story.*",
    "**/examples/**",
    "**/example/**",
];

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub no_defaults: bool,
    pub include_tests: bool,
    pub include_generated: bool,
    pub cli_excludes: Vec<String>,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
}

pub struct Policy {
    root: PathBuf,
    defaults: GlobSet,
    explicit: GlobSet,
    includes: GlobSet,
    patterns: Vec<String>,
    archignore: Option<String>,
}

pub fn defaults(language: Language) -> Vec<String> {
    active_defaults(language, false, false)
}
fn active_defaults(
    language: Language,
    include_tests: bool,
    include_generated: bool,
) -> Vec<String> {
    let mut patterns: Vec<_> = SUPPORT_EXCLUDES.iter().map(|s| (*s).to_owned()).collect();
    if !include_tests {
        patterns.extend(TEST_EXCLUDES.iter().map(|s| (*s).to_owned()));
    }
    if !include_generated {
        patterns.extend(GENERATED_EXCLUDES.iter().map(|s| (*s).to_owned()));
        if matches!(language, Language::Rust | Language::Java) {
            patterns.push("**/target/**".into());
        }
        if language == Language::Java {
            patterns.push("**/out/**".into());
        }
        if language == Language::Elixir {
            patterns.extend(["**/_build/**".into(), "**/deps/**".into()]);
        }
    }
    patterns
}

pub fn is_test(path: &str) -> bool {
    static TESTS: std::sync::OnceLock<GlobSet> = std::sync::OnceLock::new();
    TESTS
        .get_or_init(|| {
            globset(
                &TEST_EXCLUDES
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect::<Vec<_>>(),
            )
            .expect("built-in test globs are valid")
        })
        .is_match(path)
}

impl Policy {
    pub fn new(root: &Path, language: Language, options: Options) -> Result<Self> {
        let defaults = if options.no_defaults {
            Vec::new()
        } else {
            active_defaults(language, options.include_tests, options.include_generated)
        };
        let explicit: Vec<_> = options
            .excludes
            .into_iter()
            .chain(options.cli_excludes)
            .collect();
        let patterns = defaults.iter().chain(&explicit).cloned().collect();
        let root = root.canonicalize().context("resolving exclusion root")?;
        let ignore_path = root.join(".archgraphignore");
        let exists = match std::fs::symlink_metadata(&ignore_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error).context("checking .archgraphignore"),
        };
        let archignore = if exists {
            let path = ignore_path.canonicalize()?;
            if !path.starts_with(&root) {
                bail!(".archgraphignore must stay inside the scan root");
            }
            Some(
                std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", ignore_path.display()))?,
            )
        } else {
            None
        };
        Ok(Self {
            root,
            defaults: globset(&defaults)?,
            explicit: globset(&explicit)?,
            includes: globset(&options.includes)?,
            patterns,
            archignore,
        })
    }
    pub fn included(&self, path: &str) -> bool {
        self.includes.is_match(path)
    }
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
    pub fn select(&self, paths: &[String]) -> Result<Vec<String>> {
        if paths.iter().any(|path| path.contains('\0')) {
            bail!("source paths cannot contain NUL bytes");
        }
        let candidates: Vec<_> = paths
            .iter()
            .filter(|path| {
                !self.explicit.is_match(path)
                    && (!self.defaults.is_match(path) || self.includes.is_match(path))
            })
            .cloned()
            .collect();
        let ignored = ignored(&self.root, &candidates, self.archignore.as_deref())?;
        Ok(candidates
            .into_iter()
            .filter(|path| !ignored.contains(path) || self.includes.is_match(path))
            .collect())
    }
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Git implements escaping, directory matches, ordered negation and nested .gitignore files.
fn ignored(root: &Path, paths: &[String], archignore: Option<&str>) -> Result<BTreeSet<String>> {
    if paths.is_empty() {
        return Ok(BTreeSet::new());
    }
    let mut standard = Command::new("git");
    standard.arg("-C").arg(root).args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.excludesFile=/dev/null",
        "check-ignore",
        "--no-index",
        "--stdin",
        "-z",
    ]);
    let queried: Vec<_> = std::iter::once(".".to_owned())
        .chain(paths.iter().cloned())
        .collect();
    let mut ignored = check_ignore(&mut standard, &queried, true)?.unwrap_or_default();
    // Selecting a root explicitly bypasses an ancestor ignore of that entire directory.
    if ignored.remove(".") {
        ignored.clear();
    }
    // An isolated index also honors ignore files in non-Git roots and roots below a monorepo.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = std::env::temp_dir().join(format!(
        "archgraph-ignore-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&temporary)?;
    let temp = Temporary(temporary);
    let git_dir = temp.0.join("git");
    let init = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "init",
            "--quiet",
            "--bare",
            "--template=",
        ])
        .arg(&git_dir)
        .output()
        .context("initializing isolated Git ignore matcher")?;
    if !init.status.success() {
        bail!(
            "initializing Git ignore matcher: {}",
            String::from_utf8_lossy(&init.stderr).trim()
        );
    }
    let ignore_file = temp.0.join("archgraphignore");
    std::fs::write(&ignore_file, archignore.unwrap_or_default())?;
    let mut custom = Command::new("git");
    custom
        .arg("--git-dir")
        .arg(&git_dir)
        .arg("--work-tree")
        .arg(root)
        .args([
            "-c",
            "core.bare=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
        ])
        .arg(format!("core.excludesFile={}", ignore_file.display()))
        .args(["check-ignore", "--no-index", "--stdin", "-z"])
        .current_dir(root);
    ignored.extend(check_ignore(&mut custom, paths, false)?.unwrap_or_default());
    Ok(ignored)
}

fn check_ignore(
    command: &mut Command,
    paths: &[String],
    allow_nonrepo: bool,
) -> Result<Option<BTreeSet<String>>> {
    // Untranslated messages keep the "not a git repository" check below locale-independent.
    let mut child = command
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running Git ignore matcher")?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let bytes = paths.join("\0").into_bytes();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        stdin.write_all(&bytes)?;
        stdin.write_all(b"\0")
    });
    let output = child
        .wait_with_output()
        .context("reading Git ignore matches")?;
    let written = writer
        .join()
        .map_err(|_| anyhow::anyhow!("Git ignore input writer panicked"))?;
    if output.status.code() == Some(128)
        && allow_nonrepo
        && String::from_utf8_lossy(&output.stderr).contains("not a git repository")
    {
        return Ok(None);
    }
    if !matches!(output.status.code(), Some(0 | 1)) {
        bail!(
            "Git ignore matching failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    written.context("writing Git ignore paths")?;
    Ok(Some(
        output
            .stdout
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8(path.to_vec()).context("Git ignore path is not UTF-8"))
            .collect::<Result<_>>()?,
    ))
}
