//! Owned agent-rule blocks; foreign bytes remain outside the replacement span.
use std::fs;
use std::path::{Component, Path};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::RulesRef;

pub const FILES: [&str; 4] = [
    "CLAUDE.md",
    "AGENTS.md",
    ".cursorrules",
    ".github/copilot-instructions.md",
];
const START: &str = "<!-- archgraph:start -->";
const END: &str = "<!-- archgraph:end -->";

#[derive(Debug, Serialize)]
pub struct Conflict {
    pub path: String,
    pub diff: String,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub written: Vec<String>,
    pub stale: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// UTF-8 bytes / 4, matching context's deliberately rough estimate.
    pub estimated_tokens: usize,
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn text(value: &str) -> String {
    value.replace("<!--", "&lt;!--").replace(['\r', '\n'], " ")
}

fn lines(rules: &RulesRef) -> Vec<String> {
    let r = &rules.doc;
    let mut lines = Vec::new();
    if !r.layers.is_empty() {
        lines.push(format!(
            "Layers, foundation first: {}. Imports may stay within a layer or go left.{}",
            r.layers
                .iter()
                .map(|v| text(v))
                .collect::<Vec<_>>()
                .join(" -> "),
            r.layer_reason
                .as_ref()
                .map_or(String::new(), |why| format!(" Reason: {}", text(why)))
        ));
    }
    for rule in &r.forbid {
        lines.push(format!(
            "Forbid {} -> {}{}{}. Reason: {}{}",
            text(&rule.from),
            text(&rule.to),
            if rule.reachable {
                " (including indirect reachability)"
            } else {
                ""
            },
            rule.kinds.as_ref().map_or(String::new(), |k| format!(
                " kinds={}",
                serde_json::to_string(k).expect("import kinds serialize")
            )),
            text(rule.reason.as_deref().unwrap_or("not supplied")),
            rule.source
                .as_ref()
                .map_or(String::new(), |source| format!(" Source: {}", text(source)))
        ));
    }
    for rule in &r.allow {
        lines.push(format!(
            "Allow {} -> {}{}. Reason: {}{}",
            text(&rule.from),
            text(&rule.to),
            rule.kinds.as_ref().map_or(String::new(), |k| format!(
                " kinds={}",
                serde_json::to_string(k).expect("import kinds serialize")
            )),
            text(rule.reason.as_deref().unwrap_or("not supplied")),
            rule.source
                .as_ref()
                .map_or(String::new(), |source| format!(" Source: {}", text(source)))
        ));
    }
    for (module, entry) in &r.entrypoints {
        lines.push(format!(
            "Outside {} use exactly {}.",
            text(module),
            text(entry)
        ));
    }
    for rule in &r.public {
        lines.push(format!(
            "Public {}: {}{}.",
            text(&rule.module),
            rule.entries
                .iter()
                .map(|v| text(v))
                .collect::<Vec<_>>()
                .join(", "),
            rule.kinds.as_ref().map_or(String::new(), |k| format!(
                " kinds={}",
                serde_json::to_string(k).expect("import kinds serialize")
            ))
        ));
    }
    for (category, packages) in &r.pkg_category {
        lines.push(format!(
            "Package category {}: {}.",
            text(category),
            packages
                .iter()
                .map(|v| text(v))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for module in &r.modules {
        lines.push(format!(
            "Module {}: {}.",
            text(&module.id),
            text(&module.path)
        ));
    }
    if let Some(limits) = &r.early_smells {
        lines.push(format!(
            "File lines <= {}. Reason: {}",
            limits.file_size.max,
            text(&limits.file_size.reason)
        ));
        lines.push(format!(
            "Distinct file importers <= {}. Reason: {}",
            limits.fan_in.max,
            text(&limits.fan_in.reason)
        ));
        lines.push(format!(
            "Outside module importers <= {}% of production files. Reason: {}",
            limits.module_reach.max,
            text(&limits.module_reach.reason)
        ));
        if limits.every_file_in_layer.enabled {
            lines.push(format!(
                "Every production file belongs to a layer. Reason: {}",
                text(&limits.every_file_in_layer.reason)
            ));
        }
        if limits.single_implementation.enabled {
            lines.push(format!(
                "Review one observed explicit implementation (advisory). Reason: {}",
                text(&limits.single_implementation.reason)
            ));
        }
    }
    if let Some(default) = r.default {
        lines.push(format!("Unmatched imports: {default:?}."));
    }
    lines
}
fn block(rules: &RulesRef, full: bool, body: &str) -> String {
    let payload = format!(
        "<!-- archgraph:layout-sha256 {} -->\n<!-- archgraph:mode {} -->\n{body}",
        rules.sha256,
        if full { "full" } else { "brief" }
    );
    format!(
        "{START}\n<!-- archgraph:sha256 {} -->\n{payload}{END}",
        hash(payload.as_bytes())
    )
}
pub fn render(rules: &RulesRef, full: bool) -> String {
    let header = "Architecture from layout.yaml. Precedence: forbid, entrypoints, allow, public, layers, module/default.\n";
    let footer = "Before edits use connected archgraph MCP rules/explain, or `archgraph explain FILE --graph CURRENT_SCAN.json --for-agent` with a saved scan when MCP is unavailable. Installed hooks check new violations. Run archgraph sync-rules --full for all rules.\n";
    let all = lines(rules);
    let mut selected = Vec::new();
    for line in &all {
        selected.push(line.clone());
        let omitted = all.len() - selected.len();
        let body = format!(
            "{header}{}\n{} omitted rule lines; consult layout.yaml/MCP.\n{footer}",
            selected.join("\n"),
            omitted
        );
        if !full && block(rules, false, &body).len() > 2400 {
            selected.pop();
            break;
        }
    }
    let omitted = all.len() - selected.len();
    let omission = if omitted > 0 {
        format!("{omitted} omitted rule lines; consult layout.yaml/MCP.\n")
    } else {
        String::new()
    };
    block(
        rules,
        full,
        &format!("{header}{}\n{omission}{footer}", selected.join("\n")),
    )
}

fn path(root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        ensure!(
            matches!(component, Component::Normal(_)),
            "unsafe rule-file path"
        );
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(m) => ensure!(
                !m.is_symlink() && (m.is_file() || m.is_dir()),
                "refusing non-regular rule-file path {}",
                path.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}
fn matches(bytes: &[u8], needle: &str) -> Vec<usize> {
    bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, v)| (v == needle.as_bytes()).then_some(index))
        .collect()
}
fn span(bytes: &[u8]) -> Result<Option<(usize, usize)>> {
    let starts = matches(bytes, START);
    let ends = matches(bytes, END);
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    ensure!(
        starts.len() == 1 && ends.len() == 1 && starts[0] < ends[0],
        "malformed or duplicate archgraph fences; restore the owned block before syncing"
    );
    Ok(Some((starts[0], ends[0] + END.len())))
}
fn owned_mode(bytes: &[u8]) -> Option<bool> {
    let owned = std::str::from_utf8(bytes).ok()?;
    let content = owned
        .strip_prefix(START)?
        .strip_prefix('\n')?
        .strip_suffix(END)?;
    let (checksum, payload) = content.split_once('\n')?;
    let checksum = checksum
        .strip_prefix("<!-- archgraph:sha256 ")?
        .strip_suffix(" -->")?;
    if checksum != hash(payload.as_bytes()) {
        return None;
    }
    match payload.lines().nth(1)? {
        "<!-- archgraph:mode full -->" => Some(true),
        "<!-- archgraph:mode brief -->" => Some(false),
        _ => None,
    }
}
fn diff(name: &str, old: &[u8], new: &str) -> String {
    let old = String::from_utf8_lossy(old);
    format!(
        "--- {name}\n+++ {name} (generated)\n@@ -1,{} +1,{} @@\n{}{}",
        old.lines().count(),
        new.lines().count(),
        old.lines().map(|l| format!("-{l}\n")).collect::<String>(),
        new.lines().map(|l| format!("+{l}\n")).collect::<String>()
    )
}

fn write_updated(root: &Path, name: &str, original: &[u8], updated: &[u8]) -> Result<()> {
    let target = path(root, name)?;
    fs::create_dir_all(target.parent().context("missing rule-file parent")?)?;
    path(root, name)?;
    let current = match fs::read(&target) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(error) => return Err(error.into()),
    };
    ensure!(
        current == original,
        "{} changed after preflight; preserve that edit and retry sync",
        target.display()
    );
    crate::history::write_atomic(&target, updated)
}

/// Preflight every target before writing any file; hand-edited owned blocks are never replaced.
pub fn run(root: &Path, full: bool, check: bool) -> Result<Report> {
    let root = root.canonicalize()?;
    ensure!(root.is_dir(), "sync root must be a directory");
    let rules = crate::rules::load(&path(&root, "layout.yaml")?)?;
    let mut report = Report {
        schema: "archgraph/sync-rules/v1",
        written: vec![],
        stale: vec![],
        conflicts: vec![],
        estimated_tokens: 0,
    };
    let mut updates = Vec::new();
    for name in FILES {
        let target = path(&root, name)?;
        let old = match fs::read(&target) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e).with_context(|| format!("reading {}", target.display())),
        };
        let span = span(&old).with_context(|| format!("{}", target.display()))?;
        let mode = span.and_then(|(start, end)| owned_mode(&old[start..end]));
        let generated = render(
            &rules,
            if check && !full {
                mode.unwrap_or(false)
            } else {
                full
            },
        );
        report.estimated_tokens = report.estimated_tokens.max(generated.len().div_ceil(4));
        let updated = if let Some((start, end)) = span {
            if mode.is_none() {
                report.conflicts.push(Conflict {
                    path: name.into(),
                    diff: diff(name, &old[start..end], &generated),
                });
            }
            [&old[..start], generated.as_bytes(), &old[end..]].concat()
        } else {
            let separator = if !old.is_empty() && !old.ends_with(b"\n") {
                b"\n".as_slice()
            } else {
                b""
            };
            [old.as_slice(), separator, generated.as_bytes(), b"\n"].concat()
        };
        if updated != old {
            report.stale.push(name.into());
            updates.push((name, old, updated));
        }
    }
    if !check && report.conflicts.is_empty() {
        for (name, original, bytes) in updates {
            write_updated(&root, name, &original, &bytes)?;
            report.written.push(name.into());
        }
        report.stale.clear();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_change_after_preflight_is_preserved() {
        let dir = crate::collect::TempDir::new("sync-concurrent").unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, b"foreign before").unwrap();
        let original = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"concurrent foreign edit").unwrap();
        assert!(super::write_updated(dir.path(), "AGENTS.md", &original, b"replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"concurrent foreign edit");
    }
}
