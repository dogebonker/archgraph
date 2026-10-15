//! Files that change together in git history.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::model::{Cochange, CochangePair};

/// A commit touching more files is a sweep (formatting, renames, vendoring) and says nothing
/// about which files belong together.
const MAX_FILES: usize = 30;
/// Fewer shared commits than this is coincidence more often than coupling.
const MIN_COMMITS: usize = 3;

/// Pairs of `files` changed together in the last `limit` commits, following renames.
pub fn collect(root: &Path, limit: usize, files: &[String]) -> Result<Cochange> {
    // `--relative` limits the log to the scanned root and makes its paths relative to it.
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        // Unquoted non-ASCII paths, so they match the scanned names.
        // core.fsmonitor=false: never run a program the repository's config names.
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotePath=false",
            "log",
            "--name-status",
            "-M",
            "--relative",
            "--format=%x00",
        ])
        .arg(format!("-n{limit}"))
        .output()
        .context("running git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "--cochange needs git history: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let scanned: HashSet<&str> = files.iter().map(String::as_str).collect();
    // Newest commit first, so a rename is seen before the commits that used the old name.
    let mut renamed: HashMap<String, String> = HashMap::new();
    let (mut commits, mut skipped) = (0, 0);
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for commit in String::from_utf8_lossy(&out.stdout).split('\0').skip(1) {
        let mut touched = Vec::new();
        for line in commit.lines().filter(|l| l.contains('\t')) {
            let fields: Vec<&str> = line.split('\t').collect();
            let current = |p: &str| renamed.get(p).cloned().unwrap_or_else(|| p.to_owned());
            let name = current(fields[fields.len() - 1]);
            if let [status, old, _] = fields[..]
                && status.starts_with('R')
            {
                renamed.insert(old.to_owned(), name.clone());
            }
            touched.push(name);
        }
        if touched.len() > MAX_FILES {
            skipped += 1;
            continue;
        }
        commits += 1;
        let kept: BTreeSet<String> = touched
            .into_iter()
            .filter(|f| scanned.contains(f.as_str()))
            .collect();
        let kept: Vec<String> = kept.into_iter().collect();
        for (i, a) in kept.iter().enumerate() {
            for b in &kept[i + 1..] {
                *counts.entry((a.clone(), b.clone())).or_default() += 1;
            }
        }
    }
    Ok(Cochange {
        limit,
        commits,
        skipped,
        pairs: counts
            .into_iter()
            .filter(|&(_, n)| n >= MIN_COMMITS)
            .map(|((a, b), commits)| CochangePair {
                files: [a, b],
                commits,
            })
            .collect(),
    })
}
