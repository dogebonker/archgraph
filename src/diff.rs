//! Trend between two scans: module metrics, cycles and forbidden imports.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::model::{
    Cycle, CycleLevel, Diff, Document, Edge, Link, Module, ModuleDelta, SCHEMA, Snapshot, Verdict,
};
use crate::target::{CochangedPair, Delta, SliceChange};

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
        stale_known: new.stale_known.clone(),
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
        [] => "**boundrail diff: pass**".to_owned(),
        _ => format!("**boundrail diff: fail** ({})", failed.join(", ")),
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
        out += &format!("\n{}", progress(t));
        if !t.newly_broken.is_empty() {
            out += &format!(", {}", count(t.newly_broken.len(), "newly broken slice"));
        }
        if !t.newly_vacuous.is_empty() {
            out += &format!(", {}", count(t.newly_vacuous.len(), "newly vacuous claim"));
        }
        if !t.newly_cochanged.is_empty() {
            out += &format!(
                ", {}",
                count(t.newly_cochanged.len(), "newly co-changed pair")
            );
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
    if !d.stale_known.is_empty() {
        out += &format!(
            "\n### Known entries no longer forbidden ({})\n\n",
            d.stale_known.len()
        );
        for e in &d.stale_known {
            out += &format!("- {} → {} ({})\n", esc(&e.from), esc(&e.to), esc(&e.rule));
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
    if let Some(t) = d.target.as_ref().filter(|t| !t.newly_vacuous.is_empty()) {
        out += &format!("\n### Newly vacuous claims ({})\n\n", t.newly_vacuous.len());
        for c in &t.newly_vacuous {
            out += &format!("- {}", esc(&c.claim));
            if !c.notes.is_empty() {
                out += &format!(": {}", c.notes.join("; "));
            }
            out += "\n";
        }
    }
    if let Some(t) = d.target.as_ref().filter(|t| !t.newly_cochanged.is_empty()) {
        out += &cochanged_title(&t.newly_cochanged);
        for p in &t.newly_cochanged {
            out += &cochanged_row(p);
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

/// `Target: 15 of 18 slices conform (was 16 of 18), 0 of 23 slices moved`.
fn progress(t: &Delta) -> String {
    let [old, new] = t.conforming;
    let mut out = format!("Target: {} of {} slices conform", new.done, new.of);
    if old != new {
        out += &format!(" (was {} of {})", old.done, old.of);
    }
    if let Some([old, m]) = t.moved {
        out += &format!(", {} of {} slices moved", m.done, m.of);
        if old != m {
            out += &format!(" (was {} of {})", old.done, old.of);
        }
    }
    out
}

fn cochanged_title(pairs: &[CochangedPair]) -> String {
    format!(
        "\n### Changed together across a claim, no import ({})\n\n",
        pairs.len()
    )
}

/// `- {claim}, slice {slice}: {from} ↔ {to} ({n} commits)`.
fn cochanged_row(p: &CochangedPair) -> String {
    format!(
        "- {}, slice {}: {} ↔ {} ({} commits)\n",
        esc(&p.claim),
        esc(&p.slice),
        esc(&p.files[0]),
        esc(&p.files[1]),
        p.commits
    )
}

/// Bytes of `review` markdown before rows give way to a count; totals are always exact.
const REVIEW_BYTES: usize = 1300;

/// What a pull request changed in the target, for an AI reviewer to quote: `markdown` within
/// `REVIEW_BYTES`, and every row as JSON. `other_forbidden` is the new forbidden imports not
/// already under a newly broken slice, with their rule's reasons and sources.
pub fn review(old: &Document, new: &Document) -> Value {
    let d = diff(old, new);
    let (broken, fixed, vacuous, cochanged) =
        d.target
            .as_ref()
            .map_or((&[][..], &[][..], &[][..], &[][..]), |t| {
                (
                    &t.newly_broken[..],
                    &t.newly_fixed[..],
                    &t.newly_vacuous[..],
                    &t.newly_cochanged[..],
                )
            });
    let in_broken: BTreeSet<(&str, &str)> = broken
        .iter()
        .flat_map(|c| &c.imports)
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    let rules = new.config.rules.as_ref();
    let other: Vec<(&Edge, &str, Vec<&str>, Vec<&str>)> = d
        .new_forbidden
        .iter()
        .filter(|e| !in_broken.contains(&(e.from.as_str(), e.to.as_str())))
        .map(|e| {
            let rule = e.rule.as_deref().unwrap_or_default();
            let (reasons, sources) = rules.map_or_else(Default::default, |r| {
                (r.doc.reasons(rule), r.doc.sources(rule))
            });
            (e, rule, reasons, sources)
        })
        .collect();
    let mut md = format!(
        "**boundrail review**: {}, {}, {}.\n",
        count(broken.len(), "newly broken slice"),
        count(other.len(), "other new forbidden import"),
        count(fixed.len(), "newly fixed slice"),
    );
    if rules.is_none() {
        md += "No rules: nothing was checked; add layout.yaml or pass --rules.\n";
    } else if let Some(t) = &d.target {
        md += &format!("{}.\n", progress(t));
    }
    fn at(path: &str, line: Option<usize>) -> String {
        match line {
            Some(n) => format!("{}:{n}", esc(path)),
            None => esc(path),
        }
    }
    // Each line with whether it is a row, which the footer counts.
    fn slices(lines: &mut Vec<(String, bool)>, title: &str, changes: &[SliceChange]) {
        if changes.is_empty() {
            return;
        }
        lines.push((format!("\n### {title} ({})\n\n", changes.len()), false));
        let mut claim = None;
        for c in changes {
            if claim != Some(&c.claim) {
                claim = Some(&c.claim);
                let source = c
                    .source
                    .as_ref()
                    .map_or(String::new(), |s| format!(" ({})", esc(s)));
                lines.push((
                    format!("- **{}**: {}{source}\n", esc(&c.claim), esc(&c.reason)),
                    false,
                ));
            }
            for e in &c.imports {
                let row = format!(
                    "  - slice {}: {} → {}\n",
                    esc(&c.slice),
                    at(&e.from, e.line),
                    esc(&e.to)
                );
                lines.push((row, true));
            }
        }
    }
    let known_file = rules
        .and_then(|r| r.known.as_ref())
        .map_or("the known file", |k| k.path.as_str());
    let mut lines = Vec::new();
    slices(&mut lines, "Newly broken", broken);
    if !vacuous.is_empty() {
        lines.push((
            format!("\n### Newly vacuous claims ({})\n\n", vacuous.len()),
            false,
        ));
        for c in vacuous {
            let mut row = format!("- **{}**: {}", esc(&c.claim), esc(&c.reason));
            if let Some(source) = &c.source {
                row += &format!(" ({})", esc(source));
            }
            if !c.notes.is_empty() {
                row += &format!(", {}", c.notes.join("; "));
            }
            lines.push((row + "\n", true));
        }
    }
    if !other.is_empty() {
        lines.push((
            format!("\n### Other new forbidden imports ({})\n\n", other.len()),
            false,
        ));
        for (e, rule, reasons, sources) in &other {
            let mut row = format!("- {} → {}, {}", at(&e.from, e.line), esc(&e.to), esc(rule));
            if !reasons.is_empty() {
                let reasons: Vec<String> = reasons.iter().map(|r| esc(r)).collect();
                row += &format!(": {}", reasons.join("; "));
            }
            if !sources.is_empty() {
                let sources: Vec<String> = sources.iter().map(|s| esc(s)).collect();
                row += &format!(" ({})", sources.join(", "));
            }
            if e.known {
                row += &format!(", listed in {known_file}");
            }
            lines.push((row + "\n", true));
        }
    }
    slices(&mut lines, "Newly fixed", fixed);
    if !cochanged.is_empty() {
        lines.push((cochanged_title(cochanged), false));
        lines.extend(cochanged.iter().map(|p| (cochanged_row(p), true)));
    }
    let unknown = broken
        .iter()
        .chain(fixed)
        .flat_map(|c| &c.imports)
        .filter(|e| e.line.is_none())
        .count()
        + other.iter().filter(|(e, ..)| e.line.is_none()).count();
    let footer = |more: usize| {
        let mut out = String::new();
        if more > 0 {
            out += &format!(
                "{} in `boundrail review --format json`.\n",
                count(more, "more row")
            );
        }
        if !new.stale_known.is_empty() {
            let line = match new.stale_known.len() {
                1 => "1 known entry no longer occurs".to_owned(),
                n => format!("{n} known entries no longer occur"),
            };
            out += &format!("{line}: run boundrail check --update-known shrink.\n");
        }
        if unknown > 0 {
            out += &format!(
                "Lines unknown for {}: the {} collector records none.\n",
                count(unknown, "import"),
                new.source.collector.name
            );
        }
        if out.is_empty() {
            out
        } else {
            format!("\n{out}")
        }
    };
    let rows = lines.iter().filter(|(_, row)| *row).count();
    let all: usize = lines.iter().map(|(text, _)| text.len()).sum();
    let reserve = if md.len() + all + footer(0).len() <= REVIEW_BYTES {
        0
    } else {
        footer(rows).len()
    };
    let (mut more, mut full) = (0, false);
    for (text, row) in lines {
        full = full || md.len() + text.len() + reserve > REVIEW_BYTES;
        if full {
            more += usize::from(row);
        } else {
            md += &text;
        }
    }
    md += &footer(more);
    json!({
        "markdown": md,
        "progress": d.target.as_ref().map(|t| json!({"conforming": t.conforming, "moved": t.moved})),
        "newly_broken": broken,
        "newly_fixed": fixed,
        "newly_vacuous": vacuous,
        "newly_cochanged": cochanged,
        "other_forbidden": other.iter().map(|(e, rule, reasons, sources)| {
            let mut row = json!({"from": e.from, "to": e.to, "rule": rule, "reasons": reasons, "sources": sources});
            if let Some(line) = e.line {
                row["line"] = json!(line);
            }
            if e.known {
                row["known"] = json!(true);
            }
            row
        }).collect::<Vec<_>>(),
    })
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
