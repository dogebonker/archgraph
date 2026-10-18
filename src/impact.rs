//! Conservative blast radius over recorded file imports.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::collect::excludes::is_test;
use crate::model::{CycleLevel, Document, EdgeKind};

#[derive(Debug, Serialize)]
pub struct Impact {
    pub schema: &'static str,
    pub kind: &'static str,
    pub changed_files: Vec<String>,
    pub unscanned_files: Vec<String>,
    pub affected_files: Vec<AffectedFile>,
    pub affected_modules: Vec<AffectedModule>,
    pub selected_tests: Vec<String>,
    pub go_test_packages: Vec<String>,
    pub reviewers: Vec<Reviewer>,
    pub risk: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AffectedFile {
    pub path: String,
    pub module: String,
    pub depth: usize,
    pub weight: f64,
}

#[derive(Debug, Serialize)]
pub struct AffectedModule {
    pub id: String,
    pub depth: usize,
    pub weight: f64,
    pub in_cycle: bool,
    pub hotspot: bool,
}

#[derive(Debug, Serialize)]
pub struct Reviewer {
    pub identity: String,
    pub source: String,
    pub commits: usize,
    pub modules: Vec<String>,
}

/// Minimum reverse-import distance; pure re-export barrels do not consume a depth step.
pub fn analyze(
    doc: &Document,
    changed: Vec<String>,
    max_depth: usize,
    tests: &[String],
) -> Result<Impact> {
    let globs = crate::collect::globset(tests).context("invalid test glob")?;
    let changed_files: Vec<_> = changed
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let files: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let mut reverse: BTreeMap<&str, Vec<(&str, usize)>> = BTreeMap::new();
    for edge in &doc.edges {
        let barrel = files
            .get(edge.from.as_str())
            .is_some_and(|file| file.barrel);
        let cost = usize::from(
            !(barrel
                && (edge.is_reexport
                    || matches!(edge.kind, EdgeKind::Reexport | EdgeKind::TypeReexport))),
        );
        reverse
            .entry(&edge.to)
            .or_default()
            .push((&edge.from, cost));
    }
    let mut depths = BTreeMap::new();
    let mut queue = VecDeque::new();
    let mut unscanned_files = Vec::new();
    for path in &changed_files {
        if files.contains_key(path.as_str()) {
            depths.insert(path.as_str(), 0usize);
            queue.push_back(path.as_str());
        } else {
            unscanned_files.push(path.clone());
        }
    }
    while let Some(path) = queue.pop_front() {
        let depth = depths[path];
        for &(importer, cost) in reverse.get(path).into_iter().flatten() {
            let Some(next) = depth.checked_add(cost).filter(|next| *next <= max_depth) else {
                continue;
            };
            if depths.get(importer).is_none_or(|old| next < *old) {
                depths.insert(importer, next);
                if cost == 0 {
                    queue.push_front(importer);
                } else {
                    queue.push_back(importer);
                }
            }
        }
    }
    let cycle_modules: BTreeSet<_> = doc
        .cycles
        .iter()
        .filter(|cycle| matches!(cycle.level, CycleLevel::Module))
        .flat_map(|cycle| cycle.members.iter().map(String::as_str))
        .collect();
    let mut hotspot_modules = BTreeSet::new();
    if let Some(report) = &doc.hotspots {
        let mut ranked: Vec<_> = report.files.iter().collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.path.cmp(&b.path)));
        for file in ranked
            .iter()
            .take(ranked.len().div_ceil(10))
            .filter(|file| file.score > 0.0)
        {
            hotspot_modules.insert(file.module.as_str());
        }
    }
    let mut module_depths: BTreeMap<&str, usize> = BTreeMap::new();
    let mut affected_files = Vec::new();
    let mut selected_tests = Vec::new();
    for (path, depth) in depths {
        let Some(file) = files.get(path) else {
            continue;
        };
        module_depths
            .entry(&file.module)
            .and_modify(|old| *old = (*old).min(depth))
            .or_insert(depth);
        if is_test(path) || globs.is_match(path) {
            selected_tests.push(path.to_owned());
        }
        affected_files.push(AffectedFile {
            path: path.to_owned(),
            module: file.module.clone(),
            depth,
            weight: weight(depth),
        });
    }
    affected_files.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.path.cmp(&b.path)));
    let mut affected_modules: Vec<_> = module_depths
        .into_iter()
        .map(|(id, depth)| AffectedModule {
            id: id.to_owned(),
            depth,
            weight: weight(depth),
            in_cycle: cycle_modules.contains(id),
            hotspot: hotspot_modules.contains(id),
        })
        .collect();
    affected_modules.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.id.cmp(&b.id)));
    let risk = affected_modules
        .iter()
        .any(|module| module.in_cycle || module.hotspot);
    let mut limitations = vec![
        "At least these files and tests may be affected; dynamic imports, dependency injection, and dependencies absent from the scan can add more.".into(),
        format!("Reverse import traversal is capped at depth {max_depth}; weights are 1 / (1 + depth)."),
    ];
    if doc.source.language == crate::model::Language::Python {
        // shortcut: Grimp covers one package; extend collection before selecting external test trees.
        limitations.push("Python test selection covers imports in the selected Grimp package; external test trees absent from the graph have unknown coverage.".into());
    }
    if !unscanned_files.is_empty() {
        limitations.push("Changed files absent from the graph have unknown impact; deleted files require a base graph to retain their importers.".into());
    }
    if doc.files.iter().any(|file| file.barrel) {
        limitations.push("Pure re-export barrels consume no depth step. File-level edges cannot distinguish imported symbols, so barrel consumers remain conservative candidates.".into());
    }
    if let Some(report) = &doc.hotspots {
        limitations.extend(report.limitations.iter().cloned());
        if report.shallow {
            limitations.push("Hotspot risk is unavailable in this shallow clone.".into());
        }
        limitations.push("Hotspot risk means an affected module contains a positive-score file in the top 10% of the graph's hotspot report.".into());
    } else {
        limitations
            .push("Hotspot risk was not assessed because the graph has no hotspot report.".into());
    }
    Ok(Impact {
        schema: crate::model::SCHEMA,
        kind: "impact",
        changed_files,
        unscanned_files,
        affected_files,
        affected_modules,
        selected_tests,
        go_test_packages: Vec::new(),
        reviewers: Vec::new(),
        risk,
        limitations,
    })
}

/// Union both revisions so removed imports and files retain their former importers.
pub fn merge(mut head: Impact, base: Impact) -> Impact {
    head.changed_files.extend(base.changed_files);
    head.changed_files.sort();
    head.changed_files.dedup();
    let unknown: BTreeSet<_> = base.unscanned_files.into_iter().collect();
    head.unscanned_files.retain(|path| unknown.contains(path));
    let mut files: BTreeMap<_, _> = head
        .affected_files
        .into_iter()
        .map(|file| (file.path.clone(), file))
        .collect();
    for file in base.affected_files {
        files
            .entry(file.path.clone())
            .and_modify(|current| {
                current.depth = current.depth.min(file.depth);
                current.weight = weight(current.depth);
            })
            .or_insert(file);
    }
    head.affected_files = files.into_values().collect();
    head.affected_files
        .sort_by(|a, b| a.depth.cmp(&b.depth).then(a.path.cmp(&b.path)));
    let mut modules: BTreeMap<_, _> = head
        .affected_modules
        .into_iter()
        .map(|module| (module.id.clone(), module))
        .collect();
    for module in base.affected_modules {
        modules
            .entry(module.id.clone())
            .and_modify(|current| {
                current.depth = current.depth.min(module.depth);
                current.weight = weight(current.depth);
                current.in_cycle |= module.in_cycle;
                current.hotspot |= module.hotspot;
            })
            .or_insert(module);
    }
    head.affected_modules = modules.into_values().collect();
    head.affected_modules
        .sort_by(|a, b| a.depth.cmp(&b.depth).then(a.id.cmp(&b.id)));
    head.selected_tests.extend(base.selected_tests);
    head.selected_tests.sort();
    head.selected_tests.dedup();
    head.go_test_packages.extend(base.go_test_packages);
    head.go_test_packages.sort();
    head.go_test_packages.dedup();
    head.reviewers.clear();
    head.risk |= base.risk;
    head.limitations.extend(base.limitations);
    head.limitations
        .retain(|line| !line.starts_with("Changed files absent from the graph"));
    if !head.unscanned_files.is_empty() {
        head.limitations
            .push("Changed files absent from both graphs have unknown impact.".into());
    }
    head.limitations.push(
        "Impact combines the base and head graphs, retaining dependencies removed by this change."
            .into(),
    );
    head.limitations.sort();
    head.limitations.dedup();
    head
}

fn weight(depth: usize) -> f64 {
    1.0 / (1.0 + depth as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_elixir_test_conventions() {
        assert!(is_test("lib/service_test.exs"));
        assert!(is_test("test_helper.exs"));
        assert!(!is_test("lib/service.ex"));
    }
    use crate::model::{Config, Edge, Source};

    fn doc(pairs: &[(&str, &str, EdgeKind)]) -> Document {
        let files = pairs
            .iter()
            .flat_map(|(from, to, _)| [from.to_string(), to.to_string()])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let source: Source = serde_json::from_value(serde_json::json!({
            "language":"typescript", "collector":{"name":"fixture", "version":null,
            "cruised":0,"unresolved":null},"git":null,"generated_at":"fixture"
        }))
        .unwrap();
        let config: Config = serde_json::from_value(serde_json::json!({
            "depth":1,"source_root":".","excludes":[],"include_tests":true,"rules":null
        }))
        .unwrap();
        crate::analyze(
            source,
            config,
            files,
            pairs
                .iter()
                .map(|(from, to, kind)| Edge {
                    from: from.to_string(),
                    to: to.to_string(),
                    kind: *kind,
                    verdict: None,
                    rule: None,
                    reach: None,
                    ..crate::model::Edge::default()
                })
                .collect(),
            vec![],
            None,
        )
        .unwrap()
    }

    #[test]
    fn reverse_bfs_caps_depth_and_terminates_cycles() {
        let doc = doc(&[
            ("a/a.ts", "b/b.ts", EdgeKind::Static),
            ("b/b.ts", "a/a.ts", EdgeKind::Static),
            ("tests/a.spec.ts", "a/a.ts", EdgeKind::Static),
            ("far/f.ts", "tests/a.spec.ts", EdgeKind::Static),
        ]);
        let impact = analyze(&doc, vec!["b/b.ts".into()], 2, &[]).unwrap();
        assert_eq!(
            impact
                .affected_files
                .iter()
                .map(|f| (f.path.as_str(), f.depth))
                .collect::<Vec<_>>(),
            [("b/b.ts", 0), ("a/a.ts", 1), ("tests/a.spec.ts", 2)]
        );
        assert_eq!(impact.selected_tests, ["tests/a.spec.ts"]);
        assert!(impact.risk);
        assert_eq!(impact.affected_modules[1].weight, 0.5);
    }

    #[test]
    fn nested_reexports_do_not_spend_depth_and_barrel_edits_still_propagate() {
        let doc = doc(&[
            ("api/index.ts", "inner/index.ts", EdgeKind::Reexport),
            ("inner/index.ts", "src/value.ts", EdgeKind::TypeReexport),
            ("app/app.ts", "api/index.ts", EdgeKind::Static),
            ("checks/check.ts", "app/app.ts", EdgeKind::Static),
        ]);
        let impact = analyze(&doc, vec!["src/value.ts".into()], 1, &[]).unwrap();
        assert!(
            impact
                .affected_files
                .iter()
                .any(|f| f.path == "app/app.ts" && f.depth == 1)
        );
        assert!(
            !impact
                .affected_files
                .iter()
                .any(|f| f.path == "checks/check.ts")
        );
        let impact = analyze(&doc, vec!["api/index.ts".into()], 1, &[]).unwrap();
        assert!(impact.affected_files.iter().any(|f| f.path == "app/app.ts"));
    }

    #[test]
    fn custom_tests_multiple_roots_and_unscanned_changes_are_explicit() {
        let doc = doc(&[
            ("checks/check.ts", "src/a.ts", EdgeKind::Static),
            ("checks/check.ts", "src/b.ts", EdgeKind::Static),
        ]);
        let impact = analyze(
            &doc,
            vec![
                "src/a.ts".into(),
                "checks/check.ts".into(),
                "gone.ts".into(),
                "src/a.ts".into(),
            ],
            0,
            &["checks/**".into()],
        )
        .unwrap();
        assert_eq!(impact.selected_tests, ["checks/check.ts"]);
        assert_eq!(impact.unscanned_files, ["gone.ts"]);
        assert_eq!(impact.changed_files.len(), 3);
        assert_eq!(impact.affected_files[0].depth, 0);
        assert!(analyze(&doc, vec![], 3, &["[".into()]).is_err());
        // Custom globs keep `*` within one segment; built-in conventions match the scan's.
        let doc = self::doc(&[
            ("checks/deep/check.ts", "src/a.ts", EdgeKind::Static),
            ("pkg/conftest.py", "src/a.ts", EdgeKind::Static),
            ("src/tests", "src/a.ts", EdgeKind::Static),
        ]);
        let impact = analyze(&doc, vec!["src/a.ts".into()], 1, &["checks/*.ts".into()]).unwrap();
        assert_eq!(impact.selected_tests, ["pkg/conftest.py"]);
    }
    #[test]
    fn merge_preserves_removed_imports_and_uses_minimum_depth() {
        let base = doc(&[
            ("tests/old.test.ts", "src/deleted.ts", EdgeKind::Static),
            ("src/retained.ts", "src/deleted.ts", EdgeKind::Static),
        ]);
        let head = doc(&[("tests/new.test.ts", "src/retained.ts", EdgeKind::Static)]);
        let changes = vec![
            "src/deleted.ts".into(),
            "src/retained.ts".into(),
            "unknown.ts".into(),
        ];
        let report = merge(
            analyze(&head, changes.clone(), 3, &[]).unwrap(),
            analyze(&base, changes, 3, &[]).unwrap(),
        );
        assert_eq!(report.unscanned_files, ["unknown.ts"]);
        assert_eq!(
            report.selected_tests,
            ["tests/new.test.ts", "tests/old.test.ts"]
        );
        let retained = report
            .affected_files
            .iter()
            .find(|file| file.path == "src/retained.ts")
            .unwrap();
        assert_eq!(retained.depth, 0);
        assert_eq!(retained.weight, 1.0);
        assert!(
            report
                .affected_files
                .iter()
                .any(|file| file.path == "src/deleted.ts")
        );
    }
    #[test]
    fn hotspot_risk_uses_positive_top_decile_scores() {
        let mut doc = doc(&[("app/a.ts", "src/value.ts", EdgeKind::Static)]);
        doc.hotspots = Some(
            serde_json::from_value(serde_json::json!({
                "since":"12 months ago", "since_timestamp":0, "as_of":"fixture",
                "max_files":30, "authors_enabled":false, "shallow":false,
                "commits":1, "skipped":0, "limitations":[],
                "validation":{"files":2,"top_files":1,"bugfix_commits":0,"top_bugfix_commits":0,
                "share":null,"lift":null},
                "files":[{"path":"src/value.ts","module":"src","commits":1,
                "lines_added":1,"lines_deleted":0,"authors":1,"bugfix_commits":0,
                "churn":1.0,"coupling":1.0,"cycle":0.0,"author_factor":1.0,
                "score":1.0,"cycle_size":0,"cut_candidate":false,"finding_indices":[]}]
            }))
            .unwrap(),
        );
        let report = analyze(&doc, vec!["src/value.ts".into()], 0, &[]).unwrap();
        assert!(report.risk);
        assert!(report.affected_modules[0].hotspot);
        doc.hotspots.as_mut().unwrap().files[0].score = 0.0;
        assert!(
            !analyze(&doc, vec!["src/value.ts".into()], 0, &[])
                .unwrap()
                .risk
        );
        let hotspots = doc.hotspots.as_mut().unwrap();
        hotspots.shallow = true;
        hotspots.limitations.push("History is incomplete.".into());
        let report = analyze(&doc, vec!["src/value.ts".into()], 0, &[]).unwrap();
        assert!(
            report
                .limitations
                .iter()
                .any(|note| note.contains("unavailable in this shallow"))
        );
        assert!(
            report
                .limitations
                .iter()
                .any(|note| note == "History is incomplete.")
        );
    }
}
