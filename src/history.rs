//! Architecture snapshots from first-parent Git history, collected in a disposable clone.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{CycleLevel, Diff, Document, Git, SCHEMA};
use crate::{diff, render};

#[derive(Serialize)]
pub struct History {
    pub schema: &'static str,
    pub kind: &'static str,
    pub snapshots: Vec<Snapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<Failure>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub release_markers: Vec<ReleaseMarker>,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub commit: String,
    pub committed_at: String,
    pub subject: String,
    pub graph: Document,
    pub tags: Vec<String>,
    pub summary: SnapshotSummary,
    /// Changes from the previous selected snapshot; null for the first.
    pub diff: Option<Diff>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Sampling {
    Monthly,
    Commits,
    Tags,
}

#[derive(Debug, Clone)]
pub struct HistoryOptions {
    pub sampling: Sampling,
    pub limit: usize,
    pub every: usize,
    pub since: Option<String>,
    pub cache_dir: Option<PathBuf>,
    pub cache_key: String,
    pub append: bool,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            sampling: Sampling::Monthly,
            limit: 24,
            every: 1,
            since: None,
            cache_dir: None,
            cache_key: String::new(),
            append: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReleaseMarker {
    pub commit: String,
    pub committed_at: String,
    pub tags: Vec<String>,
}

fn release_markers(
    commits: &[Commit],
    tags: &BTreeMap<String, Vec<String>>,
    snapshots: &[Snapshot],
) -> Vec<ReleaseMarker> {
    let selected: HashSet<_> = snapshots.iter().map(|s| s.commit.as_str()).collect();
    let Some(first) = commits
        .iter()
        .position(|c| selected.contains(c.hash.as_str()))
    else {
        return vec![];
    };
    let last = commits
        .iter()
        .rposition(|c| selected.contains(c.hash.as_str()))
        .unwrap();
    commits[first..=last]
        .iter()
        .rev()
        .filter_map(|c| {
            tags.get(&c.hash).map(|tags| ReleaseMarker {
                commit: c.hash.clone(),
                committed_at: c.date.clone(),
                tags: tags.clone(),
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub commit: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotSummary {
    pub files: usize,
    pub modules: usize,
    pub edges: usize,
    pub cycles: usize,
    pub largest_cycle: usize,
    pub modules_in_cycles_percent: f64,
    pub cycles_per_100_modules: f64,
    pub mean_instability: Option<f64>,
    pub forbidden: Option<usize>,
    pub hotspot_concentration: Option<f64>,
}

impl SnapshotSummary {
    pub fn of(graph: &Document) -> Self {
        let cycles: Vec<_> = graph
            .cycles
            .iter()
            .filter(|c| c.level == CycleLevel::Module)
            .collect();
        let members: HashSet<_> = cycles.iter().flat_map(|c| &c.members).collect();
        let instability: Vec<_> = graph.modules.iter().filter_map(|m| m.instability).collect();
        let denominator = graph.modules.len().max(1) as f64;
        Self {
            files: graph.files.len(),
            modules: graph.modules.len(),
            edges: graph.edges.len(),
            cycles: cycles.len(),
            largest_cycle: cycles.iter().map(|c| c.members.len()).max().unwrap_or(0),
            modules_in_cycles_percent: 100.0 * members.len() as f64 / denominator,
            cycles_per_100_modules: 100.0 * cycles.len() as f64 / denominator,
            mean_instability: (!instability.is_empty())
                .then(|| instability.iter().sum::<f64>() / instability.len() as f64),
            forbidden: graph.config.rules.as_ref().map(|_| graph.summary.forbidden),
            hotspot_concentration: graph
                .hotspots
                .as_ref()
                .filter(|h| !h.shallow)
                .and_then(|h| h.validation.share),
        }
    }
}

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
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
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8(out.stdout).context("git output is not UTF-8")
}

/// A single disposable clone shared by probes; never mutates the caller's worktree registry.
pub struct Checkout {
    temp: Temp,
    relative: PathBuf,
}
impl Checkout {
    pub fn new(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).context("resolving history root")?;
        let repo = fs::canonicalize(
            git(&root, &["rev-parse", "--show-toplevel"])?.trim_end_matches(['\n', '\r']),
        )?;
        let relative = root
            .strip_prefix(&repo)
            .context("scan root is outside the repository")?
            .to_owned();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "archgraph-history-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).context("creating history temporary directory")?;
        let temp = Temp(path);
        git(
            &temp.0,
            &[
                "clone",
                "--quiet",
                "--no-hardlinks",
                "--no-checkout",
                "--",
                repo.to_str().context("repository path is not UTF-8")?,
                "repo",
            ],
        )?;
        Ok(Self { temp, relative })
    }
    pub fn at(&self, commit: &str) -> Result<PathBuf> {
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("history checkout requires a full commit hash");
        }
        let checkout = self.temp.0.join("repo");
        git(
            &checkout,
            &["checkout", "--quiet", "--detach", "--force", commit],
        )?;
        git(&checkout, &["clean", "-ffdqx"])?;
        Ok(checkout.join(&self.relative))
    }
}

struct Commit {
    hash: String,
    date: String,
    subject: String,
    timestamp: i64,
}
fn commits(root: &Path, rev: &str) -> Result<Vec<Commit>> {
    git(
        root,
        &[
            "log",
            "--first-parent",
            "--format=%H%x00%cI%x00%ct%x00%s",
            "--end-of-options",
            rev,
            "--",
        ],
    )?
    .lines()
    .map(|line| {
        let fields: Vec<_> = line.splitn(4, '\0').collect();
        if fields.len() != 4 {
            bail!("invalid history commit metadata");
        }
        Ok(Commit {
            hash: fields[0].into(),
            date: fields[1].into(),
            timestamp: fields[2].parse()?,
            subject: fields[3].into(),
        })
    })
    .collect()
}
fn tags(root: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in git(
        root,
        &[
            "for-each-ref",
            "--format=%(*objectname)%00%(objectname)%00%(refname:strip=2)",
            "refs/tags",
        ],
    )?
    .lines()
    {
        let fields: Vec<_> = line.splitn(3, '\0').collect();
        if fields.len() == 3 {
            tags.entry(
                if fields[0].is_empty() {
                    fields[1]
                } else {
                    fields[0]
                }
                .into(),
            )
            .or_default()
            .push(fields[2].into());
        }
    }
    Ok(tags)
}

fn snapshot(
    commit: &Commit,
    mut graph: Document,
    tags: &BTreeMap<String, Vec<String>>,
    previous: Option<&Snapshot>,
) -> Snapshot {
    graph.source.git = Some(Git {
        commit: commit.hash.clone(),
        dirty: false,
    });
    Snapshot {
        commit: commit.hash.clone(),
        committed_at: commit.date.clone(),
        subject: commit.subject.clone(),
        tags: tags.get(&commit.hash).cloned().unwrap_or_default(),
        summary: SnapshotSummary::of(&graph),
        diff: previous.map(|p| diff::diff(&p.graph, &graph)),
        graph,
    }
}

/// Compatibility path: latest first-parent commits, no cache, abort on the first scan failure.
pub fn collect(
    root: &Path,
    rev: &str,
    limit: usize,
    mut scan: impl FnMut(&Path) -> Result<Document>,
) -> Result<History> {
    if limit == 0 {
        bail!("history limit must be positive");
    }
    let commits = commits(root, rev)?;
    if commits.is_empty() {
        bail!("no commits selected by {rev}");
    }
    let tags = tags(root)?;
    let checkout = Checkout::new(root)?;
    let mut snapshots = Vec::new();
    for commit in commits.iter().take(limit).rev() {
        eprintln!("history: scanning {}", commit.hash);
        let graph = scan(&checkout.at(&commit.hash)?)
            .with_context(|| format!("scanning commit {}", commit.hash))?;
        snapshots.push(snapshot(commit, graph, &tags, snapshots.last()));
    }
    Ok(History {
        schema: SCHEMA,
        kind: "history",
        release_markers: release_markers(&commits, &tags, &snapshots),
        snapshots,
        failures: Vec::new(),
    })
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn cache_file(path: &Path) -> Result<()> {
    refuse_symlink(path).context("refusing symlink history cache")
}

/// Refuses a symbolic link at `path`, which would redirect a write outside its folder.
pub fn refuse_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symbolic link; refusing to write through it",
            path.display()
        );
    }
    Ok(())
}

/// Replaces `path` through a temporary file and a rename, so an interrupted write never leaves
/// it truncated.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    refuse_symlink(path)?;
    // Devices and pipes (`-o /dev/null`) cannot be replaced; they are written in place.
    if fs::metadata(path).is_ok_and(|m| !m.is_file()) {
        return Ok(fs::write(path, bytes)?);
    }
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let result = (|| {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn cache_namespace(root: &Path, options: &HistoryOptions) -> Result<Option<PathBuf>> {
    options
        .cache_dir
        .as_ref()
        .map(|dir| -> Result<PathBuf> {
            if options.cache_key.is_empty() {
                bail!("history caching requires a scan-options cache key");
            }
            cache_file(dir)?;
            if let Some(parent) = dir.parent() {
                cache_file(parent)?;
            }
            fs::create_dir_all(dir)?;
            let root = fs::canonicalize(root)?;
            let key = format!(
                "{}\0{}\0{}",
                env!("CARGO_PKG_VERSION"),
                root.display(),
                options.cache_key
            );
            let namespace = dir.join(digest(key.as_bytes()));
            cache_file(&namespace)?;
            fs::create_dir_all(&namespace)?;
            Ok(namespace)
        })
        .transpose()
}

/// Save newly verified finding context without rescanning an already cached snapshot.
pub fn persist_context(root: &Path, options: &HistoryOptions, graph: &Document) -> Result<()> {
    let commit = graph
        .source
        .git
        .as_ref()
        .context("history context requires a snapshot commit")?;
    if !matches!(commit.commit.len(), 40 | 64)
        || !commit.commit.bytes().all(|c| c.is_ascii_hexdigit())
    {
        bail!("history context requires a full commit hash");
    }
    if commit.dirty {
        bail!("history context cannot cache a dirty snapshot");
    }
    let cache =
        cache_namespace(root, options)?.context("history context requires a cache directory")?;
    write_atomic(
        &cache.join(format!("{}.json", commit.commit)),
        &serde_json::to_vec(graph)?,
    )
}

/// Sampled drift scans skip failures and cache complete scan documents under a context hash.
pub fn collect_with_options(
    root: &Path,
    rev: &str,
    options: &HistoryOptions,
    mut scan: impl FnMut(&Path) -> Result<Document>,
) -> Result<History> {
    if options.limit == 0 || options.every == 0 {
        bail!("history limit and every must be positive");
    }
    if options.append && options.cache_dir.is_none() {
        bail!("history append requires a cache directory");
    }
    let commits = commits(root, rev)?;
    let latest = commits.first().context("no commits selected for history")?;
    let tags = tags(root)?;
    let since = match &options.since {
        Some(since) => crate::hotspots::utc_midnight(since)
            .context("history --since must be a YYYY-MM-DD date")?,
        None => latest.timestamp.saturating_sub(2 * 365 * 86400),
    };
    let mut seen_months = BTreeSet::new();
    let mut selected: Vec<_> = commits
        .iter()
        .rev()
        .filter(|c| c.timestamp >= since)
        .enumerate()
        .filter_map(|(i, c)| {
            let keep = match options.sampling {
                Sampling::Monthly => {
                    seen_months.insert(c.date.get(..7).unwrap_or(&c.date).to_owned())
                }
                Sampling::Commits => i % options.every == 0,
                Sampling::Tags => tags.contains_key(&c.hash),
            };
            keep.then(|| c.hash.clone())
        })
        .collect();
    if selected.len() > options.limit {
        selected.drain(..selected.len() - options.limit);
    }
    let cache = cache_namespace(root, options)?;
    let index = format!(
        "index-{:?}-{}-{}.json",
        options.sampling,
        options.every,
        digest(options.since.as_deref().unwrap_or("").as_bytes())
    );
    if options.append
        && let Some(cache) = &cache
    {
        let index = cache.join(&index);
        cache_file(&index)?;
        if index.exists() {
            let previous: Vec<String> = serde_json::from_slice(&fs::read(&index)?)
                .context("reading history append index")?;
            selected.extend(previous);
        }
    }
    let selected: HashSet<_> = selected.into_iter().collect();
    if selected.is_empty() {
        bail!("no commits selected for history sampling");
    }
    let mut checkout = None;
    let mut snapshots = Vec::new();
    let mut failures = Vec::new();
    for commit in commits.iter().rev().filter(|c| selected.contains(&c.hash)) {
        let path = cache
            .as_ref()
            .map(|dir| dir.join(format!("{}.json", commit.hash)));
        let result = (|| -> Result<Document> {
            if let Some(path) = &path {
                cache_file(path)?;
                if path.exists() {
                    eprintln!("history: cached {}", commit.hash);
                    let input: crate::model::ScanInput = serde_json::from_slice(&fs::read(path)?)?;
                    if input.source.git.as_ref().map(|g| g.commit.as_str())
                        != Some(commit.hash.as_str())
                    {
                        bail!("history cache commit mismatch");
                    }
                    return crate::restore_scan(input);
                }
            }
            if checkout.is_none() {
                checkout = Some(Checkout::new(root)?);
            }
            eprintln!("history: scanning {}", commit.hash);
            let mut graph = scan(&checkout.as_ref().unwrap().at(&commit.hash)?)?;
            graph.source.git = Some(Git {
                commit: commit.hash.clone(),
                dirty: false,
            });
            // A failed cache write (a full disk) must not discard a successful scan.
            if let Some(path) = &path
                && let Err(error) = serde_json::to_vec(&graph)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| write_atomic(path, &bytes))
            {
                eprintln!("history: not cached {}: {error:#}", commit.hash);
            }
            Ok(graph)
        })();
        match result {
            Ok(graph) => snapshots.push(snapshot(commit, graph, &tags, snapshots.last())),
            Err(error) => {
                eprintln!("history: skipped {}: {error:#}", commit.hash);
                failures.push(Failure {
                    commit: commit.hash.clone(),
                    error: format!("{error:#}"),
                });
            }
        }
    }
    if snapshots.is_empty() {
        bail!(
            "all selected history snapshots failed: {}",
            failures
                .iter()
                .map(|f| f.error.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    if let Some(cache) = cache
        && let Err(error) =
            serde_json::to_vec(&snapshots.iter().map(|s| &s.commit).collect::<Vec<_>>())
                .map_err(anyhow::Error::from)
                .and_then(|bytes| write_atomic(&cache.join(index), &bytes))
    {
        eprintln!("history: sample index not cached: {error:#}");
    }
    Ok(History {
        schema: SCHEMA,
        kind: "history",
        release_markers: release_markers(&commits, &tags, &snapshots),
        snapshots,
        failures,
    })
}

/// A standalone timeline which reuses the ordinary graph renderer for every snapshot.
pub fn render(history: &History) -> Result<String> {
    let data = serde_json::to_string(&(history, render::template()))?.replace('<', "\\u003c");
    Ok(include_str!("history.html").replace("__HISTORY_DATA__", &data))
}
