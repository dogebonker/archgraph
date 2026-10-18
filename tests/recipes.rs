use archgraph::{
    model::{Config, Edge, EdgeKind, Evidence, FindingKind, ImportRecord, Language, Source},
    recipes::{self, Confidence, Predicate, Rank, Recipe, StepKind, Verification},
};
use serde_json::json;

fn document(
    language: Language,
    files: &[&str],
    links: &[(&str, &str)],
) -> archgraph::model::Document {
    let mut config: Config = serde_json::from_value(json!({
        "depth":1,"source_root":"","excludes":[],"include_tests":false,"rules":null
    }))
    .unwrap();
    config.import_records = Some(
        links
            .iter()
            .enumerate()
            .map(|(i, (from, to))| ImportRecord {
                from: (*from).into(),
                specifier: format!("./target-{i}"),
                line: Some(i + 1),
                kind: EdgeKind::Static,
                targets: vec![(*to).into()],
                ..ImportRecord::default()
            })
            .collect(),
    );
    let edges = links
        .iter()
        .enumerate()
        .map(|(i, (from, to))| Edge {
            from: (*from).into(),
            to: (*to).into(),
            specifier: Some(format!("./target-{i}")),
            line: Some(i + 1),
            ..Edge::default()
        })
        .collect();
    let source: Source=serde_json::from_value(json!({"language":language,"collector":{"name":"fixture","cruised":files.len()},"git":null,"generated_at":"1970-01-01T00:00:00Z"})).unwrap();
    archgraph::analyze(
        source,
        config,
        files.iter().map(|s| (*s).into()).collect(),
        edges,
        vec![],
        None,
    )
    .unwrap()
}

#[test]
fn cycle_cuts_are_measured_verifiable_and_ids_stable_for_all_languages() {
    for (language, ext) in [
        (Language::Typescript, "ts"),
        (Language::Python, "py"),
        (Language::Go, "go"),
        (Language::Rust, "rs"),
        (Language::Java, "java"),
        (Language::Elixir, "ex"),
    ] {
        let a = format!("app/a.{ext}");
        let b = format!("domain/b.{ext}");
        let mut doc = document(language, &[&a, &b], &[(&a, &b), (&b, &a)]);
        let finding = doc
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::Cycle)
            .unwrap();
        let id = finding.finding_id.clone();
        let recipe = finding.recipe.clone().unwrap();
        assert!(
            recipe
                .steps
                .iter()
                .all(|s| s.kind == StepKind::CutImport && s.mechanical)
        );
        assert_eq!(recipe.expected.pairs_removed, Some(1));
        assert_eq!(recipe.expected.cycle_size_after, Some(0));
        let projected = recipes::project(&doc, &recipe).unwrap();
        assert!(recipes::check(&doc, &projected, &recipe).pass);
        assert!(!recipes::check(&doc, &doc, &recipe).pass);
        doc.source.generated_at = "tomorrow".into();
        recipes::attach(&mut doc).unwrap();
        assert!(doc.findings.iter().any(|f| f.finding_id == id));
        assert!(
            doc.diagnostics
                .iter()
                .any(|d| d.finding.kind == FindingKind::SmallRepo)
        );
    }
}

#[test]
fn stable_identity_preserves_task_two_mcp_ids() {
    let doc = document(
        Language::Typescript,
        &["a/a.ts", "b/b.ts"],
        &[("a/a.ts", "b/b.ts"), ("b/b.ts", "a/a.ts")],
    );
    for finding in &doc.findings {
        assert_eq!(
            recipes::stable_id(finding),
            archgraph::mcp_tools::finding_id(finding)
        );
    }
}

#[test]
fn unresolved_ambiguous_cut_is_instructions_and_never_claims_measured_deltas() {
    let mut doc = document(
        Language::Typescript,
        &["a/a.ts", "b/b.ts"],
        &[("a/a.ts", "b/b.ts"), ("b/b.ts", "a/a.ts")],
    );
    doc.imports = None;
    recipes::attach(&mut doc).unwrap();
    let recipe = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Cycle)
        .unwrap()
        .recipe
        .as_ref()
        .unwrap();
    assert!(recipe.steps.iter().any(|s| !s.mechanical));
    assert_eq!(recipe.expected.pairs_removed, None);
    assert!(recipes::project(&doc, recipe).is_err());
}

#[test]
fn forbidden_edges_get_recipes_and_external_ledger_is_removed_consistently() {
    let mut doc = document(
        Language::Typescript,
        &["a/a.ts", "b/b.ts"],
        &[("a/a.ts", "b/b.ts")],
    );
    doc.edges[0].verdict = Some(archgraph::model::Verdict::Forbidden);
    recipes::attach(&mut doc).unwrap();
    let f = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::ForbiddenEdge)
        .unwrap();
    let after = recipes::project(&doc, f.recipe.as_ref().unwrap()).unwrap();
    assert!(after.edges.is_empty());
    assert!(after.imports.unwrap().is_empty());
    assert!(matches!(f.evidence, Evidence::ForbiddenEdge { .. }));
}

#[test]
fn ranking_uses_confidence_and_benefit_and_legacy_restores_kind_order() {
    let mut doc = document(
        Language::Typescript,
        &["a/a.ts", "b/b.ts"],
        &[("a/a.ts", "b/b.ts"), ("b/b.ts", "a/a.ts")],
    );
    let mut other = doc.findings[0].clone();
    other.subject = "other".into();
    other.finding_id = recipes::stable_id(&other);
    other.recipe.as_mut().unwrap().expected.pairs_removed = Some(5);
    other.recipe.as_mut().unwrap().confidence = Confidence::Low;
    doc.findings.push(other);
    recipes::rank(&mut doc, Rank::Recipe);
    assert_eq!(doc.findings[0].subject, "other");
    recipes::rank(&mut doc, Rank::Legacy);
    assert_ne!(doc.findings[0].subject, "other");
}

#[test]
fn explicit_predicate_checks_do_not_pass_vacuously() {
    let doc = document(Language::Typescript, &["a/a.ts"], &[]);
    let mut recipe = Recipe {
        steps: vec![],
        expected: Default::default(),
        verify: Verification {
            predicate: String::new(),
            structured: Predicate::default(),
        },
        confidence: Confidence::Low,
        risk_notes: vec![],
    };
    assert!(!recipes::check(&doc, &doc, &recipe).pass);
    recipe.verify.structured.files_present = vec!["a/a.ts".into()];
    assert!(recipes::check(&doc, &doc, &recipe).pass);
}

#[test]
fn named_forbidden_dependency_gets_consumer_owned_interface_instructions() {
    for (language, ext) in [
        (Language::Typescript, "ts"),
        (Language::Python, "py"),
        (Language::Go, "go"),
        (Language::Rust, "rs"),
        (Language::Java, "java"),
        (Language::Elixir, "ex"),
    ] {
        let from = format!("domain/model.{ext}");
        let to = format!("adapter/db.{ext}");
        let mut doc = document(language, &[&from, &to], &[(&from, &to)]);
        doc.edges[0].verdict = Some(archgraph::model::Verdict::Forbidden);
        doc.edges[0].names = vec!["Database".into()];
        recipes::attach(&mut doc).unwrap();
        let recipe = doc
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::ForbiddenEdge)
            .unwrap()
            .recipe
            .as_ref()
            .unwrap();
        assert_eq!(recipe.steps[0].kind, StepKind::ExtractInterface);
        assert!(recipe.steps[0].to.starts_with("domain/ports/"));
        assert!(!recipe.steps[0].mechanical);
        assert_eq!(recipe.confidence, Confidence::Medium);
        assert!(recipe.expected.pairs_removed.is_none());
    }
}

#[test]
fn deep_import_recipe_reexports_then_retargets_written_boundary() {
    use archgraph::surface::{DeepImport, ModuleSurface, SurfaceReport};
    let mut doc = document(
        Language::Typescript,
        &["app/main.ts", "domain/index.ts", "domain/thing.ts"],
        &[("app/main.ts", "domain/thing.ts")],
    );
    doc.files
        .iter_mut()
        .find(|f| f.path == "domain/index.ts")
        .unwrap()
        .barrel = true;
    doc.config
        .file_metadata
        .entry("domain/index.ts".into())
        .or_default()
        .barrel = true;
    doc.surface = Some(SurfaceReport {
        schema: archgraph::model::SCHEMA.into(),
        kind: "surface".into(),
        max_symbols: 50,
        complete: true,
        modules: vec![ModuleSurface {
            module: "domain".into(),
            entrypoints: vec!["domain/index.ts".into()],
            exported: vec![],
            used_externally: vec![],
            could_be_internal: vec![],
            deep_imports: vec![DeepImport {
                from: "app/main.ts".into(),
                to: "domain/thing.ts".into(),
                symbols: vec!["Thing".into()],
                suggestion: "use entry".into(),
            }],
            oversized: false,
        }],
        limitations: vec![],
    });
    recipes::attach(&mut doc).unwrap();
    let recipe = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::DeepImport)
        .unwrap()
        .recipe
        .as_ref()
        .unwrap();
    assert_eq!(
        recipe.steps.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [StepKind::AddReexport, StepKind::CutImport]
    );
    let after = recipes::project(&doc, recipe).unwrap();
    assert!(after.edges.iter().any(|e| e.from == "app/main.ts"
        && e.original_to.as_deref() == Some("domain/index.ts")
        && e.to == "domain/thing.ts"));
    assert!(
        after
            .edges
            .iter()
            .any(|e| e.from == "domain/index.ts" && e.to == "domain/thing.ts" && e.is_reexport)
    );
    assert!(recipes::check(&doc, &after, recipe).pass);
}

#[test]
fn split_recipe_uses_actual_cochange_groups_without_inventing_churn() {
    use archgraph::model::{Cochange, CochangePair};
    let mut doc = document(
        Language::Typescript,
        &["utils/a.ts", "utils/b.ts", "utils/c.ts", "app/main.ts"],
        &[],
    );
    let mut finding = doc.diagnostics[0].finding.clone();
    finding.kind = FindingKind::CycleViaGrabbag;
    finding.subject = "utils".into();
    finding.evidence = Evidence::Grabbag(archgraph::grabbag::Evidence {
        members: vec!["utils".into(), "app".into()],
        candidates: vec![archgraph::grabbag::Candidate {
            module: "utils".into(),
            cohesion: 0.0,
            fan_in: 5.0,
            internal_edges_per_file: 0.0,
            files: vec![
                "utils/a.ts".into(),
                "utils/b.ts".into(),
                "utils/c.ts".into(),
            ],
        }],
        edges: vec![],
        remaining_cycles: vec![],
    });
    doc.findings.push(finding);
    doc.cochange = Some(Cochange {
        limit: 100,
        commits: 20,
        skipped: 0,
        pairs: vec![CochangePair {
            files: ["utils/a.ts".into(), "utils/b.ts".into()],
            commits: 9,
        }],
    });
    recipes::attach(&mut doc).unwrap();
    let r = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::CycleViaGrabbag)
        .unwrap()
        .recipe
        .as_ref()
        .unwrap();
    assert_eq!(r.steps[0].kind, StepKind::SplitModule);
    assert_eq!(r.steps[0].detail["evidence_source"], "cochange");
    assert_eq!(
        r.steps[0].detail["groups"][0]["files"],
        json!(["utils/a.ts", "utils/b.ts"])
    );
    assert_eq!(r.expected.pairs_removed, None);
    assert!(!r.steps[0].mechanical);
}

#[test]
fn file_cycle_rename_does_not_count_as_a_new_cycle_and_numeric_mismatch_fails() {
    use archgraph::{
        model::Move,
        recipes::{Expected, Step},
    };
    let doc = document(
        Language::Typescript,
        &["app/a.ts", "app/b.ts"],
        &[("app/a.ts", "app/b.ts"), ("app/b.ts", "app/a.ts")],
    );
    let m = Move {
        file: "app/a.ts".into(),
        to: "app/renamed.ts".into(),
    };
    let mut recipe = Recipe {
        steps: vec![Step {
            kind: StepKind::MoveFile,
            file: m.file.clone(),
            line: None,
            from: m.file.clone(),
            to: m.to.clone(),
            detail: json!({"operation":"move_file"}),
            mechanical: true,
        }],
        expected: Expected::default(),
        verify: Verification {
            predicate: "rename".into(),
            structured: Predicate {
                files_present: vec![m.to],
                files_absent: vec![m.file],
                ..Predicate::default()
            },
        },
        confidence: Confidence::High,
        risk_notes: vec![],
    };
    recipe.verify.structured.scope = vec!["app/a.ts".into(), "app/b.ts".into()];
    recipes::forecast(&doc, &mut recipe).unwrap();
    assert_eq!(recipe.expected.cycle_size_after, Some(2));
    assert_eq!(recipe.expected.new_cycles, Some(0));
    let after = recipes::project(&doc, &recipe).unwrap();
    assert!(recipes::check(&doc, &after, &recipe).pass);
    recipe
        .verify
        .structured
        .expected
        .as_mut()
        .unwrap()
        .pairs_removed = Some(10);
    let result = recipes::check(&doc, &after, &recipe);
    assert!(!result.pass);
    assert!(
        result.checks.iter().any(|c| c.predicate == "pairs_removed"
            && c.expected == 10
            && c.actual == 0
            && !c.pass)
    );
}

#[test]
fn external_cut_removes_package_edges_and_accounting_record() {
    let mut doc = document(Language::Typescript, &["domain/model.ts"], &[]);
    doc.config.import_records = Some(vec![ImportRecord {
        from: "domain/model.ts".into(),
        specifier: "some-orm".into(),
        line: Some(1),
        external: Some("some-orm".into()),
        ..ImportRecord::default()
    }]);
    doc = archgraph::analyze(
        doc.source,
        doc.config,
        vec!["domain/model.ts".into()],
        vec![],
        vec![],
        None,
    )
    .unwrap();
    doc.external_edges[0].verdict = Some(archgraph::model::Verdict::Forbidden);
    recipes::attach(&mut doc).unwrap();
    let recipe = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::ForbiddenEdge)
        .unwrap()
        .recipe
        .as_ref()
        .unwrap();
    assert!(recipe.steps[0].mechanical);
    let after = recipes::project(&doc, recipe).unwrap();
    assert!(after.external_edges.is_empty());
    assert!(after.external_nodes.is_empty());
    assert_eq!(after.import_resolution.unwrap().imports, 0);
}

#[test]
fn duplicate_occurrences_are_not_misrepresented_as_a_single_mechanical_cut() {
    let mut doc = document(
        Language::Typescript,
        &["a/a.ts", "b/b.ts"],
        &[("a/a.ts", "b/b.ts"), ("b/b.ts", "a/a.ts")],
    );
    let mut duplicate = doc.imports.as_ref().unwrap()[0].clone();
    duplicate.line = Some(99);
    duplicate.specifier = "./another-alias".into();
    doc.imports.as_mut().unwrap().push(duplicate);
    recipes::attach(&mut doc).unwrap();
    let recipe = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Cycle)
        .unwrap()
        .recipe
        .as_ref()
        .unwrap();
    if recipe.steps[0].file == "a/a.ts" {
        assert!(!recipe.steps[0].mechanical);
        assert!(recipe.expected.pairs_removed.is_none());
    }
}

#[test]
fn prediction_scope_does_not_assert_a_partial_fix_eliminates_the_entire_cycle() {
    let doc = document(
        Language::Typescript,
        &["a/a.ts", "b/b.ts", "c/c.ts"],
        &[
            ("a/a.ts", "b/b.ts"),
            ("b/b.ts", "a/a.ts"),
            ("b/b.ts", "c/c.ts"),
            ("c/c.ts", "b/b.ts"),
            ("c/c.ts", "a/a.ts"),
        ],
    );
    let recipe = doc
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Cycle)
        .unwrap()
        .recipe
        .as_ref()
        .unwrap();
    assert!(!recipe.verify.structured.scope.is_empty());
    assert!(recipe.verify.structured.cycles_in.is_empty());
    assert_eq!(recipe.verify.structured.equals, None);
    let after = recipes::project(&doc, recipe).unwrap();
    assert!(recipes::check(&doc, &after, recipe).pass);
    assert!(!after.cycles.is_empty());
}

#[test]
fn predicates_remain_compact_strict_and_preserve_forbidden_assertions() {
    let encoded = serde_json::to_value(Predicate::default()).unwrap();
    assert_eq!(encoded, json!({}));
    assert!(serde_json::from_value::<Predicate>(json!({"typo":true})).is_err());
    let mut predicate: Predicate =
        serde_json::from_value(json!({"forbidden":0,"baseline_id":"receipt-123","scope":["a"]}))
            .unwrap();
    assert_eq!(predicate.baseline_id.as_deref(), Some("receipt-123"));
    let mut doc = document(Language::Typescript, &["a/a.ts"], &[]);
    let mut recipe = Recipe {
        steps: vec![],
        expected: Default::default(),
        verify: Verification {
            predicate: "all(structured)".into(),
            structured: predicate.clone(),
        },
        confidence: Confidence::Low,
        risk_notes: vec![],
    };
    assert!(recipes::check(&doc, &doc, &recipe).pass);
    doc.summary.forbidden = 1;
    assert!(!recipes::check(&doc, &doc, &recipe).pass);
    predicate.cycles_in = vec!["a".into()];
    predicate.equals = Some(0);
    recipe.verify.structured = predicate;
    assert!(
        recipes::check(&doc, &doc, &recipe)
            .checks
            .iter()
            .any(|c| c.predicate == "cycles_in")
    );
}

#[test]
fn diagnostic_provenance_survives_saved_scan_restore_and_render() {
    let mut doc = document(Language::Typescript, &["app/a.ts"], &[]);
    let diagnostic = &mut doc.diagnostics[0].finding;
    diagnostic.age_days = Some(19);
    diagnostic.introduced_by = Some(
        serde_json::from_value(json!({
            "commit":"0123456789012345678901234567890123456789",
            "author":"Fixture Author","email":"fixture@example.invalid",
            "authored_at":"2026-10-01T00:00:00Z","committed_at":"2026-10-01T00:00:00Z",
            "subject":"introduce diagnostic evidence","pull_request":null,"age_days":19
        }))
        .unwrap(),
    );
    diagnostic.provenance = Some(archgraph::model::Provenance {
        status: "verified-transition".into(),
        method: "scan-bisect".into(),
        from: Some("before".into()),
        to: "after".into(),
        previous_commit: Some("before".into()),
        closing_edge: None,
        limitations: vec![],
        scans: 2,
    });
    let saved = serde_json::to_value(&doc).unwrap();
    let restored = archgraph::restore_scan(serde_json::from_value(saved.clone()).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&restored).unwrap()["diagnostics"],
        saved["diagnostics"]
    );
    let html = archgraph::render::render(&restored).unwrap();
    assert!(html.contains("0123456789012345678901234567890123456789"));
    // Older schemas stored this same evidence in findings; classification changes retain dates.
    let mut legacy = saved;
    legacy["schema"] = json!("archgraph/v1.1");
    legacy["findings"] = json!([legacy["diagnostics"][0]["finding"].clone()]);
    legacy.as_object_mut().unwrap().remove("diagnostics");
    let restored = archgraph::restore_scan(serde_json::from_value(legacy).unwrap()).unwrap();
    let finding = &restored.diagnostics[0].finding;
    assert_eq!(finding.age_days, Some(19));
    assert_eq!(
        finding.provenance.as_ref().unwrap().status,
        "verified-transition"
    );
}
