//! Pure pre-edit/post-edit enforcement; existing violations remain advisory.
use std::collections::BTreeMap;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::{CycleLevel, Document, Evidence};

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub scope: Vec<String>,
    /// Includes additions and deletions; callers supply paths from the actual edit.
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub verdict: String,
    pub violations: Vec<Violation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub finding_id: String,
    pub kind: String,
    pub rule: String,
    pub file: String,
    pub recipe: Value,
}

fn validate_relative(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && !value.starts_with('/')
            && !value.contains(['\\', ':'])
            && !value.chars().any(char::is_control)
            && value
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")),
        "expected a repository-relative path or glob: {value:?}"
    );
    Ok(())
}

fn violation(
    kind: &str,
    rule: &str,
    file: &str,
    identity: Value,
    recipe: Option<(String, Value)>,
    instruction: &str,
) -> Violation {
    let identity = json!([kind, rule, file, identity]);
    let finding_id = recipe
        .as_ref()
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| {
            format!(
                "ag-{}",
                Sha256::digest(identity.to_string().as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        });
    Violation {
        finding_id: finding_id.clone(),
        kind: kind.into(),
        rule: rule.into(),
        file: file.into(),
        recipe: recipe.map(|(_, value)| value).unwrap_or_else(|| json!({
            "mechanical": false,
            "steps": [{"kind":"instruction", "file":file, "mechanical":false, "instruction":instruction}],
            "verify": {"predicate":"rerun the post-edit gate against the same baseline", "finding_absent":finding_id},
            "risk_notes":["Requires a source change and project tests; no executable edit is inferred."]
        })),
    }
}

/// Compare trusted scan documents without reading source or modifying either snapshot.
pub fn check(before: &Document, after: &Document, options: &Options) -> Result<Report> {
    for pattern in &options.scope {
        validate_relative(pattern)?;
    }
    for path in &options.changed_files {
        validate_relative(path)?;
    }
    let scope = crate::collect::globset(&options.scope)?;
    let mut violations = Vec::new();
    for edge in crate::diff::diff(before, after).new_forbidden {
        let rule = edge.rule.as_deref().unwrap_or("forbidden");
        let kind = if matches!(rule, "entrypoints" | "public") {
            "surface-bypass"
        } else {
            "forbidden-edge"
        };
        let recipe = after
            .findings
            .iter()
            .find_map(|finding| match &finding.evidence {
                Evidence::ForbiddenEdge { edge: candidate }
                    if candidate.from == edge.from
                        && candidate.to == edge.to
                        && candidate.kind == edge.kind
                        && candidate.rule == edge.rule
                        && candidate.specifier == edge.specifier
                        && candidate.original_to == edge.original_to =>
                {
                    finding
                        .recipe
                        .as_ref()
                        .map(|recipe| (finding.finding_id.clone(), recipe))
                }
                _ => None,
            })
            .map(|(id, recipe)| serde_json::to_value(recipe).map(|value| (id, value)))
            .transpose()?;
        violations.push(violation(kind, rule, &edge.from,
            json!({"to":edge.to,"kind":edge.kind}), recipe,
            &format!("Remove or redirect the import {} → {} to satisfy {rule}; preserve its callers and verify with project tests.", edge.from, edge.to)));
    }
    // Folder aggregation can turn main -> router -> deps into app <-> api even
    // though no import returns to its starting file. Only newly cyclic runtime
    // import pairs enforce this implicit rule. Comparing pairs also catches a
    // real file cycle added inside an already reciprocal module component.
    let new_cycles = crate::graph::new_cyclic_import_pairs(before, after);
    let module_of: BTreeMap<_, _> = after
        .files
        .iter()
        .map(|file| (&file.path, &file.module))
        .collect();
    for (from, to) in &new_cycles {
        let recipe = after
            .findings
            .iter()
            .filter_map(|finding| {
                let Evidence::Cycle(candidate) = &finding.evidence else {
                    return None;
                };
                let contains = match candidate.level {
                    CycleLevel::File => {
                        candidate.members.contains(from) && candidate.members.contains(to)
                    }
                    CycleLevel::Module => {
                        module_of
                            .get(from)
                            .is_some_and(|m| candidate.members.contains(m))
                            && module_of
                                .get(to)
                                .is_some_and(|m| candidate.members.contains(m))
                    }
                };
                contains.then_some((candidate.level != CycleLevel::File, finding))
            })
            .filter(|(_, finding)| finding.recipe.is_some())
            .min_by_key(|(module_level, finding)| (*module_level, &finding.finding_id))
            .map(|(_, finding)| (finding.finding_id.clone(), finding.recipe.as_ref().unwrap()))
            .map(|(id, recipe)| serde_json::to_value(recipe).map(|value| (id, value)))
            .transpose()?;
        violations.push(violation("cycle", "acyclic", from,
            json!({"from":from,"to":to}), recipe,
            "Break the newly introduced runtime import cycle by redirecting an internal dependency or moving responsibility; preserve behavior and run project tests."));
    }
    if !options.scope.is_empty() {
        for file in &options.changed_files {
            if !scope.is_match(file) {
                violations.push(violation("scope", "scope", file, Value::Null, None,
                    "Revert the out-of-scope edit, or obtain a scope that explicitly includes this file, then verify the edit again."));
            }
        }
    }
    violations.sort_by(|a, b| {
        (&a.kind, &a.file, &a.rule, &a.finding_id).cmp(&(&b.kind, &b.file, &b.rule, &b.finding_id))
    });
    violations.dedup_by(|a, b| a.finding_id == b.finding_id);
    Ok(Report {
        schema: "archgraph/edit-check/v1".into(),
        verdict: if violations.is_empty() {
            "PASS"
        } else {
            "BLOCK"
        }
        .into(),
        violations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Cycle, Edge, Source, Verdict};

    fn document() -> Document {
        let config: Config = serde_json::from_value(
            json!({"depth":1,"source_root":"","excludes":[],"include_tests":false,"rules":null}),
        )
        .unwrap();
        let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":3},"git":null,"generated_at":"1970-01-01T00:00:00Z"})).unwrap();
        crate::analyze(
            source,
            config,
            ["app/a.ts", "domain/b.ts", "infra/c.ts"]
                .map(String::from)
                .to_vec(),
            vec![],
            vec![],
            None,
        )
        .unwrap()
    }

    fn edge(rule: &str) -> Edge {
        Edge {
            from: "domain/b.ts".into(),
            to: "infra/c.ts".into(),
            verdict: Some(Verdict::Forbidden),
            rule: Some(rule.into()),
            ..Edge::default()
        }
    }

    fn cycle(members: &[&str]) -> Cycle {
        Cycle {
            level: CycleLevel::File,
            members: members.iter().map(|s| (*s).into()).collect(),
            exact: true,
            break_set: vec![],
            two_way: None,
            registrar_moves: vec![],
        }
    }

    #[test]
    fn unchanged_and_preexisting_problems_pass() {
        let mut doc = document();
        doc.edges.push(edge("layers"));
        doc.cycles.push(cycle(&["app/a.ts", "domain/b.ts"]));
        assert_eq!(
            check(&doc, &doc, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    #[test]
    fn new_forbidden_surface_and_external_edges_block_with_stable_recipes() {
        let before = document();
        for rule in ["layers", "public", "entrypoints"] {
            let mut after = before.clone();
            after.edges.push(edge(rule));
            let report = check(&before, &after, &Options::default()).unwrap();
            assert_eq!(report.verdict, "BLOCK");
            assert_eq!(report.violations.len(), 1);
            let v = &report.violations[0];
            assert_eq!(
                v.kind,
                if rule == "layers" {
                    "forbidden-edge"
                } else {
                    "surface-bypass"
                }
            );
            assert!(v.finding_id.starts_with("ag-"));
            assert_eq!(v.recipe["mechanical"], false);
            after.edges[0].line = Some(99);
            assert_eq!(
                v.finding_id,
                check(&before, &after, &Options::default())
                    .unwrap()
                    .violations[0]
                    .finding_id
            );
            after.external_edges = after
                .edges
                .drain(..)
                .map(|mut e| {
                    e.to = "external:db".into();
                    e
                })
                .collect();
            assert_eq!(
                check(&before, &after, &Options::default()).unwrap().verdict,
                "BLOCK"
            );
        }
    }

    #[test]
    fn attached_recipes_are_reused_without_promoting_advisory_surface_findings() {
        let before = document();
        let mut after = before.clone();
        after.edges.push(edge("layers"));
        crate::recipes::attach(&mut after).unwrap();
        let finding = after
            .findings
            .iter()
            .find(|f| matches!(f.evidence, Evidence::ForbiddenEdge { .. }))
            .unwrap();
        let expected = serde_json::to_value(finding.recipe.as_ref().unwrap()).unwrap();
        let report = check(&before, &after, &Options::default()).unwrap();
        assert_eq!(report.violations[0].recipe, expected);
        assert_eq!(report.violations[0].finding_id, finding.finding_id);
        after.edges.clear();
        // Inferred surface evidence alone does not declare an enforcement policy.
        after.surface = Some(crate::surface::SurfaceReport {
            schema: "archgraph/surface/v1".into(),
            kind: "surface".into(),
            max_symbols: 100,
            complete: true,
            limitations: vec![],
            modules: vec![crate::surface::ModuleSurface {
                module: "infra".into(),
                entrypoints: vec!["infra/index.ts".into()],
                exported: vec![],
                used_externally: vec![],
                could_be_internal: vec![],
                oversized: false,
                deep_imports: vec![crate::surface::DeepImport {
                    from: "domain/b.ts".into(),
                    to: "infra/c.ts".into(),
                    symbols: vec!["C".into()],
                    suggestion: "Use the facade".into(),
                }],
            }],
        });
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    fn imports(pairs: &[(&str, &str)]) -> Document {
        let template = document();
        let files = [
            "app/main.py",
            "app/deps.py",
            "venues/api/router.py",
            "app/old.py",
            "venues/api/old.py",
            "infra/extra.py",
        ]
        .map(String::from)
        .to_vec();
        let mut source = template.source;
        source.language = crate::model::Language::Python;
        crate::analyze(
            source,
            template.config,
            files,
            pairs
                .iter()
                .map(|(from, to)| Edge {
                    from: (*from).into(),
                    to: (*to).into(),
                    ..Edge::default()
                })
                .collect(),
            vec![],
            None,
        )
        .unwrap()
    }

    #[test]
    fn fastapi_composition_dag_does_not_block() {
        let before = imports(&[]);
        let after = imports(&[
            ("app/main.py", "venues/api/router.py"),
            ("venues/api/router.py", "app/deps.py"),
        ]);
        assert!(after.cycles.iter().any(|c| c.level == CycleLevel::Module));
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    #[test]
    fn new_file_cycle_blocks_inside_preexisting_module_cycle() {
        let before = imports(&[
            ("app/main.py", "venues/api/router.py"),
            ("venues/api/router.py", "app/deps.py"),
        ]);
        let mut pairs = before
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect::<Vec<_>>();
        pairs.push(("venues/api/router.py", "app/main.py"));
        let after = imports(&pairs);
        let report = check(&before, &after, &Options::default()).unwrap();
        assert_eq!(report.verdict, "BLOCK");
        assert!(report.violations.iter().all(|v| v.kind == "cycle"));
        assert_eq!(
            check(&after, &after, &Options::default()).unwrap().verdict,
            "PASS"
        );
        assert_eq!(
            check(&after, &before, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    #[test]
    fn existing_file_cycle_does_not_block_unrelated_composition_edges() {
        let before = imports(&[
            ("app/old.py", "venues/api/old.py"),
            ("venues/api/old.py", "app/old.py"),
        ]);
        let after = imports(&[
            ("app/old.py", "venues/api/old.py"),
            ("venues/api/old.py", "app/old.py"),
            ("app/main.py", "venues/api/router.py"),
            ("venues/api/router.py", "app/deps.py"),
        ]);
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    #[test]
    fn growing_file_cycle_blocks_but_removing_members_passes() {
        let before = imports(&[
            ("app/main.py", "venues/api/router.py"),
            ("venues/api/router.py", "app/main.py"),
        ]);
        let after = imports(&[
            ("app/main.py", "venues/api/router.py"),
            ("venues/api/router.py", "app/main.py"),
            ("venues/api/router.py", "infra/extra.py"),
            ("infra/extra.py", "app/main.py"),
        ]);
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "BLOCK"
        );
        assert_eq!(
            check(&after, &before, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    #[test]
    fn go_package_cycles_block_without_file_cycles_but_old_package_debt_passes() {
        let go = |pairs: &[(&str, &str)]| {
            let mut doc = imports(pairs);
            doc.source.language = crate::model::Language::Go;
            for file in &mut doc.files {
                file.path = file.path.replace(".py", ".go");
            }
            for edge in &mut doc.edges {
                edge.from = edge.from.replace(".py", ".go");
                edge.to = edge.to.replace(".py", ".go");
            }
            doc
        };
        // Go's compilation unit is a physical directory, independent of which
        // file the collector uses to represent an import into that package.
        let before = go(&[
            ("app/main.py", "venues/api/router.py"),
            ("venues/api/old.py", "app/deps.py"),
        ]);
        assert_eq!(
            check(&go(&[]), &before, &Options::default())
                .unwrap()
                .verdict,
            "BLOCK"
        );
        let after = go(&[
            ("app/main.py", "venues/api/old.py"),
            ("venues/api/old.py", "app/deps.py"),
            ("app/old.py", "venues/api/router.py"),
        ]);
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "PASS"
        );
    }

    #[test]
    fn scope_checks_added_and_deleted_paths_and_deduplicates() {
        let doc = document();
        let options = Options {
            scope: vec!["domain/**".into()],
            changed_files: vec![
                "domain/new.ts".into(),
                "app/deleted.ts".into(),
                "app/deleted.ts".into(),
            ],
        };
        let report = check(&doc, &doc, &options).unwrap();
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].file, "app/deleted.ts");
        assert_eq!(report.violations[0].kind, "scope");
        for invalid in ["/tmp/a", "../a", "a/../b", "a\\b", "C:/a", "a\nb"] {
            assert!(
                check(
                    &doc,
                    &doc,
                    &Options {
                        changed_files: vec![invalid.into()],
                        ..Options::default()
                    }
                )
                .is_err()
            );
            assert!(
                check(
                    &doc,
                    &doc,
                    &Options {
                        scope: vec![invalid.into()],
                        ..Options::default()
                    }
                )
                .is_err()
            );
        }
        assert!(
            check(
                &doc,
                &doc,
                &Options {
                    scope: vec!["[".into()],
                    ..Options::default()
                }
            )
            .is_err()
        );
    }
}
