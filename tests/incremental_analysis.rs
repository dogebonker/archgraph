use std::collections::{BTreeMap, BTreeSet};

use boundrail::{
    graph,
    incremental::{Analysis, Options},
    model::{Edge, EdgeKind},
    modules, quality,
};

fn assignment() -> BTreeMap<String, String> {
    [
        ("a/one.ts", "a"),
        ("a/two.ts", "a"),
        ("b/one.ts", "b"),
        ("c/one.ts", "c"),
        ("island/one.ts", "island"),
        ("island/two.ts", "island"),
    ]
    .into_iter()
    .map(|(f, m)| (f.to_owned(), m.to_owned()))
    .collect()
}
fn edge(from: &str, to: &str) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        ..Edge::default()
    }
}
fn parity(cache: &Analysis, module_of: &BTreeMap<String, String>, edges: &[Edge], opts: &Options) {
    let mut sorted = edges.to_vec();
    sorted.sort_by(|a, b| {
        (&a.from, &a.to, &a.original_to, a.kind).cmp(&(&b.from, &b.to, &b.original_to, b.kind))
    });
    let edges = sorted.as_slice();
    let pairs = modules::weighted(edges, &opts.barrels);
    let mut metrics = graph::modules(module_of, pairs.keys());
    quality::metrics(&mut metrics, module_of, &pairs);
    let (cycles, deferred) = quality::cycles(
        module_of,
        edges
            .iter()
            .filter(|e| !opts.tests.contains(&e.from) && !opts.tests.contains(&e.to)),
        &opts.entries,
        &opts.barrels,
        opts.directory_units,
        opts.all_kinds,
    );
    assert_eq!(
        serde_json::to_value(cache.modules()).unwrap(),
        serde_json::to_value(metrics).unwrap()
    );
    assert_eq!(
        serde_json::to_value(cache.cycles()).unwrap(),
        serde_json::to_value(cycles).unwrap()
    );
    assert_eq!(
        serde_json::to_value(cache.deferred_cycles()).unwrap(),
        serde_json::to_value(deferred).unwrap()
    );
    for (from, targets) in cache.condensation().iter().enumerate() {
        assert!(!targets.contains(&from));
    }
}

#[test]
fn insert_remove_revert_boundary_and_isolated_region_match_full_analysis() {
    let mut module_of = assignment();
    let opts = Options::default();
    let mut edges = vec![
        edge("a/one.ts", "b/one.ts"),
        edge("b/one.ts", "c/one.ts"),
        edge("island/one.ts", "island/two.ts"),
    ];
    let mut cache = Analysis::new(module_of.clone(), edges.clone(), opts.clone()).unwrap();
    parity(&cache, &module_of, &edges, &opts);
    edges.push(edge("c/one.ts", "a/one.ts"));
    let work = cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    assert_eq!(work.recomputed_modules, 3);
    assert_eq!(work.total_modules, 4);
    assert!(work.fallback.is_none());
    assert_eq!(cache.cycles().len(), 1);
    assert_eq!(
        cache.components().iter().filter(|c| c.len() == 3).count(),
        1
    );
    parity(&cache, &module_of, &edges, &opts);
    edges.remove(1);
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    assert!(cache.cycles().is_empty());
    parity(&cache, &module_of, &edges, &opts);
    edges.insert(1, edge("b/one.ts", "c/one.ts"));
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    parity(&cache, &module_of, &edges, &opts);
    module_of.insert("c/one.ts".into(), "b".into());
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    parity(&cache, &module_of, &edges, &opts);
    edges.retain(|e| e.from != "c/one.ts");
    module_of.remove("c/one.ts");
    edges.retain(|e| e.to != "c/one.ts");
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    parity(&cache, &module_of, &edges, &opts);
    let unchanged = cache.update(module_of, edges, opts).unwrap();
    assert_eq!(unchanged.changed_files, 0);
    assert_eq!(unchanged.recomputed_modules, 0);
    assert!(unchanged.fallback.is_none());
}

#[test]
fn barrel_weights_type_cycles_tests_and_options_match_full_analysis() {
    let module_of = assignment();
    let mut opts = Options {
        barrels: BTreeSet::from(["a/two.ts".into()]),
        ..Options::default()
    };
    let mut reexport = edge("a/two.ts", "c/one.ts");
    reexport.kind = EdgeKind::Reexport;
    let mut erased = edge("c/one.ts", "b/one.ts");
    erased.kind = EdgeKind::Type;
    let mut edges = vec![edge("b/one.ts", "a/two.ts"), reexport, erased];
    let mut cache = Analysis::new(module_of.clone(), edges.clone(), opts.clone()).unwrap();
    parity(&cache, &module_of, &edges, &opts);
    edges[2].kind = EdgeKind::Static;
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    parity(&cache, &module_of, &edges, &opts);
    opts.all_kinds = true;
    let work = cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    assert!(work.fallback.unwrap().contains("options"));
    parity(&cache, &module_of, &edges, &opts);
    opts.tests.insert("c/one.ts".into());
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    parity(&cache, &module_of, &edges, &opts);
}

#[test]
fn unknown_endpoint_fails_without_mutating_cache() {
    let module_of = assignment();
    let mut cache = Analysis::new(module_of.clone(), Vec::new(), Options::default()).unwrap();
    let before = serde_json::to_value(cache.modules()).unwrap();
    assert!(
        cache
            .update(
                module_of,
                vec![edge("a/one.ts", "absent.ts")],
                Options::default()
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(cache.modules()).unwrap(), before);
}

#[test]
fn two_thousand_file_graph_only_recomputes_changed_island() {
    let module_of: BTreeMap<_, _> = (0..2000)
        .map(|i| (format!("m{i}/file.ts"), format!("m{i}")))
        .collect();
    let opts = Options::default();
    let mut cache = Analysis::new(module_of.clone(), Vec::new(), opts.clone()).unwrap();
    let edges = vec![
        edge("m0/file.ts", "m1/file.ts"),
        edge("m1/file.ts", "m0/file.ts"),
    ];
    let work = cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    assert_eq!(work.recomputed_modules, 2);
    assert_eq!(work.total_modules, 2000);
    assert!(work.fallback.is_none());
    parity(&cache, &module_of, &edges, &opts);
}

#[test]
fn deterministic_mixed_edge_updates_match_full_analysis() {
    let module_of = assignment();
    let names: Vec<_> = module_of.keys().cloned().collect();
    let opts = Options::default();
    let mut edges = Vec::new();
    let mut cache = Analysis::new(module_of.clone(), edges.clone(), opts.clone()).unwrap();
    let mut seed = 71u64;
    for turn in 0..100 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let from = (seed >> 32) as usize % names.len();
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let to = (seed >> 32) as usize % names.len();
        edges.retain(|e: &Edge| e.from != names[from] || e.to != names[to]);
        if turn % 3 != 0 {
            let mut e = edge(&names[from], &names[to]);
            if turn % 4 == 0 {
                e.kind = EdgeKind::Type;
            }
            edges.push(e);
        }
        cache
            .update(module_of.clone(), edges.clone(), opts.clone())
            .unwrap();
        parity(&cache, &module_of, &edges, &opts);
    }
}

#[test]
fn public_break_set_budget_stays_equal_when_another_region_grows() {
    let module_of: BTreeMap<_, _> = (0..8)
        .map(|i| (format!("m{i}/file.ts"), format!("m{i}")))
        .chain([
            (String::from("island/a.ts"), String::from("island")),
            (String::from("island/b.ts"), String::from("island")),
        ])
        .collect();
    let opts = Options {
        entries: BTreeSet::from(["m0/file.ts".into()]),
        ..Options::default()
    };
    let mut edges = Vec::new();
    for from in 0..8 {
        for step in 1..=3 {
            let mut e = edge(
                &format!("m{from}/file.ts"),
                &format!("m{}/file.ts", (from + step) % 8),
            );
            e.is_reexport = from == 0;
            edges.push(e);
        }
    }
    let mut cache = Analysis::new(module_of.clone(), edges.clone(), opts.clone()).unwrap();
    parity(&cache, &module_of, &edges, &opts);
    edges.push(edge("island/a.ts", "island/b.ts"));
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    parity(&cache, &module_of, &edges, &opts);
}

#[test]
fn file_cycles_and_directory_compilation_units_match_full_analysis() {
    let module_of = assignment();
    let mut opts = Options::default();
    let mut edges = vec![edge("a/one.ts", "a/two.ts")];
    let mut cache = Analysis::new(module_of.clone(), edges.clone(), opts.clone()).unwrap();
    edges.push(edge("a/two.ts", "a/one.ts"));
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    assert_eq!(cache.cycles().len(), 1);
    parity(&cache, &module_of, &edges, &opts);
    opts.directory_units = true;
    cache
        .update(module_of.clone(), edges.clone(), opts.clone())
        .unwrap();
    assert!(cache.cycles().is_empty());
    parity(&cache, &module_of, &edges, &opts);
}
