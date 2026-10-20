//! The target check: one verdict per claim (forbid rules sharing an `id`, one with a `reason`),
//! per slice (a value of the rules' `<name>` capture), the slices that conform, and what the
//! `moves` still pending cost per slice.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::model::{Document, Edge, Language, Link, Move};
use crate::modules::dir_of;
use crate::rules::Compiled;

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub claims: Vec<Claim>,
    /// Every slice a move rule touches; a slice with no `files` has moved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<SliceMove>,
    pub progress: Progress,
    /// Notes on the whole target, such as a move rule matching no file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SliceMove {
    pub slice: String,
    /// Files still to move, with their destinations.
    pub files: Vec<Move>,
    pub import_lines: usize,
    /// Go: the unexported names to export, those of bound pairs whose files land in different
    /// folders; `None` when the collector cannot tell (native Go, other languages).
    pub names: Option<Vec<Name>>,
    /// Go: bound pairs (a method and its type, platform variants) the moves split.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub together: Vec<Link>,
    /// Go package cycles that applying every pending move would create, touching this slice's
    /// source or destination folders.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_cycles: Vec<Link>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Name {
    pub name: String,
    pub declared_in: String,
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Claim {
    pub id: String,
    pub reason: String,
    pub source: Option<String>,
    pub verdict: ClaimVerdict,
    /// Bound slices only: those where one of the claim's rules has files on both sides.
    pub slices: Vec<Slice>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimVerdict {
    Holds,
    HoldsIn,
    Broken,
    Vacuous,
}

#[derive(Debug, Clone, Serialize)]
pub struct Slice {
    /// The capture value; `*` for rules without a capture.
    pub slice: String,
    pub files: Vec<String>,
    pub broken: Vec<Link>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// Captured slices that hold in every claim bound to them.
    pub conforming: Count,
    /// Captured slices with no file left to move, when the rules have `moves`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved: Option<Count>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Count {
    pub done: usize,
    pub of: usize,
}

type Files = BTreeSet<String>;
type Imports = BTreeSet<(String, String)>;

/// `None` when the rules state no claim and no move.
pub fn analyze(doc: &Document, rules: &Compiled) -> Option<Report> {
    let policy = &doc.config.rules.as_ref()?.doc;
    let forbid = &policy.forbid;
    let mut ids: Vec<&str> = Vec::new();
    for rule in forbid {
        if let Some(id) = rule.id.as_deref()
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids.retain(|id| {
        forbid
            .iter()
            .any(|r| r.id.as_deref() == Some(id) && r.reason.is_some())
    });
    if ids.is_empty() && policy.moves.is_empty() {
        return None;
    }
    let files: Vec<&str> = doc.files.iter().map(|f| f.path.as_str()).collect();
    let edges: Vec<&Edge> = doc.edges.iter().chain(&doc.external_edges).collect();
    let externals = doc
        .external_nodes
        .iter()
        .map(|n| (n.id.clone(), n.package.clone()))
        .collect();
    let evidence = rules.forbid_slices(&files, &edges, &externals);
    // Per capture value, whether it holds in every claim bound to it.
    let mut conforming: BTreeMap<String, bool> = BTreeMap::new();
    let claims = ids
        .into_iter()
        .map(|id| {
            let mut notes = Vec::new();
            let mut slices: BTreeMap<Option<String>, (Files, Imports)> = BTreeMap::new();
            let mut bound = BTreeSet::new();
            let rules = forbid
                .iter()
                .enumerate()
                .filter(|(_, r)| r.id.as_deref() == Some(id));
            for (i, _) in rules.clone() {
                if evidence[i].is_empty() {
                    notes.push(format!("forbid[{i}] `from` matches no file"));
                } else if evidence[i]
                    .values()
                    .all(|s| s.to.is_empty() && s.hits.is_empty())
                {
                    notes.push(format!("forbid[{i}] `to` matches no file"));
                }
                for (value, s) in &evidence[i] {
                    if !s.hits.is_empty() || (!s.from.is_empty() && !s.to.is_empty()) {
                        bound.insert(value.clone());
                    }
                    let (files, broken) = slices.entry(value.clone()).or_default();
                    files.extend(s.from.iter().chain(&s.to).cloned());
                    broken.extend(s.hits.iter().map(|h| (h.from.clone(), h.to.clone())));
                }
            }
            slices.retain(|value, _| bound.contains(value));
            for (value, (_, broken)) in &slices {
                if let Some(value) = value {
                    *conforming.entry(value.clone()).or_insert(true) &= broken.is_empty();
                }
            }
            let held = slices
                .values()
                .filter(|(_, broken)| broken.is_empty())
                .count();
            let first = |field: fn(&crate::rules::Forbid) -> Option<&String>| {
                rules.clone().find_map(|(_, r)| field(r)).cloned()
            };
            Claim {
                id: id.to_owned(),
                reason: first(|r| r.reason.as_ref()).unwrap_or_default(),
                source: first(|r| r.source.as_ref()),
                verdict: match (held, slices.len()) {
                    (_, 0) => ClaimVerdict::Vacuous,
                    (held, all) if held == all => ClaimVerdict::Holds,
                    (0, _) => ClaimVerdict::Broken,
                    _ => ClaimVerdict::HoldsIn,
                },
                slices: slices
                    .into_iter()
                    .map(|(value, (files, broken))| Slice {
                        slice: value.unwrap_or_else(|| "*".into()),
                        files: files.into_iter().collect(),
                        broken: broken
                            .into_iter()
                            .map(|(from, to)| Link { from, to })
                            .collect(),
                    })
                    .collect(),
                notes,
            }
        })
        .collect();
    let mut notes = Vec::new();
    let mut pending: BTreeMap<Option<String>, Vec<Move>> = BTreeMap::new();
    for (i, slices) in rules.resolve_moves(&files).into_iter().enumerate() {
        if slices.is_empty() {
            notes.push(format!("moves[{i}] matches no file"));
        }
        for (value, evidence) in slices {
            pending.entry(value).or_default().extend(evidence.pending);
        }
    }
    let captured = pending.iter().filter(|(value, _)| value.is_some());
    let moved = (!policy.moves.is_empty()).then(|| Count {
        done: captured
            .clone()
            .filter(|(_, files)| files.is_empty())
            .count(),
        of: captured.count(),
    });
    let all: Vec<Move> = pending.values().flatten().cloned().collect();
    let cycles = if doc.source.language == Language::Go && !all.is_empty() {
        match crate::simulate::apply(doc, Some(rules), &all) {
            Ok(after) => crate::graph::new_cyclic_import_pairs(doc, &after),
            Err(err) => {
                notes.push(format!("moves not simulated: {err:#}"));
                BTreeSet::new()
            }
        }
    } else {
        BTreeSet::new()
    };
    let go_types = doc.source.collector.name == "go-types";
    if doc.source.language == Language::Go && !go_types && !all.is_empty() {
        notes.push(format!(
            "names to export unknown for {}",
            doc.source.collector.name
        ));
    }
    let moves = pending
        .into_iter()
        .map(|(value, mut files)| {
            files.sort_by(|a, b| a.file.cmp(&b.file));
            let dirs: HashSet<&str> = files
                .iter()
                .flat_map(|m| [dir_of(&m.file), dir_of(&m.to)])
                .collect();
            let new_cycles = cycles
                .iter()
                .filter(|(from, to)| dirs.contains(dir_of(from)) || dirs.contains(dir_of(to)))
                .map(|(from, to)| Link {
                    from: from.clone(),
                    to: to.clone(),
                })
                .collect();
            let renamed: HashMap<&str, &str> = files
                .iter()
                .map(|m| (m.file.as_str(), m.to.as_str()))
                .collect();
            let split: Vec<_> = doc
                .source
                .collector
                .bound
                .iter()
                .filter(|p| {
                    let after = |path| renamed.get(path).copied().unwrap_or(path);
                    dir_of(after(&p.from)) != dir_of(after(&p.to))
                })
                .collect();
            let mut names: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
            for p in &split {
                for name in &p.names {
                    names.entry((name, &p.to)).or_default().push(p.from.clone());
                }
            }
            SliceMove {
                slice: value.unwrap_or_else(|| "*".into()),
                import_lines: import_lines(doc, &files),
                names: go_types.then(|| {
                    names
                        .into_iter()
                        .map(|((name, declared_in), used_by)| Name {
                            name: name.to_owned(),
                            declared_in: declared_in.to_owned(),
                            used_by,
                        })
                        .collect()
                }),
                together: split
                    .iter()
                    .filter(|p| p.together)
                    .map(|p| Link {
                        from: p.from.clone(),
                        to: p.to.clone(),
                    })
                    .collect(),
                files,
                new_cycles,
            }
        })
        .collect();
    Some(Report {
        claims,
        moves,
        progress: Progress {
            conforming: Count {
                done: conforming.values().filter(|held| **held).count(),
                of: conforming.len(),
            },
            moved,
        },
        notes,
    })
}

/// The import lines `moves` rewrite. Go: (importer, imported folder) pairs across folders after
/// the moves that were not there before. Other languages: import records (edges when the
/// collector keeps none) whose importer or target moves.
fn import_lines(doc: &Document, moves: &[Move]) -> usize {
    let renamed: HashMap<&str, &str> = moves
        .iter()
        .map(|m| (m.file.as_str(), m.to.as_str()))
        .collect();
    let moved = |path: &str| renamed.contains_key(path);
    if doc.source.language == Language::Go {
        let rename = |path| renamed.get(path).copied().unwrap_or(path);
        let pairs = |after: bool| -> HashSet<(&str, &str)> {
            doc.edges
                .iter()
                .filter_map(|e| {
                    let (from, to) = if after {
                        (rename(&e.from), rename(&e.to))
                    } else {
                        (e.from.as_str(), e.to.as_str())
                    };
                    (dir_of(from) != dir_of(to)).then(|| (rename(&e.from), dir_of(to)))
                })
                .collect()
        };
        return pairs(true).difference(&pairs(false)).count();
    }
    match &doc.imports {
        Some(imports) => imports
            .iter()
            .filter(|i| moved(&i.from) || i.targets.iter().any(|t| moved(t)))
            .count(),
        None => doc
            .edges
            .iter()
            .filter(|e| moved(&e.from) || moved(&e.to))
            .count(),
    }
}

/// The `check` lines: a verdict per claim with up to 20 broken imports, the pending moves, notes,
/// then progress.
pub fn text(report: &Report) -> String {
    let mut out = String::new();
    for claim in &report.claims {
        let broken: Vec<&Slice> = claim
            .slices
            .iter()
            .filter(|s| !s.broken.is_empty())
            .collect();
        let named = claim.slices.iter().any(|s| s.slice != "*");
        let names = || {
            broken
                .iter()
                .map(|s| s.slice.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let verdict = match claim.verdict {
            ClaimVerdict::Vacuous => "vacuous".to_owned(),
            ClaimVerdict::Holds => "holds".to_owned(),
            ClaimVerdict::HoldsIn => format!(
                "holds in {} of {} slices; broken in {}",
                claim.slices.len() - broken.len(),
                claim.slices.len(),
                names()
            ),
            ClaimVerdict::Broken if named => format!("broken in {}", names()),
            ClaimVerdict::Broken => "broken".to_owned(),
        };
        out += &format!("claim {}: {verdict} reason={:?}", claim.id, claim.reason);
        if let Some(source) = &claim.source {
            out += &format!(" source={source:?}");
        }
        out += "\n";
        let imports: Vec<&Link> = broken.iter().flat_map(|s| &s.broken).collect();
        for link in imports.iter().take(20) {
            out += &format!("  {} -> {}\n", link.from, link.to);
        }
        if imports.len() > 20 {
            out += &format!("  ... and {} more\n", imports.len() - 20);
        }
        for note in &claim.notes {
            out += &format!("  note: {note}\n");
        }
    }
    for slice in report.moves.iter().filter(|m| !m.files.is_empty()) {
        out += &format!(
            "move {}: {}, {}",
            slice.slice,
            crate::diff::count(slice.files.len(), "file"),
            crate::diff::count(slice.import_lines, "import line")
        );
        if let Some(names) = slice.names.as_ref().filter(|n| !n.is_empty()) {
            out += &format!(", {} to export", crate::diff::count(names.len(), "name"));
        }
        if !slice.new_cycles.is_empty() {
            out += &format!(
                ", {}",
                crate::diff::count(slice.new_cycles.len(), "import") + " in new package cycles"
            );
        }
        out += "\n";
    }
    for note in &report.notes {
        out += &format!("note: {note}\n");
    }
    let Count { done, of } = report.progress.conforming;
    out += &format!("target: {done} of {of} slices conform");
    if let Some(Count { done, of }) = report.progress.moved {
        out += &format!(", {done} of {of} slices moved");
    }
    out + "\n"
}
