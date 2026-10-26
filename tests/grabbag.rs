use boundrail::{
    grabbag::Thresholds,
    model::{Config, Document, Edge, EdgeKind, Granularity, Language, Source},
    rules::{Compiled, Rules},
};
use serde_json::json;

fn scan(pairs: &[(&str, &str)], options: serde_json::Value) -> Document {
    scan_language(pairs, options, Language::Typescript)
}

fn scan_language(
    pairs: &[(&str, &str)],
    options: serde_json::Value,
    language: Language,
) -> Document {
    let mut config = json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1}}});
    for (key, value) in options.as_object().unwrap() {
        config[key] = value.clone();
    }
    let config: Config = serde_json::from_value(config).unwrap();
    let source: Source = serde_json::from_value(json!({"language":language,"collector":{"name":"fixture","cruised":0},"generated_at":"test"})).unwrap();
    let mut edges: Vec<_> = pairs
        .iter()
        .map(|(from, to)| Edge {
            from: (*from).into(),
            to: (*to).into(),
            ..Edge::default()
        })
        .collect();
    if config.all_kinds {
        for edge in &mut edges {
            edge.kind = EdgeKind::Type;
        }
    }
    let files = edges
        .iter()
        .flat_map(|e| [e.from.clone(), e.to.clone()])
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    boundrail::analyze(source, config, files, edges, Vec::new(), None).unwrap()
}

fn consumers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("c1/x.ts", "utils/date.ts"),
        ("c2/x.ts", "utils/date.ts"),
        ("c3/x.ts", "utils/date.ts"),
        ("c4/x.ts", "utils/date.ts"),
        ("c5/x.ts", "utils/date.ts"),
    ]
}

#[test]
fn disconnected_helper_files_reclassify_a_false_folder_cycle() {
    let mut edges = consumers();
    edges.extend([("app/a.ts", "utils/date.ts"), ("utils/work.ts", "app/a.ts")]);
    let doc = scan(&edges, json!({}));
    assert!(doc.cycles.is_empty());
    assert_eq!(doc.grabbags.len(), 1);
    let evidence = &doc.grabbags[0];
    assert_eq!(evidence.members, ["app", "utils"]);
    assert_eq!(
        evidence.candidates[0].files,
        ["utils/date.ts", "utils/work.ts"]
    );
    assert_eq!(evidence.candidates[0].fan_in, 6.0);
    assert_eq!(evidence.candidates[0].cohesion, 0.0);
    assert!(evidence.remaining_cycles.is_empty());
    assert!(
        doc.findings
            .iter()
            .any(|f| f.kind == boundrail::model::FindingKind::CycleViaGrabbag)
    );
}

#[test]
fn a_real_internal_path_keeps_the_cycle_even_when_metrics_qualify() {
    let mut edges = consumers();
    edges.extend([
        ("app/a.ts", "utils/date.ts"),
        ("utils/date.ts", "utils/work.ts"),
        ("utils/work.ts", "app/a.ts"),
    ]);
    let doc = scan(
        &edges,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"cohesion_max":1.0,"internal_edges_per_file_max":0.6}}}}),
    );
    assert_eq!(doc.cycles.len(), 1);
    assert_eq!(doc.cycles[0].members, ["app", "utils"]);
    assert!(doc.grabbags.is_empty());
}

#[test]
fn smaller_genuine_residual_keeps_real_module_ids_and_cut_edges() {
    let mut edges = consumers();
    edges.extend([
        ("a/x.ts", "utils/date.ts"),
        ("utils/work.ts", "b/x.ts"),
        ("b/x.ts", "a/x.ts"),
        ("a/x.ts", "b/x.ts"),
    ]);
    let doc = scan(&edges, json!({}));
    assert_eq!(doc.grabbags.len(), 1);
    assert_eq!(doc.grabbags[0].members, ["a", "b", "utils"]);
    assert_eq!(doc.grabbags[0].remaining_cycles, [vec!["a", "b"]]);
    assert_eq!(doc.cycles.len(), 1);
    assert_eq!(doc.cycles[0].members, ["a", "b"]);
    assert!(!doc.cycles[0].break_set.is_empty());
    for cut in &doc.cycles[0].break_set {
        assert!(
            doc.edges
                .iter()
                .any(|e| e.from == cut.from && e.to == cut.to)
        );
        assert!(!cut.from.contains('\0') && !cut.to.contains('\0'));
    }
}

#[test]
fn two_candidates_in_one_scc_produce_one_reclassification() {
    let mut edges = consumers();
    edges.extend([
        ("c1/x.ts", "common/date.ts"),
        ("c2/x.ts", "common/date.ts"),
        ("c3/x.ts", "common/date.ts"),
        ("c4/x.ts", "common/date.ts"),
        ("c5/x.ts", "common/date.ts"),
        ("app/a.ts", "utils/date.ts"),
        ("utils/work.ts", "common/date.ts"),
        ("common/work.ts", "app/a.ts"),
    ]);
    let doc = scan(&edges, json!({}));
    assert!(doc.cycles.is_empty());
    assert_eq!(doc.grabbags.len(), 1);
    assert_eq!(doc.grabbags[0].candidates.len(), 2);
}

#[test]
fn threshold_file_and_kind_guards_do_not_hide_findings() {
    let mut edges = consumers();
    edges.extend([("app/a.ts", "utils/date.ts"), ("utils/work.ts", "app/a.ts")]);
    let doc = scan(
        &edges,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"fan_in_min":7.0}}}}),
    );
    assert_eq!(doc.cycles.len(), 1);
    assert!(doc.grabbags.is_empty());
    let doc = scan(&edges, json!({"granularity":"file"}));
    assert_eq!(doc.config.granularity, Granularity::File);
    assert!(doc.cycles.is_empty());
    assert!(doc.grabbags.is_empty());
    let doc = scan(
        &edges,
        json!({"all_kinds":true,"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"fan_in_min":0.5}}}}),
    );
    assert!(doc.cycles.is_empty());
    assert_eq!(doc.grabbags.len(), 1);
    let mut config = doc.config.clone();
    config.all_kinds = false;
    let deferred = boundrail::analyze(
        doc.source.clone(),
        config,
        doc.files.iter().map(|f| f.path.clone()).collect(),
        doc.edges.clone(),
        Vec::new(),
        None,
    )
    .unwrap();
    assert!(deferred.cycles.is_empty());
    assert!(deferred.grabbags.is_empty());
    assert!(!deferred.type_only_cycles.is_empty());
    let doc = scan(
        &[
            ("app/x.ts", "tests/back.ts"),
            ("tests/back.ts", "app/x.ts"),
            ("tests/unused.ts", "app/x.ts"),
        ],
        json!({"include_tests":true}),
    );
    assert!(doc.cycles.is_empty());
    assert!(doc.grabbags.is_empty());
}

#[test]
fn threshold_config_defaults_ranges_and_typos_are_checked() {
    let rules: Rules = serde_yaml_ng::from_str("version: 1\ngrabbag: {fan_in_min: 3}").unwrap();
    assert_eq!(rules.grabbag.unwrap().cohesion_max, 0.2);
    Compiled::new(&rules).unwrap();
    for value in [-0.1, f64::INFINITY, f64::NAN, 1.1] {
        let rules = Rules {
            version: 1,
            grabbag: Some(Thresholds {
                cohesion_max: value,
                ..Thresholds::default()
            }),
            ..Rules::default()
        };
        assert!(Compiled::new(&rules).is_err());
    }
    for thresholds in [
        Thresholds {
            fan_in_min: -1.0,
            ..Thresholds::default()
        },
        Thresholds {
            internal_edges_per_file_max: f64::NAN,
            ..Thresholds::default()
        },
    ] {
        assert!(thresholds.validate().is_err());
    }
    let path = std::env::temp_dir().join(format!("archgraph-grabbag-{}.yaml", std::process::id()));
    std::fs::write(&path, "version: 1\ngrabbag: {fan_in_min: 3}").unwrap();
    boundrail::rules::load(&path).unwrap();
    std::fs::write(&path, "version: 1\ngrabbag: {fan_in_mni: 3}").unwrap();
    assert!(
        format!("{:#}", boundrail::rules::load(&path).unwrap_err())
            .contains("unknown field `fan_in_mni` in grabbag")
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn existing_internal_file_cycles_survive_folder_reclassification() {
    let mut edges = consumers();
    edges.extend([
        ("app/a.ts", "utils/date.ts"),
        ("utils/work.ts", "app/a.ts"),
        ("utils/counter-a.ts", "utils/counter-b.ts"),
        ("utils/counter-b.ts", "utils/counter-a.ts"),
    ]);
    let doc = scan(
        &edges,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"cohesion_max":1.0,"internal_edges_per_file_max":0.6}}}}),
    );
    assert_eq!(doc.grabbags.len(), 1);
    assert_eq!(
        doc.grabbags[0].candidates[0].files,
        ["utils/date.ts", "utils/work.ts"]
    );
    assert_eq!(doc.cycles.len(), 1);
    assert_eq!(doc.cycles[0].level, boundrail::model::CycleLevel::File);
    assert_eq!(
        doc.cycles[0].members,
        ["utils/counter-a.ts", "utils/counter-b.ts"]
    );
}

#[test]
fn overlapping_mapped_residuals_keep_the_original_conservatively() {
    let mut edges = consumers();
    edges.extend([
        ("a/x.ts", "utils/date.ts"),
        ("utils/date.ts", "a/x.ts"),
        ("b/x.ts", "utils/work.ts"),
        ("utils/work.ts", "b/x.ts"),
    ]);
    let doc = scan(&edges, json!({}));
    assert!(doc.grabbags.is_empty());
    assert_eq!(doc.cycles.len(), 1);
    assert_eq!(doc.cycles[0].members, ["a", "b", "utils"]);
}

#[test]
fn upper_thresholds_are_strict_and_fan_in_minimum_is_inclusive() {
    let mut edges = consumers();
    edges.extend([("app/a.ts", "utils/date.ts"), ("utils/work.ts", "app/a.ts")]);
    let exactly_low = scan(
        &edges,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"cohesion_max":0.0}}}}),
    );
    assert!(exactly_low.grabbags.is_empty());
    let exactly_dense = scan(
        &edges,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"internal_edges_per_file_max":0.0}}}}),
    );
    assert!(exactly_dense.grabbags.is_empty());
    let exactly_fan_in = scan(
        &edges,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"fan_in_min":6.0}}}}),
    );
    assert_eq!(exactly_fan_in.grabbags.len(), 1);
}

#[test]
fn deferred_runtime_flags_follow_verified_cycles_in_both_kind_modes() {
    let mut pairs = consumers();
    pairs.extend([
        ("app/a.ts", "utils/date.ts"),
        ("utils/work.ts", "app/a.ts"),
        ("app/a.ts", "cycle/x.ts"),
        ("cycle/x.ts", "app/a.ts"),
    ]);
    let base = scan(&pairs, json!({}));
    let mut edges = base.edges.clone();
    edges
        .iter_mut()
        .find(|e| e.from == "app/a.ts" && e.to == "cycle/x.ts")
        .unwrap()
        .kind = EdgeKind::Dynamic;
    for all_kinds in [false, true] {
        let mut config = base.config.clone();
        config.all_kinds = all_kinds;
        let doc = boundrail::analyze(
            base.source.clone(),
            config,
            base.files.iter().map(|f| f.path.clone()).collect(),
            edges.clone(),
            vec![],
            None,
        )
        .unwrap();
        assert!(!doc.grabbags.is_empty());
        assert!(!doc.type_only_cycles.is_empty());
        assert!(
            doc.type_only_cycles
                .iter()
                .all(|c| !c.contains_runtime_cycle)
        );
        assert_eq!(!doc.cycles.is_empty(), all_kinds);
    }
}

#[test]
fn retained_all_kind_scc_still_gets_strict_runtime_flag_verification() {
    let mut pairs = consumers();
    pairs.extend([
        ("app/a.ts", "utils/date.ts"),
        ("utils/work.ts", "app/a.ts"),
        ("app/a.ts", "cycle/x.ts"),
        ("cycle/x.ts", "app/a.ts"),
        ("utils/date.ts", "utils/work.ts"),
    ]);
    let base = scan(
        &pairs,
        json!({"rules":{"path":"layout.yaml","sha256":"test","doc":{"version":1,"grabbag":{"internal_edges_per_file_max":0.6}}}}),
    );
    let mut edges = base.edges.clone();
    edges
        .iter_mut()
        .find(|e| e.from == "app/a.ts" && e.to == "cycle/x.ts")
        .unwrap()
        .kind = EdgeKind::Dynamic;
    edges
        .iter_mut()
        .find(|e| e.from == "utils/date.ts" && e.to == "utils/work.ts")
        .unwrap()
        .kind = EdgeKind::Type;
    for all_kinds in [false, true] {
        let mut config = base.config.clone();
        config.all_kinds = all_kinds;
        let doc = boundrail::analyze(
            base.source.clone(),
            config,
            base.files.iter().map(|f| f.path.clone()).collect(),
            edges.clone(),
            vec![],
            None,
        )
        .unwrap();
        assert!(!doc.type_only_cycles.is_empty());
        assert!(
            doc.type_only_cycles
                .iter()
                .all(|c| !c.contains_runtime_cycle)
        );
        assert_eq!(doc.grabbags.is_empty(), all_kinds);
        assert_eq!(!doc.cycles.is_empty(), all_kinds);
    }
}

#[test]
fn python_and_coarsely_grouped_go_files_use_the_same_verification() {
    for (language, extension) in [(Language::Python, "py"), (Language::Go, "go")] {
        let mut owned: Vec<(String, String)> = (1..=5)
            .map(|n| {
                (
                    format!("c{n}/x.{extension}"),
                    format!("utils/date/date.{extension}"),
                )
            })
            .collect();
        owned.extend([
            (
                format!("app/x.{extension}"),
                format!("utils/date/date.{extension}"),
            ),
            (
                format!("utils/work/work.{extension}"),
                format!("app/x.{extension}"),
            ),
        ]);
        let pairs: Vec<_> = owned
            .iter()
            .map(|(from, to)| (from.as_str(), to.as_str()))
            .collect();
        let doc = scan_language(&pairs, json!({}), language);
        assert!(doc.cycles.is_empty(), "{language:?}");
        assert_eq!(doc.grabbags.len(), 1, "{language:?}");
        assert!(doc.grabbags[0].remaining_cycles.is_empty());
    }
}
