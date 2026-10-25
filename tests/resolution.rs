use boundrail::model::{Config, ImportReason, ImportRecord, Source};
use serde_json::json;
#[test]
fn ledger_counts_imports_not_edges_and_external_nodes_do_not_change_metrics() {
    let source:Source=serde_json::from_value(json!({"language":"typescript","collector":{"name":"test","version":null,"cruised":1},"git":null,"generated_at":"test"})).unwrap();
    let mut config: Config = serde_json::from_value(
        json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":null}),
    )
    .unwrap();
    config.import_records = Some(vec![
        ImportRecord {
            from: "app/a.ts".into(),
            specifier: "./excluded.ts".into(),
            targets: vec!["app/excluded.ts".into()],
            ..Default::default()
        },
        ImportRecord {
            from: "app/a.ts".into(),
            specifier: "@lib/Thing".into(),
            external: Some("@lib/Thing".into()),
            ..Default::default()
        },
        ImportRecord {
            from: "app/a.ts".into(),
            specifier: "alias/missing".into(),
            unresolved: Some(ImportReason::Alias),
            ..Default::default()
        },
    ]);
    let doc = boundrail::analyze(
        source,
        config,
        vec!["app/a.ts".into()],
        vec![],
        vec![],
        None,
    )
    .unwrap();
    let summary = doc.import_resolution.as_ref().unwrap();
    assert_eq!(
        (summary.imports, summary.resolved, summary.unresolved),
        (3, 2, 1)
    );
    assert_eq!(summary.resolved_percent, Some(66.7));
    assert_eq!(doc.edges.len(), 0);
    assert_eq!(
        (
            doc.modules[0].ca,
            doc.modules[0].ce,
            doc.modules[0].cohesion
        ),
        (0, 0, None)
    );
    assert_eq!(doc.external_nodes[0].package, "@lib/Thing");
    assert!(doc.external_nodes[0].id.ends_with("@lib/Thing"));
    assert_eq!(doc.external_edges.len(), 1);
    assert_eq!(doc.external_edges[0].weight, Some(1.0));
    let moved = boundrail::simulate::apply(
        &doc,
        None,
        &[boundrail::model::Move {
            file: "app/a.ts".into(),
            to: "app/b.ts".into(),
        }],
    )
    .unwrap();
    assert_eq!(moved.external_edges[0].from, "app/b.ts");
    assert_eq!(moved.unresolved[0].from, "app/b.ts");
    let saved = serde_json::to_value(&doc).unwrap();
    let restored = boundrail::restore_scan(serde_json::from_value(saved.clone()).unwrap()).unwrap();
    assert_eq!(restored.import_resolution.unwrap().resolved, 2);
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("import-accounting.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args(["unresolved", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["reason"], "alias");
    assert_eq!(rows[0]["specifier"], "alias/missing");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args(["unresolved", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .starts_with("\"app/a.ts\" \"alias/missing\" — alias\n"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let mut invalid = saved;
    invalid["imports"][0]["targets"] = json!(["../outside.ts"]);
    assert!(boundrail::restore_scan(serde_json::from_value(invalid).unwrap()).is_err());
}
