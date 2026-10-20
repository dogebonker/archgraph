//! The target check: one verdict per claim (forbid rules sharing an `id`, one with a `reason`),
//! per slice (a value of the rules' `<name>` capture), and the slices that conform.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::model::{Document, Edge, Link};
use crate::rules::Compiled;

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub claims: Vec<Claim>,
    pub progress: Progress,
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
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Count {
    pub done: usize,
    pub of: usize,
}

type Files = BTreeSet<String>;
type Imports = BTreeSet<(String, String)>;

/// `None` when the rules state no claim.
pub fn analyze(doc: &Document, rules: &Compiled) -> Option<Report> {
    let forbid = &doc.config.rules.as_ref()?.doc.forbid;
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
    if ids.is_empty() {
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
    Some(Report {
        claims,
        progress: Progress {
            conforming: Count {
                done: conforming.values().filter(|held| **held).count(),
                of: conforming.len(),
            },
        },
    })
}

/// The `check` lines: a verdict per claim with up to 20 broken imports, then progress.
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
    let Count { done, of } = report.progress.conforming;
    out += &format!("target: {done} of {of} slices conform\n");
    out
}
