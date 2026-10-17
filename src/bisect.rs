//! Verified, bounded provenance of architecture findings on first-parent Git history.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{CycleLevel, Document, Evidence, Language, Link, Verdict};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Target {
    Cycle {
        level: CycleLevel,
        members: Vec<String>,
    },
    ForbiddenEdge {
        from: String,
        to: String,
    },
    Finding {
        finding_kind: String,
        subject: String,
    },
}

impl Target {
    pub fn finding(graph: &Document, index: usize) -> Result<Self> {
        let finding = graph
            .findings
            .get(index)
            .with_context(|| format!("finding index {index} is out of range"))?;
        if let Evidence::Cycle(cycle) = &finding.evidence {
            Ok(Self::Cycle {
                level: cycle.level.clone(),
                members: cycle.members.clone(),
            })
        } else {
            Ok(Self::Finding {
                finding_kind: serde_json::to_value(finding.kind)?
                    .as_str()
                    .context("finding kind is not a string")?
                    .to_owned(),
                subject: finding.subject.clone(),
            })
        }
    }

    fn present(&self, graph: &Document) -> bool {
        match self {
            Self::Finding {
                finding_kind,
                subject,
            } => graph.findings.iter().any(|finding| {
                finding.subject == *subject
                    && serde_json::to_value(finding.kind)
                        .is_ok_and(|kind| kind.as_str() == Some(finding_kind.as_str()))
            }),
            Self::Cycle { level, members } => graph.cycles.iter().any(|cycle| {
                cycle.level == *level && members.iter().all(|member| cycle.members.contains(member))
            }),
            Self::ForbiddenEdge { from, to } => graph.edges.iter().any(|edge| {
                edge.from == *from && edge.to == *to && edge.verdict == Some(Verdict::Forbidden)
            }),
        }
    }

    fn paths(&self, graph: &Document) -> BTreeSet<String> {
        match self {
            Self::Finding {
                finding_kind,
                subject,
            } => {
                let mut paths: BTreeSet<String> = graph
                    .files
                    .iter()
                    .filter(|file| file.path == *subject || file.module == *subject)
                    .map(|file| file.path.clone())
                    .collect();
                if let Some(finding) = graph.findings.iter().find(|finding| {
                    finding.subject == *subject
                        && serde_json::to_value(finding.kind)
                            .is_ok_and(|kind| kind.as_str() == Some(finding_kind.as_str()))
                }) {
                    match &finding.evidence {
                        Evidence::SinkSplit { components, .. } => paths.extend(
                            components
                                .iter()
                                .flat_map(|component| component.files.iter().cloned()),
                        ),
                        Evidence::HiddenCoupling { files, .. } => {
                            paths.extend(files.iter().cloned())
                        }
                        Evidence::InternalsReach { edges, entries, .. } => {
                            paths.extend(entries.iter().cloned());
                            paths.extend(
                                edges
                                    .iter()
                                    .flat_map(|edge| [edge.from.clone(), edge.to.clone()]),
                            );
                        }
                        Evidence::UnstableDependency { from, to, .. } => paths.extend(
                            graph
                                .files
                                .iter()
                                .filter(|file| file.module == *from || file.module == *to)
                                .map(|file| file.path.clone()),
                        ),
                        Evidence::Seam { clusters, hubs, .. } => {
                            paths.extend(hubs.iter().cloned());
                            paths.extend(
                                clusters
                                    .iter()
                                    .flat_map(|cluster| cluster.files.iter().cloned()),
                            );
                        }
                        Evidence::SmallRepo { .. } => {
                            paths.extend(graph.files.iter().map(|file| file.path.clone()))
                        }
                        _ => {}
                    }
                }
                paths
            }
            Self::ForbiddenEdge { from, to } => [from.clone(), to.clone()].into_iter().collect(),
            Self::Cycle {
                level: CycleLevel::File,
                members,
            } => members.iter().cloned().collect(),
            Self::Cycle {
                level: CycleLevel::Module,
                members,
            } => graph
                .files
                .iter()
                .filter(|file| members.contains(&file.module))
                .map(|file| file.path.clone())
                .collect(),
        }
    }

    fn edges(&self, graph: &Document) -> Vec<Link> {
        let modules: BTreeMap<_, _> = graph
            .files
            .iter()
            .map(|file| (&file.path, &file.module))
            .collect();
        graph
            .edges
            .iter()
            .filter(|edge| match self {
                Self::Finding { .. } => false,
                Self::ForbiddenEdge { from, to } => edge.from == *from && edge.to == *to,
                Self::Cycle {
                    level: CycleLevel::File,
                    members,
                } => {
                    edge.kind.is_runtime()
                        && members.contains(&edge.from)
                        && members.contains(&edge.to)
                }
                Self::Cycle {
                    level: CycleLevel::Module,
                    members,
                } => {
                    edge.kind.is_runtime()
                        && modules
                            .get(&edge.from)
                            .is_some_and(|module| members.contains(module))
                        && modules
                            .get(&edge.to)
                            .is_some_and(|module| members.contains(module))
                        && modules.get(&edge.from) != modules.get(&edge.to)
                }
            })
            .map(|edge| Link {
                from: edge.from.clone(),
                to: edge.to.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Inclusive lower bound on the first-parent chain; overrides `since`.
    pub from: Option<String>,
    pub since: String,
    pub max_commits: usize,
    pub max_scans: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            from: None,
            since: "2 years ago".into(),
            max_commits: 10_000,
            max_scans: 64,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub commit: String,
    pub author: String,
    pub email: String,
    pub authored_at: String,
    pub committed_at: String,
    pub subject: String,
    /// Inferred from the message only. No network request or inferred GitHub identity.
    pub pull_request: Option<u64>,
    pub age_days: u64,
}

#[derive(Debug, Serialize)]
pub struct Investigation {
    pub schema: &'static str,
    pub kind: &'static str,
    pub target: Target,
    /// `verified-transition`, `present-at-boundary`, `not-present`, or `unknown`.
    pub status: &'static str,
    pub method: &'static str,
    pub from: Option<String>,
    pub to: String,
    pub commits: usize,
    pub scans: usize,
    pub introduced_by: Option<Commit>,
    pub previous_commit: Option<String>,
    pub closing_edge: Option<Link>,
    pub limitations: Vec<String>,
}

#[derive(Clone)]
struct Probe {
    present: bool,
    edges: BTreeSet<(String, String)>,
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "--literal-pathspecs",
        ])
        .args(args)
        .output()
        .context("running provenance git command")?;
    if !output.status.success() {
        bail!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("provenance git output is not UTF-8")
}

fn resolve(root: &Path, revision: &str) -> Result<String> {
    Ok(git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )?
    .trim()
    .into())
}

fn metadata(root: &Path, commit: &str) -> Result<Commit> {
    let value = git(
        root,
        &[
            "show",
            "-s",
            "--format=%H%x00%an%x00%ae%x00%aI%x00%cI%x00%ct%x00%s%x00%B",
            commit,
        ],
    )?;
    let parts: Vec<_> = value.trim_end().splitn(8, '\0').collect();
    if parts.len() != 8 {
        bail!("invalid commit metadata");
    }
    let timestamp: u64 = parts[5].parse().context("invalid commit timestamp")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(Commit {
        commit: parts[0].into(),
        author: parts[1].into(),
        email: parts[2].into(),
        authored_at: parts[3].into(),
        committed_at: parts[4].into(),
        subject: parts[6].into(),
        pull_request: pull_request(parts[7]),
        age_days: now.saturating_sub(timestamp) / 86_400,
    })
}

fn pull_request(message: &str) -> Option<u64> {
    for line in message.lines() {
        let number = line
            .strip_prefix("Merge pull request #")
            .or_else(|| line.strip_prefix("Pull-Request: #"))
            .or_else(|| line.strip_prefix("PR: #"))
            .or_else(|| {
                line.rsplit_once("(#")
                    .and_then(|(_, suffix)| suffix.strip_suffix(')'))
            });
        if let Some(number) = number {
            let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(number) = digits.parse() {
                return Some(number);
            }
        }
    }
    None
}

/// Date filtering can omit interior commits; list neighbors are not necessarily Git parents.
fn verify_adjacent(root: &Path, previous: &str, current: &str) -> Result<()> {
    let parents = git(root, &["show", "-s", "--format=%P", current])?;
    if parents.split_whitespace().next() != Some(previous) {
        bail!(
            "date-filtered history has a gap: {previous} is not the first parent of {current}; no adjacent transition was verified within the selected window"
        );
    }
    Ok(())
}

/// A renamed target changes path identity even if its architecture stayed the same.
fn renamed_target(
    root: &Path,
    previous: &str,
    current: &str,
    paths: &BTreeSet<String>,
) -> Result<bool> {
    let changes = git(
        root,
        &[
            "diff",
            "--name-status",
            "--relative",
            "-M",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            previous,
            current,
            "--",
        ],
    )?;
    let mut fields = changes.split('\0');
    while let Some(status) = fields.next().filter(|status| !status.is_empty()) {
        let old = fields.next().context("missing changed path")?;
        if status.starts_with('R') || status.starts_with('C') {
            let new = fields.next().context("missing renamed path")?;
            if status.starts_with('R') && (paths.contains(old) || paths.contains(new)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Literal source tokens are candidate generators only: scans prove every reported transition.
fn import_tokens(source: &str, target: &str, language: Language) -> Vec<String> {
    let stem = Path::new(target)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    if stem.is_empty() {
        return vec![];
    }
    match language {
        Language::Typescript => source
            .lines()
            .filter(|line| {
                line.contains("import") || line.contains("export") || line.contains("require(")
            })
            .flat_map(|line| {
                let mut tokens = Vec::new();
                let mut quote = None;
                let mut start = 0;
                for (index, character) in line.char_indices() {
                    match quote {
                        None if character == '\'' || character == '"' => {
                            quote = Some(character);
                            start = index + 1;
                        }
                        Some(end) if end == character => {
                            let token = &line[start..index];
                            if token.split('/').any(|part| {
                                part == stem
                                    || part.strip_suffix(".js") == Some(stem)
                                    || part.strip_suffix(".ts") == Some(stem)
                            }) {
                                tokens.push(token.to_owned());
                            }
                            quote = None;
                        }
                        _ => {}
                    }
                }
                tokens
            })
            .collect(),
        Language::Python => source
            .lines()
            .map(str::trim)
            .filter(|line| {
                (line.starts_with("from ") || line.starts_with("import "))
                    && line
                        .split(|character: char| !character.is_alphanumeric() && character != '_')
                        .any(|part| part == stem)
            })
            .map(str::to_owned)
            .collect(),
        Language::Java => source
            .lines()
            .filter_map(|line| {
                let import = line
                    .trim()
                    .strip_prefix("import ")?
                    .trim_start_matches("static ");
                let import = import.split(';').next()?.trim();
                import
                    .split('.')
                    .any(|part| part == stem)
                    .then(|| import.to_owned())
            })
            .collect(),
        // shortcut: unresolved and aliased import syntax uses graph bisect; add language-aware tokens when collectors expose source spans.
        _ => vec![],
    }
}

/// The callback scans an immutable checkout of the supplied OID using the same collector,
/// module configuration and rules as `graph`. This function never changes the user's checkout.
/// A verified transition is a local absent -> present boundary, not proof of earliest-ever birth.
pub fn investigate(
    root: &Path,
    graph: &Document,
    target: Target,
    options: &Options,
    mut scan_commit: impl FnMut(&str) -> Result<Document>,
) -> Result<Investigation> {
    if options.max_commits == 0 || options.max_scans < 2 {
        bail!("provenance needs positive commit limit and at least two scans");
    }
    let to = resolve(
        root,
        graph.source.git.as_ref().map_or("HEAD", |git| &git.commit),
    )?;
    let lower = options
        .from
        .as_deref()
        .map(|rev| resolve(root, rev))
        .transpose()?;
    let limit = format!("--max-count={}", options.max_commits);
    let since = format!("--since-as-filter={}", options.since);
    let mut args = vec!["rev-list", "--first-parent", &limit];
    if lower.is_none() {
        args.push(&since);
    }
    args.extend(["--end-of-options", &to, "--"]);
    let history = git(root, &args)?;
    let mut commits: Vec<String> = history.lines().map(str::to_owned).collect();
    if let Some(lower) = &lower {
        let index = commits
            .iter()
            .position(|commit| commit == lower)
            .context("--from is outside the selected first-parent history or commit cap")?;
        commits.truncate(index + 1);
    }
    let mut result = Investigation {
        schema: crate::model::SCHEMA, kind: "bisect", target,
        status: "unknown", method: "edge", from: commits.last().cloned(), to: to.clone(),
        commits: commits.len(), scans: 0, introduced_by: None, previous_commit: None,
        closing_edge: None, limitations: vec![
            "First-parent history only; a verified transition does not prove earliest-ever introduction or uninterrupted presence since then. Module/file names are matched per snapshot; renamed boundaries may change identity.".into(),
        ],
    };
    if lower.is_none() {
        result.limitations.push(format!(
            "History bounded by --since {:?} and {} commits.",
            options.since, options.max_commits
        ));
    }
    if git(root, &["rev-parse", "--is-shallow-repository"])?.trim() == "true" {
        result
            .limitations
            .push("Shallow clone: earlier history is unavailable.".into());
    }
    if matches!(result.target, Target::ForbiddenEdge { .. }) {
        result.limitations.push("Forbidden-edge probes must use the supplied graph's fixed rules; this is not proof those rules existed at the time.".into());
    }
    if commits.is_empty() {
        result
            .limitations
            .push("No commits in the selected window.".into());
        return Ok(result);
    }
    let paths = result.target.paths(graph);
    let mut states: BTreeMap<String, Probe> = BTreeMap::new();
    let mut scans = 0;
    let mut state = |commit: &str| -> Result<Probe> {
        if let Some(present) = states.get(commit) {
            return Ok(present.clone());
        }
        if scans >= options.max_scans {
            bail!("scan budget exhausted ({})", options.max_scans);
        }
        scans += 1;
        let graph = scan_commit(commit).with_context(|| format!("scanning {commit}"))?;
        let present = Probe {
            present: result.target.present(&graph),
            edges: graph
                .edges
                .into_iter()
                .map(|edge| (edge.from, edge.to))
                .collect(),
        };
        states.insert(commit.to_owned(), present.clone());
        Ok(present)
    };
    let searched = (|| -> Result<()> {
        if !state(&to)?.present {
            result.status = "not-present";
            return Ok(());
        }
        let repo = git(root, &["rev-parse", "--show-toplevel"])?;
        let root = std::fs::canonicalize(root)?;
        let canonical_repo = std::fs::canonicalize(repo.trim())?;
        let prefix = root
            .strip_prefix(&canonical_repo)
            .context("scan root outside repository")?;
        let mut candidates: BTreeMap<usize, Link> = BTreeMap::new();
        for edge in result.target.edges(graph) {
            let file = prefix.join(&edge.from).to_string_lossy().replace('\\', "/");
            let source =
                git(Path::new(repo.trim()), &["show", &format!("{to}:{file}")]).unwrap_or_default();
            for token in import_tokens(&source, &edge.to, graph.source.language) {
                let pickaxe = format!("-S{token}");
                let mut args = vec![
                    "log",
                    "--first-parent",
                    "--follow",
                    &limit,
                    "--format=%H",
                    "--no-ext-diff",
                    "--no-textconv",
                    &pickaxe,
                ];
                if lower.is_none() {
                    args.push(&since);
                }
                args.extend(["--end-of-options", &to, "--", &file]);
                let changes = git(Path::new(repo.trim()), &args)?;
                for changed in changes.lines() {
                    if let Some(index) = commits.iter().position(|commit| commit == changed)
                        && index + 1 < commits.len()
                    {
                        candidates.entry(index).or_insert_with(|| edge.clone());
                    }
                }
            }
        }
        for (index, edge) in candidates {
            let current = state(&commits[index])?;
            let previous = state(&commits[index + 1])?;
            if current.present && !previous.present {
                verify_adjacent(&root, &commits[index + 1], &commits[index])?;
                if renamed_target(&root, &commits[index + 1], &commits[index], &paths)? {
                    result.limitations.push("Target paths were renamed at the candidate boundary; architecture birth cannot be distinguished from a path identity change.".into());
                    return Ok(());
                }
                result.introduced_by = Some(metadata(&root, &commits[index])?);
                result.previous_commit = Some(commits[index + 1].clone());
                // The candidate token belongs to this edge, but only call it closing when the
                // edge itself changed as well. The graph-level transition is always verified.
                let pair = (edge.from.clone(), edge.to.clone());
                result.closing_edge = (current.edges.contains(&pair)
                    && !previous.edges.contains(&pair))
                .then_some(edge);
                result.status = "verified-transition";
                return Ok(());
            }
        }
        result.method = "scan-bisect";
        result.limitations.push("Binary fallback assumes monotonic presence between probes; removals and reintroductions can hide other transitions. Only the adjacent boundary reported here is verified.".into());
        let mut absent = commits.len() - 1;
        if state(&commits[absent])?.present {
            result.status = "present-at-boundary";
            return Ok(());
        }
        let mut present = 0;
        while absent - present > 1 {
            let middle = (present + absent) / 2;
            if state(&commits[middle])?.present {
                present = middle;
            } else {
                absent = middle;
            }
        }
        verify_adjacent(&root, &commits[absent], &commits[present])?;
        if renamed_target(&root, &commits[absent], &commits[present], &paths)? {
            result.limitations.push("Target paths were renamed at the candidate boundary; architecture birth cannot be distinguished from a path identity change.".into());
            return Ok(());
        }
        result.introduced_by = Some(metadata(&root, &commits[present])?);
        result.previous_commit = Some(commits[absent].clone());
        result.status = "verified-transition";
        Ok(())
    })();
    result.scans = scans;
    if let Err(error) = searched {
        result.status = "unknown";
        result.introduced_by = None;
        result.previous_commit = None;
        result.closing_edge = None;
        result
            .limitations
            .push(format!("Investigation incomplete: {error:#}"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{import_tokens, pull_request};
    use crate::model::Language;

    #[test]
    fn literal_import_candidates_and_offline_prs_are_conservative() {
        assert_eq!(
            import_tokens(
                "import '../model/Order.js';",
                "src/model/Order.ts",
                Language::Typescript
            ),
            ["../model/Order.js"]
        );
        assert_eq!(
            import_tokens(
                "from model import order as current",
                "model/order.py",
                Language::Python
            ),
            ["from model import order as current"]
        );
        assert_eq!(
            import_tokens("import model.Order;", "model/Order.java", Language::Java),
            ["model.Order"]
        );
        assert!(
            import_tokens(
                "const order = 'hello';",
                "model/order.ts",
                Language::Typescript
            )
            .is_empty()
        );
        assert_eq!(
            pull_request("Merge pull request #4521 from example/topic"),
            Some(4521)
        );
        assert_eq!(pull_request("a general issue #4521"), None);
    }
}
