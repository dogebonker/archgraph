use archgraph::{
    model::{Config, Edge, EdgeKind, Move, NodeMetadata, Source, Verdict},
    simulate,
};
use serde_json::json;

fn document(include_entry: bool) -> archgraph::model::Document {
    let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"dependency-cruiser","version":null,"cruised":4},"git":null,"generated_at":"test"})).unwrap();
    let mut config: Config = serde_json::from_value(json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"0".repeat(64),"doc":{"version":1,"entrypoints":{"api/**":"api/index.ts"}}}})).unwrap();
    config.file_metadata.insert(
        "api/index.ts".into(),
        NodeMetadata {
            barrel: true,
            ..Default::default()
        },
    );
    let mut files = vec![
        "api/impl.ts".into(),
        "app/via.ts".into(),
        "app/deep.ts".into(),
    ];
    let mut edges = vec![
        Edge {
            from: "app/via.ts".into(),
            to: "api/impl.ts".into(),
            original_to: Some("api/index.ts".into()),
            ..Default::default()
        },
        Edge {
            from: "app/deep.ts".into(),
            to: "api/impl.ts".into(),
            ..Default::default()
        },
    ];
    if include_entry {
        files.push("api/index.ts".into());
        edges.push(Edge {
            from: "api/index.ts".into(),
            to: "api/impl.ts".into(),
            kind: EdgeKind::Reexport,
            is_reexport: true,
            ..Default::default()
        });
    }
    archgraph::analyze(source, config, files, edges, vec![], None).unwrap()
}

#[test]
fn written_boundaries_survive_resolution_restore_and_entry_moves() {
    let doc = document(true);
    let policy = |doc: &archgraph::model::Document, from: &str| {
        doc.edges.iter().find(|e| e.from == from).unwrap().verdict
    };
    assert_eq!(policy(&doc, "app/via.ts"), Some(Verdict::Allowed));
    assert_eq!(policy(&doc, "app/deep.ts"), Some(Verdict::Forbidden));
    let input = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    let restored = archgraph::restore_scan(input).unwrap();
    assert!(
        restored
            .files
            .iter()
            .find(|f| f.path == "api/index.ts")
            .unwrap()
            .barrel
    );
    assert_eq!(policy(&restored, "app/via.ts"), Some(Verdict::Allowed));
    let moved = simulate::apply(
        &restored,
        archgraph::rules::compile(&restored.config)
            .unwrap()
            .as_ref(),
        &[Move {
            file: "api/index.ts".into(),
            to: "api/public.ts".into(),
        }],
    )
    .unwrap();
    assert!(
        moved
            .files
            .iter()
            .find(|f| f.path == "api/public.ts")
            .unwrap()
            .barrel
    );
    assert_eq!(policy(&moved, "app/via.ts"), Some(Verdict::Forbidden));
    let surface = archgraph::surface::analyze(&doc, 50).unwrap();
    assert!(
        surface
            .modules
            .iter()
            .flat_map(|m| &m.deep_imports)
            .all(|e| e.from != "app/via.ts")
    );
}

#[test]
fn excluded_facade_is_still_a_valid_written_policy_target() {
    let doc = document(false);
    let input = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    let restored = archgraph::restore_scan(input).unwrap();
    assert_eq!(
        restored
            .edges
            .iter()
            .find(|e| e.from == "app/via.ts")
            .unwrap()
            .verdict,
        Some(Verdict::Allowed)
    );
}

#[test]
fn boundary_finding_names_the_written_facade_when_definition_is_elsewhere() {
    let mut doc = document(true);
    doc.files.push(archgraph::model::File {
        path: "api/extra.ts".into(),
        owners: vec![],
        module: "api".into(),
        barrel: true,
        class: None,
    });
    doc.files.push(archgraph::model::File {
        path: "leaf/impl.ts".into(),
        owners: vec![],
        module: "leaf".into(),
        barrel: false,
        class: None,
    });
    let edge = doc
        .edges
        .iter_mut()
        .find(|e| e.from == "app/via.ts")
        .unwrap();
    edge.original_to = Some("api/extra.ts".into());
    edge.to = "leaf/impl.ts".into();
    let input = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    let restored = archgraph::restore_scan(input).unwrap();
    let finding = restored
        .findings
        .iter()
        .chain(restored.diagnostics.iter().map(|d| &d.finding))
        .find(|f| f.kind == archgraph::model::FindingKind::InternalsReach)
        .unwrap();
    assert_eq!(finding.subject, "api/**");
    if let archgraph::model::Evidence::InternalsReach {
        module, entries, ..
    } = &finding.evidence
    {
        assert_eq!(module, "api");
        assert_eq!(entries, &["api/index.ts"]);
    } else {
        panic!("expected boundary evidence");
    }
}

#[test]
fn reexport_loops_are_findings_and_survive_restore() {
    let mut doc = document(true);
    doc.edges.extend([
        Edge {
            from: "api/index.ts".into(),
            to: "api/impl.ts".into(),
            kind: EdgeKind::Reexport,
            is_reexport: true,
            ..Default::default()
        },
        Edge {
            from: "api/impl.ts".into(),
            to: "api/index.ts".into(),
            kind: EdgeKind::Reexport,
            is_reexport: true,
            ..Default::default()
        },
    ]);
    let input = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    let restored = archgraph::restore_scan(input).unwrap();
    assert!(
        restored
            .findings
            .iter()
            .any(|f| f.kind == archgraph::model::FindingKind::ReexportCycle)
    );
    assert!(restored.cycles.is_empty());
}
