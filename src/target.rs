//! The target check: one verdict per claim (forbid rules sharing an `id`, one with a `reason`),
//! per slice (a value of the rules' `<name>` capture), the slices that conform, and what the
//! `moves` still pending cost per slice.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::model::{CochangePair, Document, Edge, Language, Link, Move};
use crate::modules::dir_of;
use crate::rules::{Compiled, SliceEvidence};

#[derive(Debug, Clone, Default, Serialize)]
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
    /// Pairs from `--cochange` with one file on a claim rule's `from` side and the other on its
    /// `to` side in this slice, `from` side first, and no import either way.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cochanged: Vec<CochangePair>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    /// Captured slices that hold in every claim bound to them.
    pub conforming: Count,
    /// Captured slices with no file left to move, when the rules have `moves`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved: Option<Count>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Count {
    pub done: usize,
    pub of: usize,
}

/// What changed in the target between two scans, `[old, new]` per count.
#[derive(Debug, Clone, Serialize)]
pub struct Delta {
    /// Captured slices that hold in every claim bound to them.
    pub conforming: [Count; 2],
    /// Slices with no file left to move; only when both scans have `moves`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved: Option<[Count; 2]>,
    pub newly_broken: Vec<SliceChange>,
    /// Slices broken in old that are not broken in new, also when no longer bound or gone.
    pub newly_fixed: Vec<SliceChange>,
    /// Claims vacuous in new that old has and that are not vacuous there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub newly_vacuous: Vec<VacuousClaim>,
    /// Co-changed pairs a slice lists in new and not in old; only when both scans read co-change.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub newly_cochanged: Vec<CochangedPair>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangedPair {
    pub claim: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub slice: String,
    pub files: [String; 2],
    pub commits: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct VacuousClaim {
    pub claim: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SliceChange {
    pub claim: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub slice: String,
    pub imports: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub from: String,
    pub to: String,
    /// Absent when the collector records no line for the import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
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
    let pairs = doc.cochange.as_ref().map_or(&[][..], |c| &c.pairs[..]);
    let links: HashSet<(&str, &str)> = doc
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    // Per capture value, whether it holds in every claim bound to it.
    let mut conforming: BTreeMap<String, bool> = BTreeMap::new();
    let claims = ids
        .into_iter()
        .map(|id| {
            let mut notes = Vec::new();
            let mut slices: BTreeMap<Option<String>, (Files, Imports, Vec<&SliceEvidence>)> =
                BTreeMap::new();
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
                    if s.bound() {
                        bound.insert(value.clone());
                    }
                    let (files, broken, sides) = slices.entry(value.clone()).or_default();
                    files.extend(s.from.iter().chain(&s.to).cloned());
                    broken.extend(s.hits.iter().map(|h| (h.from.clone(), h.to.clone())));
                    sides.push(s);
                }
            }
            slices.retain(|value, _| bound.contains(value));
            for (value, (_, broken, _)) in &slices {
                if let Some(value) = value {
                    *conforming.entry(value.clone()).or_insert(true) &= broken.is_empty();
                }
            }
            let held = slices
                .values()
                .filter(|(_, broken, _)| broken.is_empty())
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
                    .map(|(value, (files, broken, sides))| Slice {
                        slice: value.unwrap_or_else(|| "*".into()),
                        files: files.into_iter().collect(),
                        broken: broken
                            .into_iter()
                            .map(|(from, to)| Link { from, to })
                            .collect(),
                        cochanged: cochanged(pairs, &links, &sides),
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
    // Names come only from go-types pairs; a go-types scan saved before pairs carried them has
    // only `bound_files`, and an empty list there would read as nothing to export.
    let legacy = !doc.source.collector.bound_files.is_empty();
    let go_types = doc.source.collector.name == "go-types" && !legacy;
    if doc.source.language == Language::Go && !go_types && !all.is_empty() {
        notes.push(if legacy && doc.source.collector.name == "go-types" {
            "names to export unknown: the scan predates bound pairs, rescan to list them".to_owned()
        } else {
            format!("names to export unknown for {}", doc.source.collector.name)
        });
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

/// The `pairs` with one file on a rule's `from` side and the other on its `to` side, `from` side
/// first, that no chain of imports among the slice's files joins either way: a handler and a
/// repository changing together through the service between them is the layering at work.
// ponytail: every pair against every slice's sides, O(pairs × slices × side files); index files
// by slice if long histories over many slices get slow.
fn cochanged(
    pairs: &[CochangePair],
    links: &HashSet<(&str, &str)>,
    sides: &[&SliceEvidence],
) -> Vec<CochangePair> {
    let across =
        |a: &String, b: &String| sides.iter().any(|s| s.from.contains(a) && s.to.contains(b));
    let files: HashSet<&str> = sides
        .iter()
        .flat_map(|s| s.from.iter().chain(&s.to).map(String::as_str))
        .collect();
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for &(a, b) in links {
        if files.contains(a) && files.contains(b) {
            next.entry(a).or_default().push(b);
        }
    }
    let reaches = |from: &str, to: &str| {
        let (mut seen, mut queue) = (HashSet::from([from]), vec![from]);
        while let Some(at) = queue.pop() {
            for &n in next.get(at).into_iter().flatten() {
                if n == to {
                    return true;
                }
                if seen.insert(n) {
                    queue.push(n);
                }
            }
        }
        false
    };
    pairs
        .iter()
        .filter(|p| !crate::cochange::linked(links, &p.files))
        .filter(|p| !reaches(&p.files[0], &p.files[1]) && !reaches(&p.files[1], &p.files[0]))
        .filter_map(|p| {
            let [a, b] = &p.files;
            let files = if across(a, b) {
                [a.clone(), b.clone()]
            } else if across(b, a) {
                [b.clone(), a.clone()]
            } else {
                return None;
            };
            Some(CochangePair {
                files,
                commits: p.commits,
            })
        })
        .collect()
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

/// The target's counts in both scans, the slices newly broken and newly fixed, the claims newly
/// vacuous and the co-changed pairs new in `new`; `None` when neither scan has a target.
pub fn delta(old: &Document, new: &Document) -> Option<Delta> {
    if old.target.is_none() && new.target.is_none() {
        return None;
    }
    let none = Report::default();
    let was = old.target.as_ref().unwrap_or(&none);
    let is = new.target.as_ref().unwrap_or(&none);
    Some(Delta {
        conforming: [was.progress.conforming, is.progress.conforming],
        moved: was
            .progress
            .moved
            .zip(is.progress.moved)
            .map(|(a, b)| [a, b]),
        newly_broken: changed(new, is, was),
        newly_fixed: changed(old, was, is),
        newly_vacuous: is
            .claims
            .iter()
            .filter(|c| {
                c.verdict == ClaimVerdict::Vacuous
                    && was
                        .claims
                        .iter()
                        .any(|o| o.id == c.id && o.verdict != ClaimVerdict::Vacuous)
            })
            .map(|c| VacuousClaim {
                claim: c.id.clone(),
                reason: c.reason.clone(),
                source: c.source.clone(),
                notes: c.notes.clone(),
            })
            .collect(),
        newly_cochanged: if old.cochange.is_some() && new.cochange.is_some() {
            newly_cochanged(was, is)
        } else {
            Vec::new()
        },
    })
}

/// Pairs a slice of `is` lists that the same claim's slice in `was` does not.
fn newly_cochanged(was: &Report, is: &Report) -> Vec<CochangedPair> {
    let listed = |claim: &str, slice: &str, files: &[String; 2]| {
        was.claims
            .iter()
            .filter(|c| c.id == claim)
            .flat_map(|c| &c.slices)
            .any(|s| s.slice == slice && s.cochanged.iter().any(|p| &p.files == files))
    };
    is.claims
        .iter()
        .flat_map(|c| c.slices.iter().map(move |s| (c, s)))
        .flat_map(|(c, s)| s.cochanged.iter().map(move |p| (c, s, p)))
        .filter(|(c, s, p)| !listed(&c.id, &s.slice, &p.files))
        .map(|(c, s, p)| CochangedPair {
            claim: c.id.clone(),
            reason: c.reason.clone(),
            source: c.source.clone(),
            slice: s.slice.clone(),
            files: p.files.clone(),
            commits: p.commits,
        })
        .collect()
}

/// Slices broken in `report`, the target of `doc`, that are not broken in `other`, with each
/// import's line in `doc`.
fn changed(doc: &Document, report: &Report, other: &Report) -> Vec<SliceChange> {
    let broken_in_other = |claim: &str, slice: &str| {
        other
            .claims
            .iter()
            .filter(|c| c.id == claim)
            .flat_map(|c| &c.slices)
            .any(|s| s.slice == slice && !s.broken.is_empty())
    };
    report
        .claims
        .iter()
        .flat_map(|c| c.slices.iter().map(move |s| (c, s)))
        .filter(|(c, s)| !s.broken.is_empty() && !broken_in_other(&c.id, &s.slice))
        .map(|(c, s)| SliceChange {
            claim: c.id.clone(),
            reason: c.reason.clone(),
            source: c.source.clone(),
            slice: s.slice.clone(),
            imports: s
                .broken
                .iter()
                .map(|l| Evidence {
                    from: l.from.clone(),
                    to: l.to.clone(),
                    line: line(doc, &l.from, &l.to),
                })
                .collect(),
        })
        .collect()
}

/// The first line an edge `from -> to` records, else the smallest of the import records of
/// `from` resolving to `to` (Go edges carry none).
// ponytail: scans every edge and record per broken import, O(links × edges); index the pairs
// once if diffs with many broken imports get slow.
pub(crate) fn line(doc: &Document, from: &str, to: &str) -> Option<usize> {
    doc.edges
        .iter()
        .chain(&doc.external_edges)
        .find_map(|e| (e.from == from && e.to == to).then_some(e.line).flatten())
        .or_else(|| {
            doc.imports
                .iter()
                .flatten()
                .filter(|i| i.from == from && i.targets.iter().any(|t| t == to))
                .filter_map(|i| i.line)
                .min()
        })
}

/// The `check` lines: a verdict per claim with up to 20 broken imports and 20 co-changed pairs, the
/// pending moves, notes, then progress.
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
        let pairs: Vec<&CochangePair> = claim.slices.iter().flat_map(|s| &s.cochanged).collect();
        for p in pairs.iter().take(20) {
            let [from, to] = &p.files;
            out += &format!(
                "  co-change: {from} <-> {to}, {} commits, no import\n",
                p.commits
            );
        }
        if pairs.len() > 20 {
            out += &format!("  ... and {} more\n", pairs.len() - 20);
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
