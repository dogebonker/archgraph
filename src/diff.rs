//! Trend between two scans: module metrics, cycles and forbidden imports.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Cycle, CycleLevel, Diff, Document, Edge, Link, Module, ModuleDelta, SCHEMA, Snapshot, Verdict,
};

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
                weighted_ca: [o.weighted_ca, n.weighted_ca],
                weighted_ce: [o.weighted_ce, n.weighted_ce],
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
    // Imports with `verdict` in `a` whose pair has another verdict, or is missing, in `b`.
    let only_in = |a: &Document, b: &Document, verdict: Verdict| -> Vec<Edge> {
        let with = |e: &&Edge| e.verdict == Some(verdict);
        let theirs: BTreeSet<(&str, &str, crate::model::EdgeKind)> = b
            .edges
            .iter()
            .chain(&b.external_edges)
            .filter(with)
            .map(|e| (e.from.as_str(), e.to.as_str(), e.kind))
            .collect();
        a.edges
            .iter()
            .chain(&a.external_edges)
            .filter(with)
            .filter(|e| !theirs.contains(&(e.from.as_str(), e.to.as_str(), e.kind)))
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
        new_cyclic_imports: crate::graph::new_cyclic_import_pairs(old, new)
            .into_iter()
            .map(|(from, to)| Link { from, to })
            .collect(),
        resolved_cycles: cycles_only_in(old, new),
        new_forbidden: only_in(new, old, Verdict::Forbidden),
        resolved_forbidden: only_in(old, new, Verdict::Forbidden),
        new_unplanned: only_in(new, old, Verdict::Unplanned),
        target: crate::target::delta(old, new),
    }
}

/// Smallest cohesion or instability change the markdown summary lists.
const MOVED: f64 = 0.05;
const ROWS: usize = 20;

/// A compact summary for a pull request or CI job: pass or fail (`failed` names the `--fail-on`
/// conditions present), new forbidden and unplanned imports, new and resolved cycles, modules
/// whose cohesion or instability moved by `MOVED` or more, added and removed module counts.
pub fn markdown(d: &Diff, failed: &[&str]) -> String {
    let mut out = match failed {
        [] => "**archgraph diff: pass**".to_owned(),
        _ => format!("**archgraph diff: fail** ({})", failed.join(", ")),
    };
    out += &format!(
        ": {}, {}, {}, {}, {}.\n",
        count(d.new_forbidden.len(), "new forbidden import"),
        count(d.new_unplanned.len(), "new unplanned import"),
        count(d.new_cyclic_imports.len(), "new cyclic import"),
        count(d.new_cycles.len(), "new dependency component"),
        count(d.resolved_cycles.len(), "resolved dependency component"),
    );
    if let Some(t) = &d.target {
        let [old, new] = t.conforming;
        out += &format!("\nTarget: {} of {} slices conform", new.done, new.of);
        if old != new {
            out += &format!(" (was {} of {})", old.done, old.of);
        }
        if let Some([old, m]) = t.moved {
            out += &format!(", {} of {} slices moved", m.done, m.of);
            if old != m {
                out += &format!(" (was {} of {})", old.done, old.of);
            }
        }
        if !t.newly_broken.is_empty() {
            out += &format!(", {}", count(t.newly_broken.len(), "newly broken slice"));
        }
        out += ".\n";
    }
    for (title, edges) in [
        ("New forbidden imports", &d.new_forbidden),
        ("New unplanned imports", &d.new_unplanned),
    ] {
        if edges.is_empty() {
            continue;
        }
        out += &format!("\n### {title} ({})\n\n", edges.len());
        for e in edges {
            let rule = e.rule.as_deref().unwrap_or_default();
            out += &format!("- {} → {} ({})\n", esc(&e.from), esc(&e.to), esc(rule));
        }
    }
    if !d.new_cyclic_imports.is_empty() {
        out += &format!(
            "\n### New cyclic runtime imports ({})\n\n",
            d.new_cyclic_imports.len()
        );
        for edge in &d.new_cyclic_imports {
            out += &format!("- {} → {}\n", esc(&edge.from), esc(&edge.to));
        }
    }
    for (title, cycles) in [
        ("New dependency components", &d.new_cycles),
        ("Resolved dependency components", &d.resolved_cycles),
    ] {
        if cycles.is_empty() {
            continue;
        }
        out += &format!("\n### {title} ({})\n\n", cycles.len());
        for c in cycles {
            let level = match c.level {
                CycleLevel::Module => "module",
                CycleLevel::File => "file",
            };
            let members: Vec<String> = c.members.iter().map(|m| esc(m)).collect();
            out += &format!("- {level}: {}\n", members.join(", "));
        }
    }
    if let Some(t) = d.target.as_ref().filter(|t| !t.newly_broken.is_empty()) {
        out += &format!("\n### Newly broken slices ({})\n\n", t.newly_broken.len());
        for b in &t.newly_broken {
            for link in &b.imports {
                out += &format!(
                    "- {}, slice {}: {} → {}\n",
                    esc(&b.claim),
                    esc(&b.slice),
                    esc(&link.from),
                    esc(&link.to)
                );
            }
        }
    }
    let change = |pair: [Option<f64>; 2]| match pair {
        [Some(a), Some(b)] => (b - a).abs(),
        _ => 0.0,
    };
    let mut moved: Vec<(&String, &ModuleDelta, f64)> = d
        .modules
        .iter()
        .map(|(id, m)| (id, m, change(m.cohesion).max(change(m.instability))))
        // Metrics are rounded to 4 decimals, so 0.5 -> 0.55 is a move of 0.05.
        .filter(|(_, _, by)| *by >= MOVED - 1e-9)
        .collect();
    moved.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(b.0)));
    if !moved.is_empty() {
        out += &format!(
            "\n### Module metrics ({} moved by {MOVED} or more)\n\n",
            moved.len()
        );
        out += "| module | cohesion | instability |\n| --- | --- | --- |\n";
        let ratio = |r: Option<f64>| r.map_or("n/a".to_owned(), |r| format!("{r:.2}"));
        let pair = |p: [Option<f64>; 2]| format!("{} → {}", ratio(p[0]), ratio(p[1]));
        for (id, m, _) in moved.iter().take(ROWS) {
            out += &format!(
                "| {} | {} | {} |\n",
                esc(id),
                pair(m.cohesion),
                pair(m.instability)
            );
        }
        if moved.len() > ROWS {
            out += &format!("\nand {} more\n", moved.len() - ROWS);
        }
    }
    out += &format!(
        "\nModules: {} added, {} removed.\n",
        d.added_modules.len(),
        d.removed_modules.len()
    );
    out
}

/// `n` and `noun`, plural unless `n` is 1.
pub fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// Paths come from the scanned repository: every character that could start markdown is
/// backslash-escaped, and line breaks become spaces so no path can open a new block.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '|' | '`' | '<' | '>' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}
