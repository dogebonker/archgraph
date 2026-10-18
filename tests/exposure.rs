use archgraph::{
    model::{Config, Document, Edge, EdgeKind, ImportReason, ImportRecord, Move, Source, Verdict},
    rules,
};
use serde_json::{Value, json};

fn configuration(layout: Value, imports: Option<Vec<ImportRecord>>) -> Config {
    let mut config: Config = serde_json::from_value(json!({"depth":2,"source_root":".","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"test","doc":layout}})).unwrap();
    config.import_records = imports;
    config
}
fn import(from: &str, package: &str, kind: EdgeKind) -> ImportRecord {
    ImportRecord {
        from: from.into(),
        specifier: package.into(),
        external: Some(package.into()),
        kind,
        line: Some(3),
        ..Default::default()
    }
}
fn scan(
    language: &str,
    layout: Value,
    records: Option<Vec<ImportRecord>>,
    edges: Vec<Edge>,
) -> anyhow::Result<Document> {
    let source: Source = serde_json::from_value(json!({"language":language,"collector":{"name":"fixture","version":null,"cruised":2},"git":null,"generated_at":"test"})).unwrap();
    archgraph::analyze(
        source,
        configuration(layout, records),
        vec!["src/domain/a.ts".into(), "src/adapter/b.ts".into()],
        edges,
        vec![],
        None,
    )
}

#[test]
fn selectors_match_exact_collector_packages_for_each_language() {
    for (language, package) in [
        ("typescript", "typeorm"),
        ("python", "sqlalchemy"),
        ("go", "gorm.io/gorm"),
    ] {
        let doc = scan(language, json!({"version":1,"pkg-category":{"orm":[package]},"forbid":[{"from":"src/domain/**","to":"category:orm"}]}), Some(vec![import("src/domain/a.ts",package,EdgeKind::Static)]), vec![]).unwrap();
        assert_eq!(
            doc.external_nodes[0].id,
            format!("external:{language}:{package}")
        );
        assert_eq!(doc.external_edges[0].verdict, Some(Verdict::Forbidden));
        assert_eq!(doc.summary.forbidden, 1);
        assert_eq!(doc.modules.iter().map(|m| m.ce).sum::<usize>(), 0);
        assert!(doc.cycles.is_empty());
    }
}

#[test]
fn policy_order_kind_filters_scoped_globs_and_internal_defaults() {
    let layout = json!({"version":1,"forbid":[{"from":"src/domain/**","to":"pkg:@scope/**","kinds":["runtime"]},{"from":"src/domain/**","to":"**"}],"allow":[{"from":"src/domain/**","to":"pkg:@scope/**"}],"public":[{"module":"**","entries":[]}],"layers":["src/**","**"],"default":"forbidden"});
    let doc = scan(
        "typescript",
        layout,
        Some(vec![
            import("src/domain/a.ts", "@scope/client", EdgeKind::Static),
            import("src/domain/a.ts", "@scope/client", EdgeKind::Type),
            import("src/domain/a.ts", "other", EdgeKind::Static),
        ]),
        vec![],
    )
    .unwrap();
    assert_eq!(
        doc.external_edges
            .iter()
            .map(|e| e.verdict)
            .collect::<Vec<_>>(),
        [
            Some(Verdict::Forbidden),
            Some(Verdict::Allowed),
            Some(Verdict::Forbidden)
        ]
    );
    assert_eq!(doc.external_edges[1].rule.as_deref(), Some("allow[0]"));
    let doc = scan(
        "typescript",
        json!({"version":1,"forbid":[{"from":"**","to":"**"}]}),
        Some(vec![import("src/domain/a.ts", "typeorm", EdgeKind::Static)]),
        vec![],
    )
    .unwrap();
    assert_eq!(doc.external_edges[0].verdict, Some(Verdict::Unplanned));
}

#[test]
fn reachable_package_policy_marks_internal_path_and_survives_move_and_diff() {
    let layout = json!({"version":1,"pkg-category":{"orm":["typeorm"]},"forbid":[{"from":"src/domain/**","to":"category:orm","reachable":true}]});
    let edge = Edge {
        from: "src/domain/a.ts".into(),
        to: "src/adapter/b.ts".into(),
        ..Default::default()
    };
    let doc = scan(
        "typescript",
        layout,
        Some(vec![import(
            "src/adapter/b.ts",
            "typeorm",
            EdgeKind::Static,
        )]),
        vec![edge],
    )
    .unwrap();
    assert_eq!(doc.edges[0].verdict, Some(Verdict::Forbidden));
    assert_eq!(
        doc.edges[0].reach.as_ref().unwrap(),
        &["src/adapter/b.ts", "external:typescript:typeorm"]
    );
    let compiled = rules::compile(&doc.config).unwrap();
    let moved = archgraph::simulate::apply(
        &doc,
        compiled.as_ref(),
        &[Move {
            file: "src/adapter/b.ts".into(),
            to: "src/domain/b.ts".into(),
        }],
    )
    .unwrap();
    assert_eq!(moved.external_edges[0].verdict, Some(Verdict::Forbidden));
    assert_eq!(moved.summary.forbidden, 2);
    assert_eq!(archgraph::diff::diff(&doc, &moved).new_forbidden.len(), 2);
    let restored = archgraph::restore_scan(
        serde_json::from_value(serde_json::to_value(&moved).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(restored.summary.forbidden, 2);
    assert_eq!(
        serde_json::to_value(&restored.external_exposure).unwrap(),
        serde_json::to_value(&moved.external_exposure).unwrap()
    );
}

#[test]
fn exposure_counts_occurrences_distinct_packages_weights_and_unresolved() {
    let mut records = vec![
        import("src/domain/a.ts", "typeorm", EdgeKind::Static),
        import("src/domain/a.ts", "typeorm", EdgeKind::Type),
    ];
    records.push(ImportRecord {
        from: "src/domain/a.ts".into(),
        specifier: "missing".into(),
        unresolved: Some(ImportReason::Alias),
        ..Default::default()
    });
    records.push(ImportRecord {
        from: "src/domain/a.ts".into(),
        specifier: "./local".into(),
        targets: vec!["src/adapter/b.ts".into()],
        ..Default::default()
    });
    let doc = scan(
        "typescript",
        json!({"version":1,"external_exposure":{"max_percent":50}}),
        Some(records),
        vec![],
    )
    .unwrap();
    let report = doc.external_exposure.as_ref().unwrap();
    let row = report
        .modules
        .iter()
        .find(|m| m.module == "src/domain")
        .unwrap();
    assert_eq!(
        (
            row.imports,
            row.external_imports,
            row.external_packages,
            row.unresolved_imports
        ),
        (4, 2, 1, 1)
    );
    assert_eq!(row.external_percent, Some(50.0));
    assert_eq!(row.weighted_external_percent, Some(35.4839));
    assert!(!row.exceeds_threshold);
    assert!((row.external_weight - 1.1).abs() < 1e-10);
    assert_eq!(row.packages[0].imports, 2);
    assert_eq!(doc.external_edges.len(), 2);
    let empty = report
        .modules
        .iter()
        .find(|m| m.module == "src/adapter")
        .unwrap();
    assert_eq!(empty.external_percent, None);
    assert_eq!(empty.weighted_external_percent, None);
}

#[test]
fn unavailable_ledger_is_not_false_zero_or_false_policy_pass() {
    let doc = scan("typescript", json!({"version":1}), None, vec![]).unwrap();
    assert!(doc.external_exposure.is_none());
    assert!(
        scan(
            "typescript",
            json!({"version":1,"forbid":[{"from":"**","to":"pkg:typeorm"}]}),
            None,
            vec![]
        )
        .unwrap_err()
        .to_string()
        .contains("import accounting")
    );
    let mut saved = serde_json::to_value(&doc).unwrap();
    saved["config"]["rules"]["doc"]["allow"] = json!([{"from":"**","to":"pkg:*"}]);
    assert!(archgraph::restore_scan(serde_json::from_value(saved).unwrap()).is_err());
}

#[test]
fn category_and_threshold_validation_and_capture_binding() {
    for layout in [
        json!({"version":1,"forbid":[{"from":"**","to":"category:nope"}]}),
        json!({"version":1,"pkg-category":{"empty":[]}}),
        json!({"version":1,"external_exposure":{"max_percent":101}}),
        json!({"version":1,"forbid":[{"from":"src/<x>/**","to":"category:<x>"}]}),
        json!({"version":1,"forbid":[{"from":"**","to":"pkg:"}]}),
    ] {
        assert!(rules::Compiled::new(&serde_json::from_value(layout).unwrap()).is_err());
    }
    let doc = scan(
        "typescript",
        json!({"version":1,"forbid":[{"from":"src/<area>/**","to":"pkg:@scope/<area>"}]}),
        Some(vec![import(
            "src/domain/a.ts",
            "@scope/domain",
            EdgeKind::Static,
        )]),
        vec![],
    )
    .unwrap();
    assert_eq!(doc.external_edges[0].verdict, Some(Verdict::Forbidden));
    let doc = scan(
        "typescript",
        json!({"version":1}),
        Some(vec![
            import("src/domain/a.ts", "typeorm", EdgeKind::Static),
            import("src/domain/a.ts", "typeorm", EdgeKind::Static),
        ]),
        vec![],
    )
    .unwrap();
    assert_eq!(doc.external_edges.len(), 1);
    assert!(
        doc.external_exposure
            .unwrap()
            .modules
            .iter()
            .any(|row| row.exceeds_threshold && row.external_imports == 2)
    );
}

#[test]
fn rejects_external_aggregate_weight_overflow() {
    let layout = json!({"version":1,"edge_weights":{"runtime":1e308}});
    let records = Some(vec![
        import("src/domain/a.ts", "first", EdgeKind::Static),
        import("src/domain/a.ts", "second", EdgeKind::Static),
    ]);
    let error = scan("typescript", layout, records, vec![]).unwrap_err();
    assert!(error.to_string().contains("smaller finite weights"));
}

#[test]
fn yaml_loader_accepts_categories_and_rejects_unknown_threshold_keys() {
    let path = std::env::temp_dir().join(format!(
        "archgraph-exposure-layout-{}.yaml",
        std::process::id()
    ));
    std::fs::write(&path, "version: 1\npkg-category:\n  persistence: [typeorm, 'gorm.io/gorm{,/**}']\nexternal_exposure:\n  max_percent: 70\nforbid:\n  - {from: 'src/domain/**', to: 'category:persistence'}\n").unwrap();
    let loaded = rules::load(&path).unwrap();
    assert_eq!(loaded.doc.pkg_category["persistence"].len(), 2);
    assert_eq!(loaded.doc.external_exposure.unwrap().max_percent, 70.0);
    std::fs::write(&path, "version: 1\nexternal_exposure: {max_percnt: 70}\n").unwrap();
    assert!(
        rules::load(&path)
            .unwrap_err()
            .to_string()
            .contains("parsing")
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn absent_policy_has_no_verdict_and_package_like_file_names_are_not_external() {
    let mut config = configuration(
        json!({"version":1}),
        Some(vec![import("src/domain/a.ts", "typeorm", EdgeKind::Static)]),
    );
    config.rules = None;
    let source:Source=serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","version":null,"cruised":2},"git":null,"generated_at":"test"})).unwrap();
    let doc = archgraph::analyze(
        source,
        config,
        vec!["src/domain/a.ts".into()],
        vec![],
        vec![],
        None,
    )
    .unwrap();
    assert_eq!(doc.external_edges[0].verdict, None);
    assert_eq!(doc.summary.unplanned, 0);
    let compiled = rules::Compiled::new(
        &serde_json::from_value(json!({"version":1,"forbid":[{"from":"**","to":"pkg:typeorm"}]}))
            .unwrap(),
    )
    .unwrap();
    let mut edges = vec![Edge {
        from: "src/domain/a.ts".into(),
        to: "pkg:typeorm".into(),
        ..Default::default()
    }];
    compiled.decide(
        &mut edges,
        &std::collections::BTreeMap::from([
            ("src/domain/a.ts".into(), "domain".into()),
            ("pkg:typeorm".into(), "elsewhere".into()),
        ]),
    );
    assert_eq!(edges[0].verdict, Some(Verdict::Unplanned));
}

#[test]
fn simulation_lists_added_and_removed_external_violations_by_kind() {
    let doc=scan("typescript",json!({"version":1,"forbid":[{"from":"src/domain/**","to":"pkg:typeorm","kinds":["runtime"]},{"from":"**","to":"pkg:typeorm","kinds":["type"]}]}),Some(vec![import("src/adapter/b.ts","typeorm",EdgeKind::Static),import("src/adapter/b.ts","typeorm",EdgeKind::Type)]),vec![]).unwrap();
    let compiled = rules::compile(&doc.config).unwrap();
    let result = archgraph::simulate::simulate(
        &doc,
        compiled.as_ref(),
        "src/adapter/b.ts",
        "src/domain/b.ts",
    )
    .unwrap();
    assert_eq!(result.new_forbidden.len(), 1);
    assert_eq!(result.new_forbidden[0].kind, EdgeKind::Static);
    assert!(result.removed_forbidden.is_empty());
    let moved = archgraph::simulate::apply(
        &doc,
        compiled.as_ref(),
        &[Move {
            file: "src/adapter/b.ts".into(),
            to: "src/domain/b.ts".into(),
        }],
    )
    .unwrap();
    let back = archgraph::simulate::simulate(
        &moved,
        compiled.as_ref(),
        "src/domain/b.ts",
        "src/adapter/b.ts",
    )
    .unwrap();
    assert!(back.new_forbidden.is_empty());
    assert_eq!(back.removed_forbidden.len(), 1);
    assert_eq!(back.removed_forbidden[0].kind, EdgeKind::Static);
    let mut changed = doc.config.clone();
    changed.rules.as_mut().unwrap().doc.forbid.push(
        serde_json::from_value(json!({"from":"**","to":"pkg:typeorm","kinds":["runtime"]}))
            .unwrap(),
    );
    let updated = archgraph::analyze(
        doc.source.clone(),
        changed,
        doc.files.iter().map(|f| f.path.clone()).collect(),
        vec![],
        vec![],
        None,
    )
    .unwrap();
    let diff = archgraph::diff::diff(&doc, &updated);
    assert_eq!(diff.new_forbidden.len(), 1);
    assert_eq!(diff.new_forbidden[0].kind, EdgeKind::Static);
}

#[test]
fn external_ids_cannot_spoof_ordinary_file_policy_targets_or_move_destinations() {
    let doc = scan(
        "typescript",
        json!({"version":1,"forbid":[{"from":"src/domain/**","to":"external:typescript:typeorm"}]}),
        Some(vec![import(
            "src/adapter/b.ts",
            "typeorm",
            EdgeKind::Static,
        )]),
        vec![],
    )
    .unwrap();
    let mut paths: Vec<_> = doc.files.iter().map(|f| f.path.clone()).collect();
    paths.push("external:typescript:typeorm".into());
    assert!(
        archgraph::analyze(
            doc.source.clone(),
            doc.config.clone(),
            paths,
            vec![],
            vec![],
            None
        )
        .unwrap_err()
        .to_string()
        .contains("collides")
    );
    let mut saved = serde_json::to_value(&doc).unwrap();
    saved["files"].as_array_mut().unwrap().push(json!({"path":"external:typescript:typeorm","module":"external:typescript:typeorm","barrel":false}));
    assert!(
        archgraph::validate_scan(&serde_json::from_value(saved).unwrap())
            .unwrap_err()
            .to_string()
            .contains("collides")
    );
    let compiled = rules::compile(&doc.config).unwrap();
    assert!(
        archgraph::simulate::apply(
            &doc,
            compiled.as_ref(),
            &[Move {
                file: "src/domain/a.ts".into(),
                to: "external:typescript:typeorm".into()
            }]
        )
        .unwrap_err()
        .to_string()
        .contains("collides")
    );
    let moved = archgraph::simulate::apply(
        &doc,
        compiled.as_ref(),
        &[Move {
            file: "src/domain/a.ts".into(),
            to: "other:ordinary.ts".into(),
        }],
    )
    .unwrap();
    assert!(
        moved
            .files
            .iter()
            .any(|file| file.path == "other:ordinary.ts")
    );
}
