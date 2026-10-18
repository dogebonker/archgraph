//! Every Git subprocess archgraph runs, hardened so a repository's config never names a program
//! Git would run on our behalf (fsmonitor, hooks).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

fn subcommand<'a>(args: &[&'a str]) -> &'a str {
    args.iter().find(|arg| !arg.starts_with('-')).unwrap_or(&"")
}

/// Standard output of `git -C root args`; a failed exit is an error carrying Git's stderr.
pub fn run(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .context("running git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            subcommand(args),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// `run`, requiring UTF-8 output.
pub fn text(root: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(run(root, args)?)
        .with_context(|| format!("git {} output is not UTF-8", subcommand(args)))
}

/// The canonical top-level directory of the work tree containing `root`.
pub fn toplevel(root: &Path) -> Result<PathBuf> {
    let path = text(root, &["rev-parse", "--show-toplevel"])?;
    std::fs::canonicalize(path.trim_end_matches(['\n', '\r'])).context("resolving the Git root")
}

/// The full hash of the commit `rev` names.
pub fn resolve(root: &Path, rev: &str) -> Result<String> {
    Ok(text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )?
    .trim()
    .to_owned())
}

pub fn is_shallow(root: &Path) -> Result<bool> {
    Ok(text(root, &["rev-parse", "--is-shallow-repository"])?.trim() == "true")
}

/// A full SHA-1 or SHA-256 commit hash, never an abbreviation or a ref name.
pub fn is_full_hash(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64) && commit.bytes().all(|c| c.is_ascii_hexdigit())
}
