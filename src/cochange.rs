//! Files that change together in git history.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::model::{Cochange, CochangePair};
use crate::{git, hotspots};

/// A commit touching more files is a sweep (formatting, renames, vendoring) and says nothing
/// about which files belong together.
const MAX_FILES: usize = 30;
/// Fewer shared commits than this is coincidence more often than coupling.
const MIN_COMMITS: usize = 3;

/// Pairs of `files` changed together in the last `limit` commits, following renames.
pub fn collect(root: &Path, limit: usize, files: &[String]) -> Result<Cochange> {
    // `--relative` and the `.` pathspec limit the log to commits touching the scanned root and
    // make their paths relative to it; `-z` leaves paths unquoted, so they match the scanned names.
    let out = git::run(
        root,
        &[
            "log",
            "--numstat",
            "-z",
            "-M",
            "--no-merges",
            "--no-ext-diff",
            "--no-textconv",
            "--relative",
            hotspots::LOG_FORMAT,
            &format!("-n{limit}"),
            "--",
            ".",
        ],
    )
    .context("--cochange needs git history")?;
    let scanned: HashSet<&str> = files.iter().map(String::as_str).collect();
    let log = hotspots::parse_log(&String::from_utf8_lossy(&out))?;
    let (commits, skipped) = hotspots::follow_renames(log, MAX_FILES);
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for commit in &commits {
        let kept: BTreeSet<&str> = commit
            .changes
            .iter()
            .map(|c| c.path.as_str())
            .filter(|p| scanned.contains(p))
            .collect();
        let kept: Vec<&str> = kept.into_iter().collect();
        for (i, a) in kept.iter().enumerate() {
            for b in &kept[i + 1..] {
                *counts.entry((a, b)).or_default() += 1;
            }
        }
    }
    Ok(Cochange {
        limit,
        commits: commits.len(),
        skipped,
        pairs: counts
            .into_iter()
            .filter(|&(_, n)| n >= MIN_COMMITS)
            .map(|((a, b), commits)| CochangePair {
                files: [a.to_owned(), b.to_owned()],
                commits,
            })
            .collect(),
    })
}
