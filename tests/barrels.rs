use std::collections::BTreeMap;

use archgraph::collect::RawGraph;
use archgraph::collect::barrels::{BarrelModule, ExportRoute, ImportRoute, resolve, typescript};
use archgraph::collect::imports::{Sources, analyze};
use archgraph::model::{Confidence, Edge, EdgeKind};

fn module(
    file: &str,
    pure: bool,
    declared: &[&str],
    exports: &[(&str, &str, &str)],
) -> BarrelModule {
    BarrelModule {
        file: file.into(),
        pure,
        declared: declared.iter().map(|s| (*s).into()).collect(),
        exports: exports
            .iter()
            .map(|(exported, imported, to)| ExportRoute {
                exported: (*exported).into(),
                imported: (*imported).into(),
                to: (*to).into(),
            })
            .collect(),
        imports: exports
            .iter()
            .map(|(_, imported, to)| {
                route(
                    to,
                    &[*imported],
                    *imported == "*",
                    EdgeKind::Reexport,
                    false,
                )
            })
            .collect(),
    }
}
fn route(
    to: &str,
    names: &[&str],
    wildcard: bool,
    kind: EdgeKind,
    side_effect: bool,
) -> ImportRoute {
    ImportRoute {
        to: to.into(),
        names: names.iter().map(|s| (*s).into()).collect(),
        wildcard,
        kind,
        specifier: Some(format!("./{to}")),
        line: 2,
        side_effect,
    }
}
fn graph(modules: &[BarrelModule]) -> RawGraph {
    RawGraph {
        imports: None,
        files: modules.iter().map(|m| m.file.clone()).collect(),
        edges: modules
            .iter()
            .flat_map(|m| {
                m.imports.iter().map(|i| Edge {
                    from: m.file.clone(),
                    to: i.to.clone(),
                    kind: i.kind,
                    ..Edge::default()
                })
            })
            .collect(),
        collector: serde_json::from_value(
            serde_json::json!({"name":"test", "version":null, "cruised":modules.len()}),
        )
        .unwrap(),
        metadata: BTreeMap::new(),
    }
}

#[test]
fn named_and_default_aliases_resolve_through_multiple_barrels() {
    let mut user = module("user.ts", false, &[], &[]);
    user.imports = vec![route(
        "entry.ts",
        &["publicName", "default"],
        false,
        EdgeKind::Static,
        false,
    )];
    let modules = vec![
        user,
        module(
            "entry.ts",
            true,
            &[],
            &[
                ("publicName", "middle", "bridge.ts"),
                ("default", "default", "bridge.ts"),
            ],
        ),
        module(
            "bridge.ts",
            true,
            &[],
            &[
                ("middle", "original", "leaf.ts"),
                ("default", "default", "leaf.ts"),
            ],
        ),
        module("leaf.ts", false, &["original", "default"], &[]),
    ];
    let mut graph = graph(&modules);
    assert!(resolve(&mut graph, &modules).is_empty());
    let user = graph.edges.iter().find(|e| e.from == "user.ts").unwrap();
    assert_eq!(user.to, "leaf.ts");
    assert_eq!(user.original_to.as_deref(), Some("entry.ts"));
    assert_eq!(user.kind, EdgeKind::Static);
    assert_eq!(user.confidence, Some(Confidence::High));
    assert!(graph.metadata["entry.ts"].barrel);
    assert!(
        graph
            .edges
            .iter()
            .filter(|e| e.from == "entry.ts")
            .all(|e| e.to == "leaf.ts" && e.is_reexport)
    );
}

#[test]
fn star_exports_require_unique_symbol_proof_and_side_effects_keep_runtime_loading() {
    let mut user = module("user.ts", false, &[], &[]);
    user.imports = vec![route("entry.ts", &["Foo"], false, EdgeKind::Static, false)];
    let mut modules = vec![
        user,
        module(
            "entry.ts",
            true,
            &[],
            &[("*", "*", "left.ts"), ("*", "*", "right.ts")],
        ),
        module("left.ts", false, &["Foo"], &[]),
        module("right.ts", false, &["Bar"], &[]),
    ];
    let mut resolved = graph(&modules);
    resolve(&mut resolved, &modules);
    assert_eq!(
        resolved
            .edges
            .iter()
            .find(|e| e.from == "user.ts")
            .unwrap()
            .to,
        "left.ts"
    );
    modules[3].declared.push("Foo".into());
    let mut ambiguous = graph(&modules);
    assert!(
        resolve(&mut ambiguous, &modules)
            .iter()
            .any(|w| w.contains("ambiguous"))
    );
    let user = ambiguous
        .edges
        .iter()
        .find(|e| e.from == "user.ts")
        .unwrap();
    assert_eq!(user.to, "entry.ts");
    assert_eq!(user.kind, EdgeKind::Reexport);
    assert_eq!(user.confidence, Some(Confidence::Low));
    modules[0].imports[0].wildcard = true;
    let mut namespace = graph(&modules);
    resolve(&mut namespace, &modules);
    assert_eq!(
        namespace
            .edges
            .iter()
            .find(|e| e.from == "user.ts")
            .unwrap()
            .to,
        "entry.ts"
    );
    modules[0].imports[0].side_effect = true;
    let mut side_effect = graph(&modules);
    resolve(&mut side_effect, &modules);
    let user = side_effect
        .edges
        .iter()
        .find(|e| e.from == "user.ts")
        .unwrap();
    assert_eq!(user.to, "entry.ts");
    assert_eq!(user.kind, EdgeKind::Static);
    assert_eq!(user.confidence, None);
}

#[test]
fn loops_and_depth_limits_keep_a_low_confidence_written_target() {
    let mut user = module("user.ts", false, &[], &[]);
    user.imports = vec![route("a.ts", &["Foo"], false, EdgeKind::Type, false)];
    let modules = vec![
        user,
        module("a.ts", true, &[], &[("Foo", "Foo", "b.ts")]),
        module("b.ts", true, &[], &[("Foo", "Foo", "a.ts")]),
    ];
    let mut cyclic = graph(&modules);
    assert!(
        resolve(&mut cyclic, &modules)
            .iter()
            .any(|w| w.contains("cycle"))
    );
    let user = cyclic.edges.iter().find(|e| e.from == "user.ts").unwrap();
    assert_eq!(user.to, "a.ts");
    assert_eq!(user.kind, EdgeKind::Reexport);
    assert_eq!(user.confidence, Some(Confidence::Low));
    for length in [10, 11] {
        let mut user = module("user.ts", false, &[], &[]);
        user.imports = vec![route("b0.ts", &["Foo"], false, EdgeKind::Static, false)];
        let mut modules = vec![user];
        for i in 0..length {
            modules.push(module(
                &format!("b{i}.ts"),
                true,
                &[],
                &[("Foo", "Foo", &format!("b{}.ts", i + 1))],
            ));
        }
        modules.push(module(&format!("b{length}.ts"), false, &["Foo"], &[]));
        let mut limited = graph(&modules);
        let warnings = resolve(&mut limited, &modules);
        let user = limited.edges.iter().find(|e| e.from == "user.ts").unwrap();
        if length == 10 {
            assert_eq!(user.to, "b10.ts");
        } else {
            assert_eq!(user.to, "b0.ts");
            assert!(warnings.iter().any(|w| w.contains("depth 10")));
        }
    }
}

#[test]
fn typescript_source_proves_purity_and_local_export_aliases() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("barrel-source");
    std::fs::create_dir_all(&root).unwrap();
    for (file, source) in [
        ("user.ts", "import { Public } from './facade';"),
        (
            "facade.ts",
            "import { Foo as local } from './leaf'; export { local as Public };",
        ),
        ("leaf.ts", "export const Foo = 1;"),
        (
            "mixed.ts",
            "export {Foo} from './leaf'; console.log('load');",
        ),
    ] {
        std::fs::write(root.join(file), source).unwrap();
    }
    let modules = vec![
        {
            let mut m = module("user.ts", false, &[], &[]);
            m.imports.push(route(
                "facade.ts",
                &["Public"],
                false,
                EdgeKind::Static,
                false,
            ));
            m
        },
        {
            let mut m = module("facade.ts", false, &[], &[]);
            m.imports
                .push(route("leaf.ts", &["Foo"], false, EdgeKind::Static, false));
            m
        },
        module("leaf.ts", false, &["Foo"], &[]),
        module("mixed.ts", false, &[], &[("Foo", "Foo", "leaf.ts")]),
    ];
    let json = serde_json::json!({"modules": [
        {"source":"user.ts","dependencies":[{"module":"./facade","resolved":"facade.ts"}]},
        {"source":"facade.ts","dependencies":[{"module":"./leaf","resolved":"leaf.ts"}]},
        {"source":"leaf.ts","dependencies":[]},
        {"source":"mixed.ts","dependencies":[{"module":"./leaf","resolved":"leaf.ts"}]},
    ]});
    let mut graph = graph(&modules);
    assert!(
        typescript(
            &serde_json::to_vec(&json).unwrap(),
            &mut graph,
            &mut Sources::new(&root).unwrap(),
        )
        .unwrap()
        .is_empty()
    );
    assert!(graph.metadata["facade.ts"].barrel);
    assert!(!graph.metadata["mixed.ts"].barrel);
    let user = graph.edges.iter().find(|e| e.from == "user.ts").unwrap();
    assert_eq!(user.to, "leaf.ts");
    assert_eq!(user.original_to.as_deref(), Some("facade.ts"));
    let facade = graph.edges.iter().find(|e| e.from == "facade.ts").unwrap();
    assert_eq!(facade.kind, EdgeKind::Reexport);
    assert!(facade.is_reexport);
}

#[test]
fn distinct_written_boundaries_survive_resolving_to_the_same_pair() {
    let mut user = module("user.ts", false, &[], &[]);
    user.imports = vec![
        route("entry.ts", &["Foo"], false, EdgeKind::Type, false),
        route("leaf.ts", &["Foo"], false, EdgeKind::Static, false),
    ];
    let modules = vec![
        user,
        module("entry.ts", true, &[], &[("Foo", "Foo", "leaf.ts")]),
        module("leaf.ts", false, &["Foo"], &[]),
    ];
    let mut graph = graph(&modules);
    resolve(&mut graph, &modules);
    let user: Vec<_> = graph.edges.iter().filter(|e| e.from == "user.ts").collect();
    assert_eq!(user.len(), 2);
    assert!(user.iter().all(|e| e.to == "leaf.ts"));
    assert!(
        user.iter()
            .any(|e| e.original_to.as_deref() == Some("entry.ts") && e.kind == EdgeKind::Type)
    );
    assert!(
        user.iter()
            .any(|e| e.original_to.is_none() && e.kind == EdgeKind::Static)
    );
}

#[test]
fn a_star_does_not_forward_default_exports_and_mixed_modules_are_not_bypassed() {
    let mut user = module("user.ts", false, &[], &[]);
    user.imports = vec![route(
        "entry.ts",
        &["default"],
        false,
        EdgeKind::Static,
        false,
    )];
    let mut modules = vec![
        user,
        module("entry.ts", true, &[], &[("*", "*", "leaf.ts")]),
        module("leaf.ts", false, &["default"], &[]),
    ];
    let mut star = graph(&modules);
    resolve(&mut star, &modules);
    assert_eq!(
        star.edges.iter().find(|e| e.from == "user.ts").unwrap().to,
        "entry.ts"
    );
    modules[1].pure = false;
    let mut mixed = graph(&modules);
    resolve(&mut mixed, &modules);
    let user = mixed.edges.iter().find(|e| e.from == "user.ts").unwrap();
    assert_eq!(user.to, "entry.ts");
    assert_eq!(user.kind, EdgeKind::Static);
    assert_eq!(user.confidence, None);
}

#[test]
fn source_metadata_ignores_missing_and_escaping_files() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("barrel-missing");
    std::fs::create_dir_all(&root).unwrap();
    let modules = vec![
        module("missing.ts", false, &[], &[("Foo", "Foo", "leaf.ts")]),
        module("leaf.ts", false, &["Foo"], &[]),
    ];
    let mut graph = graph(&modules);
    let before = graph.edges.clone();
    let json = serde_json::json!({"modules":[{"source":"missing.ts","dependencies":[{"module":"./leaf","resolved":"leaf.ts"}]}]});
    typescript(
        &serde_json::to_vec(&json).unwrap(),
        &mut graph,
        &mut Sources::new(&root).unwrap(),
    )
    .unwrap();
    assert_eq!(graph.edges, before);
    assert!(graph.metadata.is_empty());
    #[cfg(unix)]
    {
        let outside = root.with_extension("outside.ts");
        std::fs::write(&outside, "export {Foo} from './leaf';").unwrap();
        let _ = std::fs::remove_file(root.join("missing.ts"));
        std::os::unix::fs::symlink(&outside, root.join("missing.ts")).unwrap();
        typescript(
            &serde_json::to_vec(&json).unwrap(),
            &mut graph,
            &mut Sources::new(&root).unwrap(),
        )
        .unwrap();
        assert_eq!(graph.edges, before);
        assert!(graph.metadata.is_empty());
    }
}

#[test]
fn forwarding_metadata_is_conservative_about_executable_statements() {
    for source in [
        "export {Foo} from './leaf';",
        "export type {Foo as Public} from './leaf';",
        "import {Foo} from './leaf'; export {Foo};",
        "import * as api from './leaf'; export {api};",
    ] {
        assert!(analyze(source).exports.pure, "{source}");
    }
    for source in [
        "import './load'; export {Foo} from './leaf';",
        "export {Foo} from './leaf'; doWork();",
        "import {Unused} from './leaf'; export {Foo} from './other';",
        "export const Foo = 1;",
    ] {
        assert!(!analyze(source).exports.pure, "{source}");
    }
    assert_eq!(
        analyze("export default class Foo {} export interface Shape {}")
            .exports
            .declared,
        ["Shape", "default"]
    );
    assert_eq!(
        analyze("export async function load() {} export declare class Model {}")
            .exports
            .declared,
        ["Model", "load"]
    );
}

#[test]
fn executable_reexport_modules_retain_runtime_cycles() {
    use std::collections::BTreeSet;
    let mixed = module(
        "mixed.ts",
        false,
        &["Initialized"],
        &[("Foo", "Foo", "leaf.ts")],
    );
    let mut leaf = module("leaf.ts", false, &["Foo"], &[]);
    leaf.imports.push(route(
        "mixed.ts",
        &["Initialized"],
        false,
        EdgeKind::Static,
        false,
    ));
    let modules = vec![mixed, leaf];
    let mut graph = graph(&modules);
    resolve(&mut graph, &modules);
    let reexport = graph.edges.iter().find(|e| e.from == "mixed.ts").unwrap();
    assert_eq!(reexport.kind, EdgeKind::Static);
    assert!(reexport.is_reexport);
    let module_of = graph
        .files
        .iter()
        .map(|file| (file.clone(), file.clone()))
        .collect();
    let cycles = archgraph::graph::cycles(
        &module_of,
        &graph.edges,
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
    );
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].members, ["leaf.ts", "mixed.ts"]);
}

#[test]
fn exported_variable_metadata_collects_all_bindings_not_initializer_names() {
    let exports = analyze(
        r#"
export const Foo = call(1, 2), Bar = { notExported: [3, 4] };
export let Empty, AlsoEmpty;
export const First: Record<string, number> = {}, Second = make<Foo, Bar>();
export const { original: Renamed, Nested: { Inner }, List: [Head, ...Tail], ...Rest } = value;
export const { keep = call(fake, other), alias: actual = (() => ({ hidden: true }))(), [compute(key)]: Computed } = value;
export const [ArrayFirst, , ArrayThird = make(1, 2), ...ArrayRest] = value;
export const Template = `${outside, inside = value}`, AfterTemplate = 1;
export const Deferred = import('./load'), AfterDeferred = 2;
"#,
    )
    .exports;
    assert_eq!(
        exports.declared,
        [
            "AfterDeferred",
            "AfterTemplate",
            "AlsoEmpty",
            "ArrayFirst",
            "ArrayRest",
            "ArrayThird",
            "Bar",
            "Computed",
            "Deferred",
            "Empty",
            "First",
            "Foo",
            "Head",
            "Inner",
            "Renamed",
            "Rest",
            "Second",
            "Tail",
            "Template",
            "actual",
            "keep",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    );
}

#[test]
fn later_exported_declarators_resolve_star_imports_and_preserve_a_runtime_cycle() {
    use std::collections::BTreeSet;
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("barrel-export-declarators");
    std::fs::create_dir_all(&root).unwrap();
    for (file, source) in [
        (
            "consumer.ts",
            "import { Bar } from './facade'; export const Used = Bar;",
        ),
        ("facade.ts", "export * from './leaf';"),
        (
            "leaf.ts",
            "import {Used} from './consumer'; export const Foo = 1, Bar = Used;",
        ),
    ] {
        std::fs::write(root.join(file), source).unwrap();
    }
    let mut consumer = module("consumer.ts", false, &["Used"], &[]);
    consumer
        .imports
        .push(route("facade.ts", &["Bar"], false, EdgeKind::Static, false));
    let mut leaf = module("leaf.ts", false, &["Foo", "Bar"], &[]);
    leaf.imports.push(route(
        "consumer.ts",
        &["Used"],
        false,
        EdgeKind::Static,
        false,
    ));
    let modules = vec![
        consumer,
        module("facade.ts", true, &[], &[("*", "*", "leaf.ts")]),
        leaf,
    ];
    let json = serde_json::json!({"modules":[
        {"source":"consumer.ts","dependencies":[{"module":"./facade","resolved":"facade.ts"}]},
        {"source":"facade.ts","dependencies":[{"module":"./leaf","resolved":"leaf.ts"}]},
        {"source":"leaf.ts","dependencies":[{"module":"./consumer","resolved":"consumer.ts"}]}
    ]});
    let mut graph = graph(&modules);
    assert!(
        typescript(
            &serde_json::to_vec(&json).unwrap(),
            &mut graph,
            &mut Sources::new(&root).unwrap(),
        )
        .unwrap()
        .is_empty()
    );
    let edge = graph
        .edges
        .iter()
        .find(|e| e.from == "consumer.ts")
        .unwrap();
    assert_eq!(edge.to, "leaf.ts");
    assert_eq!(edge.kind, EdgeKind::Static);
    assert_eq!(edge.confidence, Some(Confidence::High));
    let module_of = graph
        .files
        .iter()
        .map(|file| (file.clone(), file.clone()))
        .collect();
    let cycles = archgraph::graph::cycles(
        &module_of,
        &graph.edges,
        &BTreeSet::new(),
        &BTreeSet::from(["facade.ts".into()]),
        false,
    );
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].members, ["consumer.ts", "leaf.ts"]);
}
