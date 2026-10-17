//! Explainable architecture hotspots from the scanned commit's Git history.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{CycleLevel, Document, Evidence};

#[derive(Debug, Clone)]
pub struct Options {
    pub since: String,
    pub max_files: usize,
    pub authors: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            since: "12 months ago".into(),
            max_files: 30,
            authors: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotspots {
    pub since: String,
    pub since_timestamp: i64,
    pub as_of: String,
    pub max_files: usize,
    pub authors_enabled: bool,
    pub shallow: bool,
    pub commits: usize,
    pub skipped: usize,
    pub limitations: Vec<String>,
    pub files: Vec<Hotspot>,
    pub validation: Validation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotspot {
    pub path: String,
    pub module: String,
    pub commits: usize,
    pub lines_added: usize,
    pub lines_deleted: usize,
    pub authors: usize,
    pub bugfix_commits: usize,
    pub churn: f64,
    pub coupling: f64,
    pub cycle: f64,
    pub author_factor: f64,
    pub score: f64,
    pub cycle_size: usize,
    pub cut_candidate: bool,
    pub finding_indices: Vec<usize>,
}

/// Unique fix-message commits touching scanned files, rather than summed file touches.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Validation {
    pub files: usize,
    pub top_files: usize,
    pub bugfix_commits: usize,
    pub top_bugfix_commits: usize,
    pub share: Option<f64>,
    /// Naive file-share baseline; use random_lift to account for multi-file commits.
    pub lift: Option<f64>,
    #[serde(default)]
    pub expected_random_share: Option<f64>,
    #[serde(default)]
    pub random_lift: Option<f64>,
}

#[derive(Default)]
struct Changes {
    commits: HashSet<String>,
    authors: HashSet<String>,
    fixes: HashSet<String>,
    added: usize,
    deleted: usize,
}

struct Change {
    path: String,
    old: Option<String>,
    added: usize,
    deleted: usize,
}

#[derive(Default)]
struct Commit {
    hash: String,
    author: String,
    fix: bool,
    changes: Vec<Change>,
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .output()
        .context("running Git for hotspots")?;
    if !out.status.success() {
        bail!(
            "hotspots needs Git history: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Relative windows end at the graph's commit, so historical snapshots remain reproducible.
/// Months use 365/12 days and years use 365 days; explicit dates use Git's date parser.
fn since_timestamp(root: &Path, since: &str, end: i64) -> Result<i64> {
    let words: Vec<_> = since.split_whitespace().collect();
    if let [count, unit, "ago"] = words.as_slice() {
        let count: i64 = count
            .parse()
            .context("hotspot window count must be an integer")?;
        let seconds = match unit.trim_end_matches('s') {
            "second" => 1,
            "minute" => 60,
            "hour" => 3600,
            "day" => 86400,
            "week" => 7 * 86400,
            "month" => 365 * 86400 / 12,
            "year" => 365 * 86400,
            _ => bail!("unsupported hotspot window unit: {unit}"),
        };
        if count < 0 {
            bail!("hotspot window must not be negative");
        }
        return end
            .checked_sub(
                count
                    .checked_mul(seconds)
                    .context("hotspot window is too large")?,
            )
            .context("hotspot window is too large");
    }
    // Reject ambiguous relative strings instead of silently anchoring them to today's date.
    if !since.bytes().take(4).all(|c| c.is_ascii_digit()) || since.len() < 10 {
        bail!("hotspot --since must be an absolute date or '<N> days/weeks/months/years ago'");
    }
    let out = git(root, &["rev-parse", &format!("--since={since}")])?;
    String::from_utf8(out)?
        .trim()
        .strip_prefix("--max-age=")
        .context("Git did not return a hotspot start date")?
        .parse()
        .context("invalid Git start date")
}

/// One NUL-delimited numstat pass; rename records carry separate old/new path tokens.
fn parse_log(bytes: &[u8]) -> Result<Vec<Commit>> {
    let text = std::str::from_utf8(bytes).context("Git hotspot paths must be UTF-8")?;
    let fields: Vec<_> = text.split('\0').collect();
    let mut commits = Vec::new();
    let mut at = 0;
    while at < fields.len() {
        if fields[at] == "ARCHGRAPH" {
            if at + 3 >= fields.len() {
                bail!("truncated Git hotspot commit header");
            }
            let message = fields[at + 3].to_ascii_lowercase();
            commits.push(Commit {
                hash: fields[at + 1].into(),
                author: fields[at + 2].into(),
                fix: message
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .any(|word| {
                        matches!(
                            word,
                            "fix"
                                | "fixes"
                                | "fixed"
                                | "bug"
                                | "bugs"
                                | "bugfix"
                                | "regression"
                                | "regressions"
                        )
                    }),
                changes: Vec::new(),
            });
            at += 4;
        } else if fields[at].is_empty() || fields[at] == "\n" {
            at += 1;
        } else {
            let mut stat = fields[at].trim_start_matches('\n').splitn(3, '\t');
            let added = stat.next().context("missing Git numstat additions")?;
            let deleted = stat.next().context("missing Git numstat deletions")?;
            let path = stat.next().context("missing Git numstat path")?;
            let (old, path) = if path.is_empty() {
                if at + 2 >= fields.len() {
                    bail!("truncated Git hotspot rename");
                }
                at += 2;
                (Some(fields[at - 1].to_owned()), fields[at])
            } else {
                (None, path)
            };
            let number = |n: &str| -> Result<usize> {
                if n == "-" {
                    Ok(0)
                } else {
                    n.parse().context("invalid Git numstat count")
                }
            };
            commits
                .last_mut()
                .context("Git numstat without a commit")?
                .changes
                .push(Change {
                    path: path.into(),
                    old,
                    added: number(added)?,
                    deleted: number(deleted)?,
                });
            at += 1;
        }
    }
    Ok(commits)
}

pub fn collect(root: &Path, graph: &Document, options: &Options) -> Result<Hotspots> {
    if options.max_files == 0 {
        bail!("hotspot max-files must be greater than zero");
    }
    let revision = graph
        .source
        .git
        .as_ref()
        .map_or("HEAD", |g| g.commit.as_str());
    let header = String::from_utf8(git(
        root,
        &[
            "show",
            "-s",
            "--format=%H %ct",
            "--end-of-options",
            revision,
        ],
    )?)?;
    let (as_of, timestamp) = header
        .trim()
        .split_once(' ')
        .context("missing hotspot commit timestamp")?;
    let since_timestamp = since_timestamp(root, &options.since, timestamp.parse()?)?;
    let shallow = git(root, &["rev-parse", "--is-shallow-repository"])? == b"true\n";
    let prefix = String::from_utf8(git(root, &["rev-parse", "--show-prefix"])?)?;
    let prefix = prefix.trim_end_matches('\n');
    let scanned: HashSet<_> = graph.files.iter().map(|f| f.path.as_str()).collect();
    let mut stats: HashMap<String, Changes> = HashMap::new();
    let mut renamed: HashMap<String, String> = HashMap::new();
    let (mut count, mut skipped) = (0, 0);
    if !shallow {
        let bytes = git(
            root,
            &[
                "log",
                "--numstat",
                "-z",
                "-M",
                "--topo-order",
                "--no-merges",
                "--no-ext-diff",
                "--no-textconv",
                "--format=%x00ARCHGRAPH%x00%H%x00%aE%x00%B%x00",
                &format!("--since-as-filter=@{since_timestamp}"),
                "--end-of-options",
                as_of,
                "--",
            ],
        )?;
        for commit in parse_log(&bytes)? {
            let sweep = commit.changes.len() > options.max_files;
            if sweep {
                skipped += 1;
            } else if !commit.changes.is_empty() {
                count += 1;
            }
            for change in commit.changes {
                let current = renamed.get(&change.path).cloned().unwrap_or(change.path);
                if let Some(old) = change.old {
                    renamed.insert(old, current.clone());
                }
                if sweep {
                    continue;
                }
                let Some(path) = current
                    .strip_prefix(prefix)
                    .filter(|p| scanned.contains(*p))
                else {
                    continue;
                };
                let stat = stats.entry(path.into()).or_default();
                stat.commits.insert(commit.hash.clone());
                stat.authors.insert(commit.author.clone());
                stat.added += change.added;
                stat.deleted += change.deleted;
                if commit.fix {
                    stat.fixes.insert(commit.hash.clone());
                }
            }
        }
    }
    let max_commits = stats.values().map(|s| s.commits.len()).max().unwrap_or(0);
    let max_authors = stats.values().map(|s| s.authors.len()).max().unwrap_or(0);
    let max_coupling = graph
        .modules
        .iter()
        .map(|m| m.weighted_ca + m.weighted_ce)
        .fold(0.0_f64, f64::max);
    let mut cycles = BTreeMap::new();
    for cycle in graph
        .cycles
        .iter()
        .filter(|c| c.level == CycleLevel::Module)
    {
        for member in &cycle.members {
            let size = cycles.entry(member.as_str()).or_insert(0);
            *size = (*size).max(cycle.members.len());
        }
    }
    let mut adjacent = HashSet::new();
    for edge in &graph.module_edges {
        if cycles.contains_key(edge.from.as_str()) {
            adjacent.insert(edge.to.as_str());
        }
        if cycles.contains_key(edge.to.as_str()) {
            adjacent.insert(edge.from.as_str());
        }
    }
    let coupling: HashMap<_, _> = graph
        .modules
        .iter()
        .map(|m| (m.id.as_str(), m.weighted_ca + m.weighted_ce))
        .collect();
    let cuts: HashSet<_> = graph
        .cycles
        .iter()
        .flat_map(|c| {
            c.break_set.iter().chain(
                c.two_way
                    .iter()
                    .flatten()
                    .filter(|pair| {
                        !pair.even && !pair.registry && !pair.entry && !pair.resolved_by_move
                    })
                    .flat_map(|pair| &pair.edges),
            )
        })
        .map(|l| l.from.as_str())
        .collect();
    let mut files: Vec<_> = graph
        .files
        .iter()
        .map(|file| {
            let changes = stats.get(&file.path);
            let commits = changes.map_or(0, |s| s.commits.len());
            let authors = changes.map_or(0, |s| s.authors.len());
            let churn = if max_commits == 0 {
                0.0
            } else {
                (commits as f64).ln_1p() / (max_commits as f64).ln_1p()
            };
            let coupling = if max_coupling == 0.0 {
                0.0
            } else {
                coupling.get(file.module.as_str()).copied().unwrap_or(0.0) / max_coupling
            };
            let cycle_size = cycles.get(file.module.as_str()).copied().unwrap_or(0);
            let cycle = if cycle_size > 0 {
                1.0
            } else if adjacent.contains(file.module.as_str()) {
                0.5
            } else {
                0.0
            };
            let author_factor = if options.authors && max_authors > 0 {
                authors as f64 / max_authors as f64
            } else {
                0.0
            };
            Hotspot {
                path: file.path.clone(),
                module: file.module.clone(),
                commits,
                lines_added: changes.map_or(0, |s| s.added),
                lines_deleted: changes.map_or(0, |s| s.deleted),
                authors,
                bugfix_commits: changes.map_or(0, |s| s.fixes.len()),
                churn,
                coupling,
                cycle,
                author_factor,
                score: churn * (coupling + cycle) * (1.0 + author_factor),
                cycle_size,
                cut_candidate: cuts.contains(file.path.as_str()),
                finding_indices: Vec::new(),
            }
        })
        .collect();
    files.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
    });
    let top_files = files.len().div_ceil(10);
    let fixes: HashSet<_> = stats.values().flat_map(|s| &s.fixes).collect();
    let top_fixes: HashSet<_> = files
        .iter()
        .take(top_files)
        .filter_map(|f| stats.get(&f.path))
        .flat_map(|s| &s.fixes)
        .collect();
    let share = (!fixes.is_empty()).then(|| top_fixes.len() as f64 / fixes.len() as f64);
    let mut fix_sizes: BTreeMap<&String, usize> = BTreeMap::new();
    for changes in stats.values() {
        for commit in &changes.fixes {
            *fix_sizes.entry(commit).or_default() += 1;
        }
    }
    // Chance that a uniform sample of top_files hits at least one of a commit's files.
    let expected_random_share = (!fix_sizes.is_empty()).then(|| {
        fix_sizes
            .values()
            .map(|&k| {
                if k > files.len() - top_files {
                    return 1.0;
                }
                1.0 - (0..k).fold(1.0, |miss, i| {
                    miss * (files.len() - top_files - i) as f64 / (files.len() - i) as f64
                })
            })
            .sum::<f64>()
            / fix_sizes.len() as f64
    });
    let validation = Validation {
        files: files.len(),
        top_files,
        bugfix_commits: fixes.len(),
        top_bugfix_commits: top_fixes.len(),
        share,
        lift: share.map(|share| share / (top_files as f64 / files.len() as f64)),
        expected_random_share,
        random_lift: share
            .zip(expected_random_share)
            .filter(|(_, expected)| *expected > 0.0)
            .map(|(observed, expected)| observed / expected),
    };
    let mut limitations = vec![
        "Commit-message fix/bug/regression matches are a proxy, not verified defects; concentration is in-sample, not predictive validation.".into(),
        "Merge commits are excluded; rename following uses Git similarity detection and may miss rewritten files or divergent branch renames.".into(),
        "Only scanned files contribute to normalization; commits touching more than max_files anywhere in the repository are excluded.".into(),
    ];
    if shallow {
        limitations.push(
            "Shallow clone: churn is unavailable; fetch full history to compute hotspots.".into(),
        );
    }
    if graph.source.git.as_ref().is_some_and(|g| g.dirty) {
        limitations.push("Working tree is dirty: churn describes committed history only.".into());
    }
    let mut result = Hotspots {
        since: options.since.clone(),
        since_timestamp,
        as_of: as_of.into(),
        max_files: options.max_files,
        authors_enabled: options.authors,
        shallow,
        commits: count,
        skipped,
        limitations,
        files,
        validation,
    };
    attach_findings(&mut result, graph);
    Ok(result)
}

/// Refresh after findings are recomputed or re-ranked: indices refer to the current document.
pub fn attach_findings(report: &mut Hotspots, graph: &Document) {
    for file in &mut report.files {
        file.finding_indices = graph
            .findings
            .iter()
            .enumerate()
            .filter_map(|(i, finding)| {
                let attached = finding.subject == file.path
                    || match &finding.evidence {
                        Evidence::Cycle(c) => {
                            if c.level == CycleLevel::File {
                                c.members.contains(&file.path)
                            } else {
                                c.members.contains(&file.module)
                            }
                        }
                        Evidence::HiddenCoupling { files, .. } => files.contains(&file.path),
                        Evidence::ReexportCycle { members, .. } => members.contains(&file.path),
                        Evidence::Grabbag(e) => e.members.contains(&file.module),
                        Evidence::InternalsReach { edges, .. } => edges
                            .iter()
                            .any(|e| e.from == file.path || e.to == file.path),
                        Evidence::SinkSplit { components, .. } => {
                            components.iter().any(|c| c.files.contains(&file.path))
                        }
                        Evidence::Seam { clusters, .. } => {
                            clusters.iter().any(|c| c.files.contains(&file.path))
                        }
                        Evidence::UnstableDependency { from, to, .. } => {
                            file.module == *from || file.module == *to
                        }
                        Evidence::MisplacedFile { .. } | Evidence::SmallRepo { .. } => false,
                    };
                attached.then_some(i)
            })
            .collect();
    }
}

/// Maximum attached hotspot per finding, preserving the existing order when priorities tie.
pub fn finding_priorities(report: &Hotspots, count: usize) -> Vec<f64> {
    let mut scores = vec![0.0_f64; count];
    for file in &report.files {
        for &i in &file.finding_indices {
            if let Some(score) = scores.get_mut(i) {
                *score = score.max(file.score);
            }
        }
    }
    scores
}
