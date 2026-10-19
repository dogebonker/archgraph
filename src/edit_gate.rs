//! Pure pre-edit/post-edit enforcement; existing violations remain advisory.
use std::collections::{BTreeMap, BTreeSet};

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
    for cycle in &after.cycles {
        let members: BTreeSet<_> = cycle.members.iter().collect();
        if before.cycles.iter().any(|old| {
            old.level == cycle.level && members.iter().all(|member| old.members.contains(member))
        }) {
            continue;
        }
        let module_of: BTreeMap<_, _> = after
            .files
            .iter()
            .map(|file| (&file.path, &file.module))
            .collect();
        let witness = after
            .edges
            .iter()
            .filter(|edge| edge.kind.is_runtime())
            .filter(|edge| match cycle.level {
                CycleLevel::File => members.contains(&edge.from) && members.contains(&edge.to),
                CycleLevel::Module => {
                    module_of
                        .get(&edge.from)
                        .is_some_and(|m| members.contains(m))
                        && module_of.get(&edge.to).is_some_and(|m| members.contains(m))
                        && module_of.get(&edge.from) != module_of.get(&edge.to)
                }
            })
            .min_by_key(|edge| (&edge.from, &edge.to));
        // Module components can lack source edges in older persisted documents.
        let file = witness
            .map(|edge| edge.from.as_str())
            .or_else(|| {
                after
                    .files
                    .iter()
                    .filter(|file| members.contains(&file.module) || members.contains(&file.path))
                    .map(|file| file.path.as_str())
                    .min()
            })
            .unwrap_or("");
        let recipe = after
            .findings
            .iter()
            .find_map(|finding| match &finding.evidence {
                Evidence::Cycle(candidate)
                    if candidate.level == cycle.level
                        && candidate.members.iter().collect::<BTreeSet<_>>() == members =>
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
        violations.push(violation("cycle", "acyclic", file,
            json!({"level":cycle.level,"members":members}), recipe,
            "Break the newly introduced cyclic component by redirecting an internal dependency or moving responsibility; preserve behavior and run project tests."));
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

    #[test]
    fn new_grown_and_merged_cycles_block_but_subsets_pass() {
        let mut before = document();
        let mut after = before.clone();
        after.cycles.push(cycle(&["app/a.ts", "domain/b.ts"]));
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "BLOCK"
        );
        before = after.clone();
        after.cycles[0].members.push("infra/c.ts".into());
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "BLOCK"
        );
        assert_eq!(
            check(&after, &before, &Options::default()).unwrap().verdict,
            "PASS"
        );
        before.cycles.push(cycle(&["infra/c.ts"]));
        assert_eq!(
            check(&before, &after, &Options::default()).unwrap().verdict,
            "BLOCK"
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
