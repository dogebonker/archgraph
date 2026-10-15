//! Trend between two scans: module metrics, cycles and forbidden imports.

use std::collections::{BTreeMap, HashSet};

use crate::model::{Cycle, Diff, Document, Edge, Module, ModuleDelta, SCHEMA, Snapshot, Verdict};

pub fn diff(old: &Document, new: &Document) -> Diff {
    let snapshot = |d: &Document| Snapshot {
        commit: d.source.git.as_ref().map(|g| g.commit.clone()),
        generated_at: d.source.generated_at.clone(),
    };
    fn by_id(d: &Document) -> BTreeMap<&str, &Module> {
        d.modules.iter().map(|m| (m.id.as_str(), m)).collect()
    }
    let (was, is) = (by_id(old), by_id(new));
    let missing = |a: &BTreeMap<&str, &Module>, b: &BTreeMap<&str, &Module>| {
        a.keys()
            .filter(|id| !b.contains_key(*id))
            .map(|id| id.to_string())
            .collect()
    };
    let modules = was
        .iter()
        .filter_map(|(id, o)| {
            let n = is.get(id)?;
            let delta = ModuleDelta {
                cohesion: [o.cohesion, n.cohesion],
                ca: [o.ca, n.ca],
                ce: [o.ce, n.ce],
                instability: [o.instability, n.instability],
            };
            Some((id.to_string(), delta))
        })
        .collect();
    let cycles_only_in = |a: &Document, b: &Document| -> Vec<Cycle> {
        a.cycles
            .iter()
            .filter(|c| {
                !b.cycles
                    .iter()
                    .any(|d| d.level == c.level && d.members == c.members)
            })
            .cloned()
            .collect()
    };
    let forbidden_only_in = |a: &Document, b: &Document| -> Vec<Edge> {
        let forbidden = |e: &&Edge| e.verdict == Some(Verdict::Forbidden);
        let theirs: HashSet<(&str, &str)> = b
            .edges
            .iter()
            .filter(forbidden)
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        a.edges
            .iter()
            .filter(forbidden)
            .filter(|e| !theirs.contains(&(e.from.as_str(), e.to.as_str())))
            .cloned()
            .collect()
    };
    Diff {
        schema: SCHEMA,
        kind: "diff",
        old: snapshot(old),
        new: snapshot(new),
        modules,
        added_modules: missing(&is, &was),
        removed_modules: missing(&was, &is),
        new_cycles: cycles_only_in(new, old),
        resolved_cycles: cycles_only_in(old, new),
        new_forbidden: forbidden_only_in(new, old),
        resolved_forbidden: forbidden_only_in(old, new),
    }
}
