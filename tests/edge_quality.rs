use std::collections::{BTreeMap, BTreeSet};

use boundrail::{
    graph,
    model::{Edge, EdgeKind},
    quality,
};

fn edge(from: &str, to: &str, kind: EdgeKind, weight: f64) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind,
        weight: Some(weight),
        is_reexport: kind == EdgeKind::Reexport,
        ..Edge::default()
    }
}

fn mapping(files: &[(&str, &str)]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(file, module)| ((*file).into(), (*module).into()))
        .collect()
}

#[test]
fn type_only_cycles_are_accounted_without_runtime_findings() {
    let modules = mapping(&[("a.ts", "a"), ("b.ts", "b")]);
    let edges = [
        edge("a.ts", "b.ts", EdgeKind::Type, 0.1),
        edge("b.ts", "a.ts", EdgeKind::Type, 0.1),
    ];
    let (cycles, deferred) = quality::cycles(
        &modules,
        &edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
        false,
    );
    assert!(cycles.is_empty());
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].kinds, ["type"]);
    assert!(!deferred[0].contains_runtime_cycle);
    let (cycles, _) = quality::cycles(
        &modules,
        &edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
        true,
    );
    assert_eq!(cycles.len(), 1);
}

#[test]
fn a_lazy_back_edge_does_not_close_a_runtime_cycle() {
    let modules = mapping(&[("a", "a"), ("b", "b")]);
    let edges = [
        edge("a", "b", EdgeKind::Static, 1.0),
        edge("b", "a", EdgeKind::Dynamic, 0.3),
    ];
    let (cycles, deferred) = quality::cycles(
        &modules,
        &edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
        false,
    );
    assert!(cycles.is_empty());
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].kinds, ["lazy", "runtime"]);
    assert!(!deferred[0].contains_runtime_cycle);
}

#[test]
fn mixed_component_preserves_runtime_cycle_and_records_larger_deferred_component() {
    let modules = mapping(&[("a", "a"), ("b", "b"), ("c", "c")]);
    let edges = [
        edge("a", "b", EdgeKind::Static, 1.0),
        edge("b", "a", EdgeKind::Static, 1.0),
        edge("b", "c", EdgeKind::Dynamic, 0.3),
        edge("c", "a", EdgeKind::Static, 1.0),
    ];
    let (cycles, deferred) = quality::cycles(
        &modules,
        &edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
        false,
    );
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].members, ["a", "b"]);
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].members, ["a", "b", "c"]);
    assert_eq!(deferred[0].kinds, ["lazy", "runtime"]);
    assert!(deferred[0].contains_runtime_cycle);
}

#[test]
fn internal_file_deferred_cycles_are_separate_and_go_directory_units_are_preserved() {
    let modules = mapping(&[("pkg/a", "pkg"), ("pkg/b", "pkg")]);
    let edges = [
        edge("pkg/a", "pkg/b", EdgeKind::Type, 0.1),
        edge("pkg/b", "pkg/a", EdgeKind::Dynamic, 0.3),
    ];
    let (_, deferred) = quality::cycles(
        &modules,
        &edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
        false,
    );
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].level, boundrail::model::CycleLevel::File);
    let (_, deferred) = quality::cycles(
        &modules,
        &edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        true,
        false,
    );
    assert!(deferred.is_empty());
}

#[test]
fn weighted_metrics_use_strongest_pair_and_neighbour_and_preserve_counts() {
    let modules = mapping(&[
        ("a/one", "a"),
        ("a/two", "a"),
        ("b/one", "b"),
        ("b/two", "b"),
        ("c/one", "c"),
    ]);
    let edges = [
        edge("a/one", "b/one", EdgeKind::Type, 0.1),
        edge("a/one", "b/one", EdgeKind::Dynamic, 0.3),
        edge("a/two", "b/two", EdgeKind::Static, 2.0),
        edge("b/one", "c/one", EdgeKind::Dynamic, 0.3),
        edge("a/one", "a/two", EdgeKind::Type, 0.1),
    ];
    let pairs = boundrail::modules::weighted(&edges, &BTreeSet::new());
    let mut report = graph::modules(&modules, pairs.keys());
    quality::metrics(&mut report, &modules, &pairs);
    let a = &report[0];
    assert_eq!(
        (a.ca, a.ce, a.internal_edges, a.outgoing_edges),
        (0, 1, 1, 2)
    );
    assert_eq!(a.weighted_ce, 2.0);
    assert_eq!(a.weighted_outgoing_edges, 2.3);
    assert_eq!(a.weighted_internal_edges, 0.1);
    assert_eq!(a.cohesion, Some(0.0417));
    assert_eq!(report[1].weighted_ca, 2.0);
    assert_eq!(report[1].weighted_ce, 0.3);
    assert_eq!(report[1].instability, Some(0.1304));
}

#[test]
fn barrel_wiring_inherits_consumers_weight_through_chains() {
    let modules = mapping(&[
        ("client/a", "client"),
        ("api/index", "api"),
        ("api/nested", "api"),
        ("api/real", "api"),
    ]);
    let edges = [
        edge("client/a", "api/index", EdgeKind::Dynamic, 0.3),
        edge("api/index", "api/nested", EdgeKind::Reexport, 0.0),
        edge("api/nested", "api/real", EdgeKind::Reexport, 0.0),
    ];
    let barrels = BTreeSet::from(["api/index".into(), "api/nested".into()]);
    let pairs = boundrail::modules::weighted(&edges, &barrels);
    let mut report = graph::modules(&modules, pairs.keys());
    quality::metrics(&mut report, &modules, &pairs);
    assert_eq!(
        report
            .iter()
            .find(|m| m.id == "client")
            .unwrap()
            .weighted_ce,
        0.3
    );
    let api = report.iter().find(|m| m.id == "api").unwrap();
    assert_eq!(api.weighted_internal_edges, 0.0);
    assert_eq!(api.weighted_ca, 0.3);
}
