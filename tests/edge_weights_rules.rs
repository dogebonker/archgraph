use std::collections::BTreeMap;

use archgraph::model::{Edge, EdgeKind, Verdict};
use archgraph::rules::{Compiled, EdgeWeights, Rules};

fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind,
        is_reexport: kind == EdgeKind::Reexport,
        ..Edge::default()
    }
}

fn decide(yaml: &str, edges: &mut [Edge]) {
    let rules: Rules = serde_yaml_ng::from_str(yaml).unwrap();
    let modules = edges
        .iter()
        .flat_map(|e| [&e.from, &e.to])
        .map(|path| (path.clone(), path.clone()))
        .collect::<BTreeMap<_, _>>();
    Compiled::new(&rules).unwrap().decide(edges, &modules);
}

#[test]
fn weights_defaults_overrides_and_validation() {
    let weights = EdgeWeights::default();
    assert_eq!(weights.weight(EdgeKind::Static), 1.0);
    assert_eq!(weights.weight(EdgeKind::Dynamic), 0.3);
    assert_eq!(weights.weight(EdgeKind::Type), 0.1);
    assert_eq!(weights.weight(EdgeKind::TypeReexport), 0.1);
    assert_eq!(weights.weight(EdgeKind::Reexport), 0.0);
    let rules: Rules = serde_yaml_ng::from_str("version: 1\nedge_weights: { lazy: 2.0 }").unwrap();
    assert_eq!(rules.edge_weights.unwrap().lazy, 2.0);
    assert_eq!(rules.edge_weights.unwrap().runtime, 1.0);
    Compiled::new(&rules).unwrap();
    for value in [-0.1, f64::INFINITY, f64::NAN] {
        let rules = Rules {
            version: 1,
            edge_weights: Some(EdgeWeights {
                runtime: value,
                ..weights
            }),
            ..Rules::default()
        };
        assert!(Compiled::new(&rules).is_err());
    }
}

#[test]
fn forbids_apply_to_all_kinds_unless_filtered() {
    let mut edges = [
        edge("a", "b", EdgeKind::Type),
        edge("a", "c", EdgeKind::Static),
    ];
    decide("version: 1\nforbid: [{from: a, to: '*'}]", &mut edges);
    assert!(edges.iter().all(|e| e.verdict == Some(Verdict::Forbidden)));
    decide(
        "version: 1\nforbid: [{from: a, to: '*', kinds: [runtime]}]",
        &mut edges,
    );
    assert_eq!(edges[0].verdict, Some(Verdict::Unplanned));
    assert_eq!(edges[1].verdict, Some(Verdict::Forbidden));
}

#[test]
fn reachable_forbids_filter_the_entire_path() {
    let mut edges = [
        edge("a", "b", EdgeKind::Static),
        edge("b", "c", EdgeKind::Type),
        edge("c", "d", EdgeKind::Static),
    ];
    decide(
        "version: 1\nforbid: [{from: a, to: d, reachable: true, kinds: [runtime]}]",
        &mut edges,
    );
    assert_eq!(edges[0].verdict, Some(Verdict::Unplanned));
    decide(
        "version: 1\nforbid: [{from: a, to: d, reachable: true}]",
        &mut edges,
    );
    assert_eq!(edges[0].verdict, Some(Verdict::Forbidden));
    assert_eq!(
        edges[0].reach,
        Some(vec!["b".into(), "c".into(), "d".into()])
    );
}

#[test]
fn allow_and_public_filters_do_not_affect_other_kinds() {
    let mut edges = [
        edge("client", "api/private", EdgeKind::Type),
        edge("client", "api/private", EdgeKind::Static),
    ];
    decide(
        "version: 1\npublic: [{module: 'api/**', entries: ['api/index'], kinds: [runtime]}]\nallow: [{from: client, to: 'api/**', kinds: [type]}]",
        &mut edges,
    );
    assert_eq!(edges[0].verdict, Some(Verdict::Allowed));
    assert_eq!(edges[1].verdict, Some(Verdict::Forbidden));
    let mut edges = [edge("client", "api", EdgeKind::TypeReexport)];
    decide(
        "version: 1\nforbid: [{from: client, to: api, kinds: [type]}]",
        &mut edges,
    );
    assert_eq!(edges[0].verdict, Some(Verdict::Forbidden));
}

#[test]
fn layout_loader_accepts_new_keys_and_rejects_nested_typos() {
    let path = std::env::temp_dir().join(format!(
        "archgraph-edge-weights-{}.yaml",
        std::process::id()
    ));
    std::fs::write(&path, "version: 1\nedge_weights: {runtime: 1}\npublic: [{module: 'api/**', entries: [api/index], kinds: [type]}]\nforbid: [{from: a, to: b, kinds: [type]}]\nallow: [{from: b, to: a, kinds: [type]}]").unwrap();
    archgraph::rules::load(&path).unwrap();
    std::fs::write(&path, "version: 1\nedge_weights: {runtim: 1}").unwrap();
    let error = archgraph::rules::load(&path).unwrap_err();
    assert!(format!("{error:#}").contains("unknown field `runtim` in edge_weights"));
    std::fs::remove_file(path).unwrap();
}
