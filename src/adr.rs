//! Conservative ADR proposals: decision text supplies provenance, the graph supplies enforcement.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
pub struct Proposal {
    pub id: String,
    pub from: String,
    pub to: String,
    pub reason: String,
    pub source: String,
    pub lifecycle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<crate::model::EdgeKind>>,
    pub source_sha256: Option<String>,
}
impl Proposal {
    fn rule(&self) -> Value {
        let mut value = serde_json::json!({"id":self.id,"from":self.from,"to":self.to,"reason":self.reason,"source":self.source});
        if let Some(kinds) = &self.kinds {
            value["kinds"] = serde_json::to_value(kinds).expect("edge kinds serialize");
        }
        value
    }
}
#[derive(Debug, Serialize)]
pub struct Notice {
    pub source: String,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub documents: usize,
    pub proposals: Vec<Proposal>,
    pub informational: Vec<Notice>,
}
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub schema: &'static str,
    pub stale: Vec<Notice>,
    pub missing: Vec<Notice>,
    pub unknown: Vec<Notice>,
    /// Forbid and allow rules without a `source`, listed only when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsourced: Option<Vec<String>>,
}

fn safe_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    let relative = path
        .strip_prefix(root)
        .context("ADR path must be inside the repository")?;
    let mut cursor = root.to_owned();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            let name = part.to_str().context("ADR paths must be UTF-8")?;
            if name.chars().any(char::is_control) {
                bail!("ADR paths must not contain control characters");
            }
            cursor.push(part);
            if fs::symlink_metadata(&cursor)?.file_type().is_symlink() {
                bail!("ADR paths must not traverse symlinks: {}", cursor.display());
            }
        } else {
            bail!("ADR paths must be normalized repository-relative paths");
        }
    }
    Ok(path)
}
fn markdown_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!("ADR directory contains symlink: {}", entry.path().display());
        }
        if kind.is_dir() {
            markdown_files(&entry.path(), files)?;
        } else if kind.is_file() && entry.path().extension().is_some_and(|v| v == "md") {
            files.push(entry.path());
        }
    }
    Ok(())
}
struct Document<'a> {
    meta: Value,
    body: &'a str,
}
fn document(text: &str) -> Result<Document<'_>> {
    if let Some(rest) = text.strip_prefix("---\n") {
        let (yaml, body) = rest
            .split_once("\n---\n")
            .context("unterminated ADR frontmatter")?;
        let meta: Value = serde_yaml_ng::from_str(yaml)?;
        if !meta.is_object() {
            bail!("ADR frontmatter must be a mapping");
        }
        for key in ["id", "title", "status", "domain", "superseded_by"] {
            if let Some(value) = meta.get(key)
                && !value.is_null()
                && !value.is_string()
            {
                bail!("ADR {key} must be a string or null");
            }
        }
        for key in ["rules", "respectGitignore"] {
            if let Some(value) = meta.get(key)
                && !value.is_boolean()
            {
                bail!("ADR {key} must be a boolean");
            }
        }
        if let Some(value) = meta.get("files")
            && !value
                .as_array()
                .is_some_and(|files| files.iter().all(Value::is_string))
        {
            bail!("ADR files must be an array of globs");
        }
        Ok(Document { meta, body })
    } else {
        Ok(Document {
            meta: Value::Null,
            body: text,
        })
    }
}
#[derive(Default)]
struct MarkdownState {
    fence: Option<(char, usize)>,
    comment: bool,
    quote: bool,
}
impl MarkdownState {
    fn visible(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let indented = line.starts_with('\t') || line.len() - trimmed.len() >= 4;
        let marker = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'));
        let length = marker.map_or(0, |marker| {
            trimmed.chars().take_while(|c| *c == marker).count()
        });
        if let Some((open, count)) = self.fence {
            if !indented
                && marker == Some(open)
                && length >= count
                && trimmed[length..].trim().is_empty()
            {
                self.fence = None;
            }
            return false;
        }
        let mut commented = self.comment;
        let mut remaining = line;
        loop {
            let delimiter = if self.comment { "-->" } else { "<!--" };
            let Some((_, after)) = remaining.split_once(delimiter) else {
                break;
            };
            self.comment = !self.comment;
            commented = true;
            remaining = after;
        }
        if commented {
            return false;
        }
        if self.quote {
            if trimmed.is_empty() {
                self.quote = false;
            }
            return false;
        }
        if trimmed.starts_with('>') {
            self.quote = true;
            return false;
        }
        if indented {
            return false;
        }
        if length >= 3 {
            self.fence = marker.map(|marker| (marker, length));
            return false;
        }
        true
    }
}
fn section<'a>(body: &'a str, names: &[&str]) -> &'a str {
    let mut offset = 0;
    let mut start = None;
    let mut markdown = MarkdownState::default();
    for line in body.split_inclusive('\n') {
        let visible = markdown.visible(line);
        if visible && let Some(heading) = line.strip_prefix("## ") {
            if let Some(start) = start {
                return &body[start..offset];
            }
            if names
                .iter()
                .any(|name| heading.trim().eq_ignore_ascii_case(name))
            {
                start = Some(offset + line.len());
            }
        }
        offset += line.len();
    }
    start.map_or("", |start| &body[start..])
}
fn lifecycle(doc: &Document<'_>) -> Result<String> {
    let front = doc.meta["status"].as_str().map(status_word);
    let body = section(doc.body, &["Status"])
        .split_whitespace()
        .next()
        .map(status_word);
    let history = section(doc.body, &["Status History"]);
    let mut statuses = Vec::new();
    for line in history.lines() {
        if let Some(status) = line.trim().strip_prefix("status:") {
            statuses.push(status_word(status.trim().trim_matches(['\'', '"'])));
        }
    }
    let latest = statuses.last().cloned();
    let given: Vec<_> = [front, body, latest].into_iter().flatten().collect();
    if given.windows(2).any(|pair| pair[0] != pair[1]) {
        bail!("conflicting ADR status/frontmatter/history");
    }
    if doc
        .meta
        .get("superseded_by")
        .is_some_and(|v| !v.is_null() && v != "")
    {
        return Ok("superseded".into());
    }
    Ok(given.first().cloned().unwrap_or_else(|| "unknown".into()))
}
fn status_word(value: &str) -> String {
    value
        .trim_matches(|c: char| !c.is_alphanumeric())
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn identity(doc: &Document<'_>, path: &Path) -> (String, String) {
    let heading = doc
        .body
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or("")
        .trim();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let id = doc.meta["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let first = heading.split_whitespace().next().unwrap_or("");
            if first.starts_with("ADR-") {
                first.to_owned()
            } else {
                format!("ADR-{}", stem.split('-').next().unwrap_or("unknown"))
            }
        });
    let title = doc.meta["title"]
        .as_str()
        .unwrap_or(heading)
        .trim_start_matches(&id)
        .trim()
        .to_owned();
    (id, title)
}
fn endpoint(raw: &str, modules: &BTreeMap<String, Vec<String>>) -> Option<String> {
    let raw = raw.trim().trim_end_matches('.').trim();
    let quoted = raw.len() >= 2
        && ((raw.starts_with('`') && raw.ends_with('`'))
            || (raw.starts_with('"') && raw.ends_with('"')));
    let name = if quoted { &raw[1..raw.len() - 1] } else { raw };
    let paths = modules.get(name);
    let path = if let Some(paths) = paths {
        if paths.len() != 1 {
            return None;
        }
        paths[0].as_str()
    } else {
        if !quoted {
            return None;
        }
        name
    };
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':', '<', '>'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|v| v == ".." || v == "." || v.is_empty())
        || (paths.is_none() && !path.contains('/'))
    {
        return None;
    }
    if crate::collect::globset(&[path.to_owned()]).is_err() {
        return None;
    }
    Some(path.to_owned())
}

fn reviewed_lazy_packages(doc: &Document<'_>, decision: &str) -> bool {
    // shortcut: exact reviewed ARCH-018 Decision contract; review new wording before expanding this adapter.
    let sha: String = Sha256::digest(decision.trim().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    doc.meta["files"] == serde_json::json!(["src/**/*.ts"])
        && sha == "010061f97483270f60c413113783a510eb2b76a23412ec76ed55208da82e0653"
}

fn notice(report: &mut Report, source: &str, message: impl Into<String>) {
    report.informational.push(Notice {
        source: source.into(),
        message: message.into(),
    });
}

pub fn import(root: &Path, dir: &Path) -> Result<Report> {
    let root = fs::canonicalize(root)?;
    let dir = safe_path(&root, dir)?;
    let modules = if root.join("layout.yaml").exists() {
        crate::rules::load(&root.join("layout.yaml"))?
            .doc
            .modules
            .into_iter()
            .fold(
                BTreeMap::<String, Vec<String>>::new(),
                |mut modules, rule| {
                    modules.entry(rule.id).or_default().push(rule.path);
                    modules
                },
            )
    } else {
        BTreeMap::new()
    };
    let mut files = Vec::new();
    markdown_files(&dir, &mut files)?;
    files.sort();
    let mut report = Report {
        schema: "archgraph/adr-import/v1",
        documents: files.len(),
        proposals: Vec::new(),
        informational: Vec::new(),
    };
    for path in files {
        safe_path(&root, &path)?;
        let source = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&path)?;
        let source_sha256: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let text = String::from_utf8(bytes)?.replace("\r\n", "\n");
        let doc = match document(&text) {
            Ok(doc) => doc,
            Err(error) => {
                notice(&mut report, &source, format!("informational only: {error}"));
                continue;
            }
        };
        let status = match lifecycle(&doc) {
            Ok(status) => status,
            Err(error) => {
                notice(&mut report, &source, format!("informational only: {error}"));
                continue;
            }
        };
        if !matches!(status.as_str(), "accepted" | "unknown") {
            notice(
                &mut report,
                &source,
                format!("informational only: lifecycle {status}"),
            );
            continue;
        }
        let (id, title) = identity(&doc, &path);
        let decisions = section(doc.body, &["Decision", "Decision Outcome"]);
        let start = report.proposals.len();
        let mut constraints = Vec::new();
        let mut markdown = MarkdownState::default();
        let mut visible_decision = String::new();
        for line in decisions.lines() {
            if !markdown.visible(line) {
                continue;
            }
            if line.starts_with("### ") {
                break;
            }
            visible_decision.push_str(line);
            visible_decision.push('\n');
            let line = line.trim().trim_start_matches(['-', '*']).trim();
            let Some((from, to)) = line.split_once(" must not import ") else {
                continue;
            };
            if let (Some(from), Some(to)) = (endpoint(from, &modules), endpoint(to, &modules)) {
                constraints.push((from, to, None));
            }
        }
        let reviewed_packages = reviewed_lazy_packages(&doc, &visible_decision);
        if reviewed_packages {
            for package in ["inquirer", "posthog-node", "@sentry/*"] {
                constraints.push((
                    "src/**/*.ts".into(),
                    format!("pkg:{package}"),
                    Some(vec![crate::model::EdgeKind::Static]),
                ));
            }
            notice(
                &mut report,
                &source,
                "partial mapping: named package classes only, static runtime imports; future unspecified heavy dependencies require review",
            );
        }
        for (from, to, kinds) in constraints {
            let kind_tag = if kinds.is_some() { "\0runtime" } else { "" };
            let hash: String =
                Sha256::digest(format!("{source}\0{from}\0{to}{kind_tag}").as_bytes())
                    .iter()
                    .take(6)
                    .map(|b| format!("{b:02x}"))
                    .collect();
            let proposal_id = format!("adr-{hash}");
            if report.proposals.iter().any(|p| p.id == proposal_id) {
                continue;
            }
            report.proposals.push(Proposal {
                id: proposal_id,
                from,
                to,
                reason: format!("{id} {title}"),
                source: source.clone(),
                lifecycle: status.clone(),
                kinds,
                source_sha256: Some(source_sha256.clone()),
            });
        }
        if !reviewed_packages
            && (doc.meta.get("files").is_some() || doc.meta.get("components").is_some())
        {
            notice(
                &mut report,
                &source,
                "scope metadata retained as informational only; it does not imply an import prohibition",
            );
        }
        if !section(doc.body, &["Enforcement"]).is_empty() {
            notice(
                &mut report,
                &source,
                "Enforcement regex checks require source-text semantics; not translated to import edges",
            );
        }
        if doc.meta["rules"] == true {
            notice(
                &mut report,
                &source,
                "companion .rules.ts is not executed or translated",
            );
        }
        if status == "unknown" {
            notice(
                &mut report,
                &source,
                "lifecycle unknown; review before applying proposals",
            );
        }
        if start == report.proposals.len() {
            notice(
                &mut report,
                &source,
                "informational only: no unambiguous import prohibition in Decision or Decision Outcome",
            );
        }
    }
    Ok(report)
}

pub fn yaml(report: &Report) -> Result<String> {
    let mut result = String::from("# imported from ADRs; review before applying\n");
    let rules: Vec<_> = report.proposals.iter().map(Proposal::rule).collect();
    result.push_str(&serde_yaml_ng::to_string(
        &serde_json::json!({"forbid":rules}),
    )?);
    for note in &report.informational {
        result.push_str(&format!(
            "# {}: {}\n",
            note.source,
            note.message.replace(['\n', '\r'], " ")
        ));
    }
    Ok(result)
}

fn verify_source(root: &Path, proposal: &Proposal) -> Result<()> {
    let path = safe_path(root, Path::new(&proposal.source))?;
    let bytes = fs::read(path)?;
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if proposal.source_sha256.as_deref() != Some(&sha) {
        bail!(
            "source ADR changed or has no content proof: {}; re-import before applying",
            proposal.source
        );
    }
    let text = String::from_utf8(bytes)?.replace("\r\n", "\n");
    let status = lifecycle(&document(&text)?)?;
    if !matches!(status.as_str(), "accepted" | "unknown") || status != proposal.lifecycle {
        bail!(
            "source ADR lifecycle changed: {}; re-import before applying",
            proposal.source
        );
    }
    Ok(())
}
fn publish(
    root: &Path,
    path: &Path,
    original: Option<&[u8]>,
    candidate: &[u8],
    proposals: &[&Proposal],
) -> Result<()> {
    let current = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current.as_deref() != original {
        bail!("layout.yaml changed after proposal preflight; preserve that edit and retry import");
    }
    for proposal in proposals {
        verify_source(root, proposal)?;
    }
    crate::history::write_atomic(path, candidate)
}

pub fn apply(root: &Path, report: &Report) -> Result<usize> {
    let root = fs::canonicalize(root)?;
    let path = root.join("layout.yaml");
    let existed = path.exists();
    if existed {
        safe_path(&root, &path)?;
    }
    let original = if existed {
        fs::read_to_string(&path)?
    } else {
        "version: 1\n".into()
    };
    if path.exists() {
        crate::rules::load(&path)?;
    }
    let mut value: Value = serde_yaml_ng::from_str(&original)?;
    let existing: BTreeSet<_> = value["forbid"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["id"].as_str())
        .map(str::to_owned)
        .collect();
    let fresh: Vec<_> = report
        .proposals
        .iter()
        .filter(|p| !existing.contains(&p.id))
        .collect();
    if fresh.is_empty() {
        return Ok(0);
    }
    for proposal in &fresh {
        verify_source(&root, proposal)?;
    }
    let additions: Vec<_> = fresh.iter().map(|p| p.rule()).collect();
    let map = value.as_object_mut().context("layout must be a mapping")?;
    let sequence = map
        .entry("forbid")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("forbid must be a sequence")?;
    sequence.extend(additions.clone());
    let rules: crate::rules::Rules = serde_json::from_value(value)?;
    crate::rules::Compiled::new(&rules)?;
    let item_yaml = serde_yaml_ng::to_string(&additions)?;
    let mut candidate = original.clone();
    let mut inserted = false;
    let lines: Vec<_> = original.split_inclusive('\n').collect();
    let header: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("forbid:"))
        .collect();
    if header.len() > 1 {
        bail!("ambiguous duplicate forbid sections");
    }
    if let Some((index, line)) = header.first() {
        if line.trim_end() != "forbid:" {
            bail!("--apply requires a block-style forbid sequence; review stdout instead");
        }
        let end = lines[index + 1..]
            .iter()
            .position(|line| !line.trim().is_empty() && !line.starts_with([' ', '\t', '#', '-']))
            .map_or(lines.len(), |at| index + 1 + at);
        let offset: usize = lines[..end].iter().map(|line| line.len()).sum();
        let indent = lines[index + 1..end]
            .iter()
            .find_map(|line| {
                let trimmed = line.trim_start();
                trimmed
                    .starts_with("- ")
                    .then_some(&line[..line.len() - trimmed.len()])
            })
            .unwrap_or("  ");
        let mut append = format!("\n{indent}# imported from ADRs\n");
        for line in item_yaml.lines() {
            append.push_str(indent);
            append.push_str(line);
            append.push('\n');
        }
        candidate.insert_str(offset, &append);
        inserted = true;
    }
    if !inserted {
        candidate.push_str("\n# imported from ADRs\nforbid:\n");
        candidate.push_str(&item_yaml);
    }
    // Reject anchors and aliases rather than silently changing their scope while appending.
    if original.lines().any(|line| {
        line.split_whitespace()
            .any(|word| word.starts_with('&') || word.starts_with('*'))
    }) {
        bail!("--apply does not edit YAML anchors or aliases");
    }
    let parsed: crate::rules::Rules = serde_yaml_ng::from_str(&candidate)?;
    crate::rules::Compiled::new(&parsed)?;
    publish(
        &root,
        &path,
        existed.then_some(original.as_bytes()),
        candidate.as_bytes(),
        &fresh,
    )?;
    Ok(fresh.len())
}

pub fn check(root: &Path, require_source: bool) -> Result<CheckReport> {
    let root = fs::canonicalize(root)?;
    crate::rules::load(&root.join("layout.yaml"))?;
    let raw: Value = serde_yaml_ng::from_str(&fs::read_to_string(root.join("layout.yaml"))?)?;
    let mut report = CheckReport {
        schema: "archgraph/adr-check/v1",
        stale: Vec::new(),
        missing: Vec::new(),
        unknown: Vec::new(),
        unsourced: require_source.then(Vec::new),
    };
    if let Some(unsourced) = &mut report.unsourced {
        let rules: Vec<_> =
            ["forbid", "allow"]
                .into_iter()
                .flat_map(|key| {
                    raw[key].as_array().into_iter().flatten().enumerate().map(
                        move |(index, item)| {
                            let label = item["id"]
                                .as_str()
                                .map_or_else(|| format!("{key}[{index}]"), str::to_owned);
                            (label, item["source"].is_string())
                        },
                    )
                })
                .collect();
        // Rules sharing an id are one claim, sourced when any of them is (as in `Rules::sources`).
        for (label, _) in &rules {
            if !rules
                .iter()
                .any(|(other, sourced)| other == label && *sourced)
                && !unsourced.contains(label)
            {
                unsourced.push(label.clone());
            }
        }
    }
    let sources: BTreeSet<_> = ["forbid", "allow"]
        .into_iter()
        .flat_map(|key| raw[key].as_array().into_iter().flatten())
        .filter_map(|item| item["source"].as_str())
        .collect();
    for source in sources {
        let entry = |message: String| Notice {
            source: source.to_owned(),
            message,
        };
        let path = match safe_path(&root, Path::new(source)) {
            Ok(path) => path,
            Err(error) => {
                report
                    .missing
                    .push(entry(format!("source unavailable: {error}")));
                continue;
            }
        };
        let text = match fs::read_to_string(path) {
            Ok(text) => text.replace("\r\n", "\n"),
            Err(error) => {
                report
                    .unknown
                    .push(entry(format!("source unreadable: {error}")));
                continue;
            }
        };
        let doc = match document(&text) {
            Ok(doc) => doc,
            Err(error) => {
                report
                    .unknown
                    .push(entry(format!("source malformed: {error}")));
                continue;
            }
        };
        match lifecycle(&doc) {
            Ok(status)
                if matches!(
                    status.as_str(),
                    "superseded" | "deprecated" | "rejected" | "proposed"
                ) =>
            {
                report.stale.push(entry(format!(
                    "rule may be stale: source lifecycle {status}"
                )))
            }
            Ok(status) if status == "accepted" => (),
            Ok(status) => report
                .unknown
                .push(entry(format!("source lifecycle {status}"))),
            Err(error) => report.unknown.push(entry(error.to_string())),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TempDir;
    fn decision(root: &TempDir, text: &str) {
        fs::create_dir_all(root.path().join("docs")).unwrap();
        fs::write(root.path().join("docs/ADR-001.md"), text).unwrap();
    }
    const DECISION: &str = "# ADR-001 Boundary\n\n## Status\nAccepted\n\n## Decision\n`src/domain/**` must not import `src/adapters/**`.\n";

    #[test]
    fn eighteen_format_fixtures_match_reviewed_goldens() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adrs");
        for format in ["madr", "adr-kit", "archgate"] {
            let actual = serde_json::to_value(import(&root, Path::new(format)).unwrap()).unwrap();
            let expected: Value = serde_json::from_slice(
                &fs::read(root.join(format!("{format}.golden.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(actual, expected, "{format}");
            assert_eq!(actual["documents"], 6);
        }
    }
    #[test]
    fn excludes_fenced_and_alternative_decision_text() {
        let root = TempDir::new("adr-import").unwrap();
        decision(
            &root,
            "# ADR-001 Guard\n## Decision\n```text\n`src/a/**` must not import `src/b/**`.\n```\n### Rejected alternative\n`src/a/**` must not import `src/b/**`.\n",
        );
        assert!(
            import(root.path(), Path::new("docs"))
                .unwrap()
                .proposals
                .is_empty()
        );
    }
    #[test]
    fn ambiguous_module_ids_do_not_produce_inert_or_overbroad_rules() {
        let modules = BTreeMap::from([
            (
                "duplicate".into(),
                vec!["src/a/**".into(), "src/b/**".into()],
            ),
            ("captured".into(), vec!["src/<feature>/**".into()]),
            ("outside".into(), vec!["/tmp/**".into()]),
        ]);
        for name in ["duplicate", "captured", "outside", "`", "\""] {
            assert!(endpoint(name, &modules).is_none());
        }
    }

    #[test]
    fn conflicting_status_or_superseded_source_is_informational() {
        let root = TempDir::new("adr-import").unwrap();
        decision(&root, &format!("---\nstatus: Deprecated\n---\n{DECISION}"));
        let report = import(root.path(), Path::new("docs")).unwrap();
        assert!(report.proposals.is_empty());
        assert!(report.informational[0].message.contains("conflicting"));
        decision(
            &root,
            &format!("---\nstatus: Accepted\nsuperseded_by: ADR-002\n---\n{DECISION}"),
        );
        assert!(
            import(root.path(), Path::new("docs"))
                .unwrap()
                .proposals
                .is_empty()
        );
    }
    #[test]
    fn apply_is_idempotent_and_preserves_foreign_bytes_and_permissions() {
        let root = TempDir::new("adr-import").unwrap();
        decision(&root, DECISION);
        let before = "# custom header\nversion: 1\nforbid:\n- id: keep\n  from: src/x/**\n  to: src/y/**\n# custom footer\nallow: []\n";
        let path = root.path().join("layout.yaml");
        fs::write(&path, before).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let report = import(root.path(), Path::new("docs")).unwrap();
        assert_eq!(apply(root.path(), &report).unwrap(), 1);
        let after = fs::read_to_string(&path).unwrap();
        let begin = after.find("\n# imported from ADRs\n").unwrap();
        let end = after[begin..].find("allow: []").unwrap() + begin;
        assert_eq!(format!("{}{}", &after[..begin], &after[end..]), before);
        assert_eq!(apply(root.path(), &report).unwrap(), 0);
        assert_eq!(after, fs::read_to_string(&path).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        decision(&root, &DECISION.replace("Accepted", "Deprecated"));
        assert_eq!(check(root.path(), false).unwrap().stale.len(), 1);
        fs::remove_file(root.path().join("docs/ADR-001.md")).unwrap();
        assert_eq!(check(root.path(), false).unwrap().missing.len(), 1);
    }
    #[test]
    fn apply_refuses_flow_sections_without_mutation() {
        let root = TempDir::new("adr-import").unwrap();
        decision(&root, DECISION);
        let path = root.path().join("layout.yaml");
        let original = "version: 1\nforbid: []\n";
        fs::write(&path, original).unwrap();
        let report = import(root.path(), Path::new("docs")).unwrap();
        assert!(
            apply(root.path(), &report)
                .unwrap_err()
                .to_string()
                .contains("block-style")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }
    #[test]
    fn duplicate_numbered_adrs_get_distinct_proposal_ids() {
        let root = TempDir::new("adr-import").unwrap();
        decision(&root, DECISION);
        fs::write(root.path().join("docs/ADR-001-copy.md"), DECISION).unwrap();
        let report = import(root.path(), Path::new("docs")).unwrap();
        assert_eq!(report.proposals.len(), 2);
        assert_ne!(report.proposals[0].id, report.proposals[1].id);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_source_and_parent_escape_are_rejected() {
        let root = TempDir::new("adr-import").unwrap();
        decision(&root, DECISION);
        std::os::unix::fs::symlink(
            root.path().join("docs/ADR-001.md"),
            root.path().join("docs/linked.md"),
        )
        .unwrap();
        assert!(import(root.path(), Path::new("docs")).is_err());
        assert!(import(root.path(), Path::new("../outside")).is_err());
    }
    #[test]
    fn comments_code_and_invalid_globs_never_become_active_proposals() {
        let root = TempDir::new("adr-markdown-negative").unwrap();
        let ban = "`src/a/**` must not import `src/b/**`.";
        for body in [
            format!("````text\n```\n{ban}\n```\n````"),
            format!("<!--\n{ban}\n-->"),
            format!("<!-- first --> <!-- second\n{ban}\n-->"),
            format!("<!-- prior\n--> <!-- next\n{ban}\n-->"),
            format!("    {ban}"),
            format!("\t{ban}"),
            format!("> {ban}"),
            format!("> Quoted example:\n{ban}"),
            "`src/[.ts` must not import `src/b/**`.".into(),
        ] {
            decision(&root, &format!("# ADR-001 Examples\n## Decision\n{body}\n"));
            assert!(
                import(root.path(), Path::new("docs"))
                    .unwrap()
                    .proposals
                    .is_empty(),
                "{body}"
            );
        }
        decision(
            &root,
            &format!("# ADR-001 Boundary\n## Decision\n<!-- first --> <!-- second -->\n{ban}\n"),
        );
        assert_eq!(
            import(root.path(), Path::new("docs"))
                .unwrap()
                .proposals
                .len(),
            1
        );
    }
    #[test]
    fn concurrent_layout_and_changed_source_are_preserved() {
        let root = TempDir::new("adr-preflight").unwrap();
        decision(&root, DECISION);
        let report = import(root.path(), Path::new("docs")).unwrap();
        let path = root.path().join("layout.yaml");
        fs::write(&path, "version: 1\n# concurrent edit\n").unwrap();
        assert!(
            publish(
                root.path(),
                &path,
                Some(b"version: 1\n"),
                b"replacement",
                &[]
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "version: 1\n# concurrent edit\n"
        );
        for changed in [
            format!("{DECISION}\nA newer decision."),
            DECISION.replace("Accepted", "Superseded"),
        ] {
            decision(&root, &changed);
            assert!(apply(root.path(), &report).is_err());
            assert!(
                publish(
                    root.path(),
                    &path,
                    Some(b"version: 1\n# concurrent edit\n"),
                    b"replacement",
                    &[&report.proposals[0]]
                )
                .is_err()
            );
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                "version: 1\n# concurrent edit\n"
            );
        }
    }
    #[test]
    fn pinned_archgate_lazy_dependency_adapter_matches_reviewed_golden() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adrs");
        let actual =
            serde_json::to_value(import(&root, Path::new("archgate-real")).unwrap()).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(root.join("archgate-real.golden.json")).unwrap())
                .unwrap();
        assert_eq!(actual, expected);
        let fixture = TempDir::new("adr-contract-exception").unwrap();
        let text =
            fs::read_to_string(root.join("archgate-real/ARCH-018-lazy-load-heavy-dependencies.md"))
                .unwrap();
        decision(
            &fixture,
            &text.replace(
                "## Do's and Don'ts",
                "Exception: static inquirer imports are allowed.\n\n## Do's and Don'ts",
            ),
        );
        assert!(
            import(fixture.path(), Path::new("docs"))
                .unwrap()
                .proposals
                .is_empty()
        );
    }
}
