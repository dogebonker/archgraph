//! Canonical Rust results consumed by the dependency-free map preview tests.
use std::collections::{BTreeMap, BTreeSet};

use boundrail::model::{Config, Document, Edge, EdgeKind, ImportRecord, Language, Move, Source};
use serde_json::{Value, json};

fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind,
        ..Edge::default()
    }
}

fn projection(doc: &Document) -> Value {
    let mut cycles: Vec<Value> = doc
        .cycles
        .iter()
        .map(|cycle| {
            let mut members = cycle.members.clone();
            members.sort();
            json!({"level":cycle.level,"members":members})
        })
        .collect();
    cycles.sort_by_key(Value::to_string);
    let modules: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.module.as_str()))
        .collect();
    let production: BTreeSet<_> = doc
        .files
        .iter()
        .filter(|f| f.class != Some(boundrail::model::ModuleClass::Tests))
        .map(|f| f.path.as_str())
        .collect();
    let directed: BTreeSet<_> = doc
        .edges
        .iter()
        .filter(|e| {
            (doc.config.all_kinds || e.kind.is_runtime())
                && production.contains(e.from.as_str())
                && production.contains(e.to.as_str())
        })
        .filter_map(|e| {
            let (from, to) = (*modules.get(e.from.as_str())?, *modules.get(e.to.as_str())?);
            (from != to).then_some((from, to))
        })
        .collect();
    let pairs: Vec<_> = directed
        .iter()
        .filter(|(from, to)| from < to && directed.contains(&(*to, *from)))
        .map(|(from, to)| [*from, *to])
        .collect();
    let mut verdicts: Vec<Value> = doc
        .edges
        .iter()
        .map(|e| json!({"from":e.from,"to":e.to,"verdict":e.verdict,"rule":e.rule,"kind":e.kind}))
        .collect();
    verdicts.sort_by_key(|v| {
        (
            v["from"].to_string(),
            v["to"].to_string(),
            v["verdict"].to_string(),
            v["rule"].to_string(),
            v["kind"].to_string(),
        )
    });
    // The target as the preview recomputes it: verdicts and broken imports per slice, progress.
    // The preview matches no package or global, so a claim with a `pkg:`, `category:` or
    // `global:` rule, and the conforming count it feeds, are not previewed.
    let packaged = |id: &str| {
        doc.config.rules.as_ref().is_some_and(|r| {
            r.doc.forbid.iter().any(|f| {
                f.id.as_deref() == Some(id)
                    && ["pkg:", "category:", "global:"]
                        .iter()
                        .any(|prefix| f.to.starts_with(prefix))
            })
        })
    };
    let target = doc.target.as_ref().map(|t| {
        let claims: Vec<Value> = t
            .claims
            .iter()
            .map(|c| {
                if packaged(&c.id) {
                    return json!({"id":c.id,"preview":false});
                }
                let slices: Vec<Value> = c
                    .slices
                    .iter()
                    .map(|s| json!({"slice":s.slice,"broken":s.broken}))
                    .collect();
                json!({"id":c.id,"verdict":c.verdict,"slices":slices})
            })
            .collect();
        let mut progress = json!(t.progress);
        if t.claims.iter().any(|c| packaged(&c.id)) {
            progress["conforming"] = json!({"preview":false});
        }
        json!({"claims":claims,"progress":progress})
    });
    json!({"cycles":cycles,"two_way_pairs":pairs,"target":target,
        "largest_cycle":doc.cycles.iter().filter(|c| c.level == boundrail::model::CycleLevel::Module).map(|c| c.members.len()).max().unwrap_or(0),
        "modules":doc.summary.modules,"forbidden":doc.summary.forbidden,"unplanned":doc.summary.unplanned,"verdicts":verdicts})
}

fn case(
    name: &str,
    pairs: &[(&str, &str, EdgeKind)],
    file: &str,
    to: &str,
    options: Value,
    policy: Option<Value>,
    language: Language,
) -> Value {
    let mut config =
        json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":null});
    for (key, value) in options.as_object().unwrap() {
        config[key] = value.clone();
    }
    if let Some(policy) = policy {
        config["rules"] = json!({"path":"layout.yaml","sha256":"0000000000000000000000000000000000000000000000000000000000000000","doc":policy});
    }
    let config: Config = serde_json::from_value(config).unwrap();
    let source: Source = serde_json::from_value(json!({"language":language,"collector":{"name":"fixture","cruised":0},"git":null,"generated_at":"2026-10-18T00:00:00Z"})).unwrap();
    let edges: Vec<_> = pairs
        .iter()
        .map(|(from, to, kind)| edge(from, to, *kind))
        .collect();
    let files = edges
        .iter()
        .flat_map(|e| [e.from.clone(), e.to.clone()])
        .chain([file.to_owned()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let doc = boundrail::analyze(source, config, files, edges, vec![], None).unwrap();
    finish(name, doc, &[(file, to)])
}

/// One move is stored as an object, several as the ordered list the preview applies together.
fn finish(name: &str, doc: Document, moves: &[(&str, &str)]) -> Value {
    let rules = boundrail::rules::compile(&doc.config).unwrap();
    let moves: Vec<Move> = moves
        .iter()
        .map(|(file, to)| Move {
            file: (*file).into(),
            to: (*to).into(),
        })
        .collect();
    let mut after = boundrail::simulate::apply(&doc, rules.as_ref(), &moves).unwrap();
    after.target = rules
        .as_ref()
        .and_then(|rules| boundrail::target::analyze(&after, rules));
    let moved = match moves.as_slice() {
        [one] => json!(one),
        many => json!(many),
    };
    json!({"name":name,"document":doc,"move":moved,"before":projection(&doc),"after":projection(&after)})
}

fn fixtures() -> Value {
    use EdgeKind::{Dynamic as L, Reexport as R, Static as V, Type as T};
    use Language::{Go, Typescript as Ts};
    let basic = [("a/one.ts", "b/two.ts", V), ("b/two.ts", "a/three.ts", V)];
    let mut cases = vec![
        case(
            "cut-module-cycle",
            &basic,
            "a/three.ts",
            "b/three.ts",
            json!({}),
            None,
            Ts,
        ),
        case(
            "merge-modules-retains-file-cycle",
            &[("a/one.ts", "b/two.ts", V), ("b/two.ts", "a/one.ts", V)],
            "a/one.ts",
            "b/one.ts",
            json!({}),
            None,
            Ts,
        ),
        case(
            "relocate-creates-cycle",
            &[("a/one.ts", "b/two.ts", V), ("b/two.ts", "c/three.ts", V)],
            "c/three.ts",
            "a/three.ts",
            json!({}),
            None,
            Ts,
        ),
        case(
            "weighted-type-and-lazy",
            &[
                ("a/one.ts", "b/two.ts", T),
                ("b/two.ts", "a/three.ts", L),
                ("a/one.ts", "a/three.ts", V),
            ],
            "a/three.ts",
            "c/three.ts",
            json!({}),
            None,
            Ts,
        ),
        case(
            "all-kinds-cycle",
            &[("a/one.ts", "b/two.ts", T), ("b/two.ts", "a/three.ts", L)],
            "a/three.ts",
            "b/three.ts",
            json!({"all_kinds":true}),
            None,
            Ts,
        ),
        case(
            "custom-weights",
            &[
                ("a/one.ts", "b/two.ts", T),
                ("a/three.ts", "b/two.ts", L),
                ("a/one.ts", "a/three.ts", V),
            ],
            "a/three.ts",
            "c/three.ts",
            json!({}),
            Some(
                json!({"version":1,"edge_weights":{"runtime":2.0,"lazy":0.7,"type":0.2,"reexport":0.05}}),
            ),
            Ts,
        ),
        case(
            "capture-modules",
            &[("src/domains/cart/a.ts", "src/domains/pay/b.ts", V)],
            "src/domains/pay/b.ts",
            "src/domains/cart/b.ts",
            json!({}),
            Some(
                json!({"version":1,"modules":[{"id":"domain/<name>","path":"src/domains/<name>/**"}]}),
            ),
            Ts,
        ),
        case(
            "source-root-outside",
            &[("src/a/one.ts", "lib/two.ts", V)],
            "lib/two.ts",
            "src/a/two.ts",
            json!({"source_root":"src"}),
            None,
            Ts,
        ),
        case(
            "go-package-directories",
            &[
                ("cmd/tool/main.go", "internal/deep/a.go", V),
                ("internal/deep/a.go", "internal/other/b.go", V),
            ],
            "internal/other/b.go",
            "internal/deep/b.go",
            json!({"package_modules":true}),
            None,
            Go,
        ),
        case(
            "test-back-edge-is-not-cycle",
            &[
                ("a/one.ts", "b/two.ts", V),
                ("b/two.test.ts", "a/one.ts", V),
            ],
            "b/two.ts",
            "a/two.ts",
            json!({"include_tests":true}),
            None,
            Ts,
        ),
        case(
            "file-granularity",
            &[("a/one.ts", "b/two.ts", V), ("b/two.ts", "a/one.ts", V)],
            "a/one.ts",
            "c/one.ts",
            json!({"granularity":"file"}),
            None,
            Ts,
        ),
        case(
            "layers-and-default",
            &[
                ("ui/view.ts", "domain/model.ts", V),
                ("domain/model.ts", "ui/helper.ts", V),
                ("ui/view.ts", "misc/other.ts", V),
            ],
            "ui/helper.ts",
            "domain/helper.ts",
            json!({}),
            Some(json!({"version":1,"layers":["ui/**","domain/**"],"default":"forbidden"})),
            Ts,
        ),
        case(
            "forbid-before-allow-kind-filter",
            &[("app/a.ts", "store/b.ts", V), ("app/c.ts", "store/b.ts", T)],
            "store/b.ts",
            "app/b.ts",
            json!({}),
            Some(
                json!({"version":1,"forbid":[{"id":"runtime-store","from":"app/**","to":"store/**","kinds":["runtime"]}],"allow":[{"id":"app-imports","from":"app/**","to":"**"}]}),
            ),
            Ts,
        ),
        case(
            "entrypoint-bypass",
            &[
                ("app/a.ts", "billing/internal.ts", V),
                ("app/a.ts", "billing/index.ts", V),
            ],
            "billing/internal.ts",
            "app/internal.ts",
            json!({}),
            Some(json!({"version":1,"entrypoints":{"billing/**":"billing/index.ts"}})),
            Ts,
        ),
        case(
            "public-capture-boundary",
            &[
                ("domains/a/a.ts", "domains/b/internal.ts", V),
                ("domains/a/a.ts", "domains/b/index.ts", V),
            ],
            "domains/b/internal.ts",
            "domains/a/internal.ts",
            json!({}),
            Some(
                json!({"version":1,"public":[{"module":"domains/<name>/**","entries":["domains/<name>/index.ts"]}]}),
            ),
            Ts,
        ),
        case(
            "reachable-forbid",
            &[
                ("app/client.ts", "shared/bridge.ts", V),
                ("shared/bridge.ts", "server/private.ts", V),
            ],
            "server/private.ts",
            "shared/private.ts",
            json!({}),
            Some(
                json!({"version":1,"forbid":[{"id":"no-server","from":"app/**","to":"server/**","reachable":true}]}),
            ),
            Ts,
        ),
    ];
    cases.push(case(
        "elixir-test-back-edge-is-not-cycle",
        &[
            ("lib/a.ex", "service/b.ex", V),
            ("service/b_test.exs", "lib/a.ex", V),
        ],
        "lib/a.ex",
        "service/a.ex",
        json!({"include_tests":true}),
        None,
        Language::Elixir,
    ));
    cases.push(case(
        "glob-braces-and-question-mark",
        &[
            ("src/app/a.ts", "lib/core.ts", V),
            ("src/ui/b.ts", "lib/safe.ts", V),
            ("src/app/long.ts", "lib/core.ts", V),
        ],
        "lib/core.ts",
        "lib/other.ts",
        json!({}),
        Some(json!({"version":1,"forbid":[{"id":"brace-question","from":"src/{app,ui}/?.ts","to":"lib/{core,safe}.ts"}]})),
        Ts,
    ));
    cases.push(case(
        "glob-negated-character-classes",
        &[
            ("src/beta.ts", "data/value.ts", V),
            ("src/alpha.ts", "data/value.ts", V),
            ("src/beta.ts", "data/xray.ts", V),
        ],
        "data/value.ts",
        "data/xvalue.ts",
        json!({}),
        Some(json!({"version":1,"forbid":[{"id":"negated-classes","from":"src/[!a]*.ts","to":"data/[^x]*.ts"}]})),
        Ts,
    ));
    cases.push(case(
        "glob-negated-class-matches-separator",
        &[("src/beta.ts", "data/value.ts", V)],
        "data/value.ts",
        "lib/value.ts",
        json!({}),
        Some(json!({"version":1,"forbid":[{"id":"negated-separator","from":"src[!a]*.ts","to":"data/**"}]})),
        Ts,
    ));
    // A layout written for a target: claim reason and source, moves, and a capture inside braces.
    cases.push(case(
        "target-layout",
        &[
            ("src/a/handler.ts", "src/a/repo.ts", V),
            ("src/b/handler.ts", "src/shared/id.ts", V),
            ("src/b/repo.ts", "src/shared/id.ts", V),
            ("src/app/c/handler/h.ts", "lib/x.ts", V),
        ],
        "src/a/handler.ts",
        "src/app/a/handler/handler.ts",
        json!({}),
        Some(json!({"version":1,"layer-reason":"Handlers sit above repositories.",
            "forbid":[{"id":"layered","from":"{src/<name>/handler*.ts,src/app/<name>/handler/*.ts}",
                "to":"{src/<name>/repo*.ts,src/app/<name>/repo/*.ts}",
                "reason":"Handlers reach storage through services.","source":"docs/adr/ADR-0003.md"}],
            "moves":[{"from":"src/<name>/handler*.ts","to":"src/app/<name>/handler/"},{"from":"lib/x.ts","to":"core/"}]})),
        Ts,
    ));
    // Moving the domain file out unbinds slice a: the claim holds where it held in one slice.
    let sliced = [
        ("src/m/a/app/x.ts", "src/m/a/domain/d.ts", V),
        ("src/m/b/app/y.ts", "src/shared/id.ts", V),
        ("src/m/b/domain/e.ts", "src/shared/id.ts", V),
        ("src/m/c/domain/f.ts", "src/shared/id.ts", V),
    ];
    let claim = json!({"id":"app-no-domain","from":"src/m/<name>/app/**","to":"src/m/<name>/domain/**",
        "reason":"apps never reach a domain"});
    cases.push(case(
        "target-claim-binding",
        &sliced,
        "src/m/a/domain/d.ts",
        "src/shared/d.ts",
        json!({}),
        Some(json!({"version":1,"forbid":[claim]})),
        Ts,
    ));
    // An earlier id-less rule takes the import's verdict; the claim still sees it, renamed.
    cases.push(case(
        "target-first-match",
        &sliced,
        "src/m/a/app/x.ts",
        "src/m/a/app/w.ts",
        json!({}),
        Some(json!({"version":1,"forbid":[{"from":"src/m/*/app/**","to":"src/m/*/domain/**"},
            {"id":"app-no-domain","from":"src/m/a/app/**","to":"src/m/a/domain/**","reason":"apps never reach a domain"}]})),
        Ts,
    ));
    cases.push(case(
        "target-reachable-claim",
        &[
            ("app/client.ts", "shared/bridge.ts", V),
            ("shared/bridge.ts", "server/private.ts", V),
        ],
        "server/private.ts",
        "shared/private.ts",
        json!({}),
        Some(
            json!({"version":1,"forbid":[{"id":"no-server","from":"app/**","to":"server/**","reachable":true,"reason":"clients never reach the server"}]}),
        ),
        Ts,
    ));
    // A package claim is not previewed, nor the conforming count; the file claim is. Package
    // rules need import accounting, so the scan carries an import record per edge.
    let config: Config = serde_json::from_value(json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,
        "rules":{"path":"layout.yaml","sha256":"0000000000000000000000000000000000000000000000000000000000000000","doc":{"version":1,"forbid":[
            {"id":"no-db","from":"src/<name>/service.ts","to":"src/db/**","reason":"services reach the database through repositories"},
            {"id":"no-sql","from":"src/<name>/service.ts","to":"src/db/**","reason":"services never use SQL"},
            {"id":"no-sql","from":"src/<name>/service.ts","to":"pkg:pg","reason":"services never use SQL"}]}}})).unwrap();
    let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":0},"git":null,"generated_at":"2026-10-18T00:00:00Z"})).unwrap();
    let pairs = [
        ("src/a/service.ts", "src/db/q.ts"),
        ("src/b/service.ts", "src/shared/x.ts"),
    ];
    let config = Config {
        import_records: Some(
            pairs
                .iter()
                .map(|(from, to)| ImportRecord {
                    from: (*from).into(),
                    specifier: (*to).into(),
                    targets: vec![(*to).into()],
                    ..Default::default()
                })
                .collect(),
        ),
        ..config
    };
    let doc = boundrail::analyze(
        source,
        config,
        pairs
            .iter()
            .flat_map(|(f, t)| [f.to_string(), t.to_string()])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        pairs.iter().map(|(f, t)| edge(f, t, V)).collect(),
        vec![],
        None,
    )
    .unwrap();
    let mut package = finish(
        "target-package-claim",
        doc,
        &[("src/b/service.ts", "src/b/svc/service.ts")],
    );
    package["warnings"] = json!([
        "Internal-only preview: external dependency policy verdicts and exposure are not recomputed. Run boundrail simulate for authoritative results."
    ]);
    cases.push(package);
    cases.push(case(
        "test-class-survives-rename",
        &[
            ("a/one.ts", "b/two.ts", V),
            ("b/two.test.ts", "a/one.ts", V),
        ],
        "b/two.test.ts",
        "a/helper.ts",
        json!({"include_tests":true}),
        None,
        Ts,
    ));
    let mut written = case(
        "written-barrel-rule-target",
        &[
            ("app/a.ts", "billing/internal.ts", V),
            ("billing/index.ts", "billing/internal.ts", R),
        ],
        "billing/index.ts",
        "billing/public.ts",
        json!({}),
        Some(json!({"version":1,"entrypoints":{"billing/**":"billing/index.ts"}})),
        Ts,
    );
    let mut input: boundrail::model::ScanInput =
        serde_json::from_value(written["document"].clone()).unwrap();
    input
        .edges
        .iter_mut()
        .find(|e| e.from == "app/a.ts")
        .unwrap()
        .original_to = Some("billing/index.ts".into());
    written = finish(
        "written-barrel-rule-target",
        boundrail::restore_scan(input).unwrap(),
        &[("billing/index.ts", "billing/public.ts")],
    );
    cases.push(written);
    // The map's move list: both moves applied together, as `simulate --move A B --move C D`.
    let chained = case(
        "chained-moves-with-rules",
        &[
            ("app/view.ts", "domain/model.ts", V),
            ("app/helper.ts", "domain/model.ts", V),
            ("domain/model.ts", "app/helper.ts", V),
            ("domain/model.ts", "infra/db.ts", V),
        ],
        "app/helper.ts",
        "domain/helper.ts",
        json!({}),
        Some(
            json!({"version":1,"layers":["domain/**","app/**"],"forbid":[{"id":"no-infra","from":"domain/**","to":"infra/**"}]}),
        ),
        Ts,
    );
    cases.push(finish(
        "chained-moves-with-rules",
        boundrail::restore_scan(serde_json::from_value(chained["document"].clone()).unwrap())
            .unwrap(),
        &[
            ("app/helper.ts", "domain/helper.ts"),
            ("infra/db.ts", "domain/store/db.ts"),
        ],
    ));
    let mut grabbag_edges = vec![
        ("app/a.ts", "utils/date.ts", V),
        ("utils/work.ts", "app/a.ts", V),
    ];
    grabbag_edges.extend([
        ("c1/a.ts", "utils/date.ts", V),
        ("c2/a.ts", "utils/date.ts", V),
        ("c3/a.ts", "utils/date.ts", V),
        ("c4/a.ts", "utils/date.ts", V),
        ("c5/a.ts", "utils/date.ts", V),
    ]);
    let mut grabbag = case(
        "grabbag-refinement-needs-cli",
        &grabbag_edges,
        "utils/work.ts",
        "app/work.ts",
        json!({}),
        None,
        Ts,
    );
    grabbag["unsupported"] = json!(true);
    cases.push(grabbag);
    Value::Array(cases)
}

#[test]
fn map_preview_fixtures_match_rust() {
    let actual = fixtures();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/whatif-parity.json");
    if std::env::var_os("UPDATE_WHATIF_FIXTURES").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
    }
    let expected: Value = serde_json::from_slice(
        &std::fs::read(path).expect("run UPDATE_WHATIF_FIXTURES=1 cargo test --test whatif"),
    )
    .unwrap();
    assert_eq!(
        actual, expected,
        "Rust preview semantics changed; regenerate and run Node parity tests"
    );
    assert!(actual.as_array().unwrap().len() >= 15);
    for (name, before, after) in [
        ("glob-braces-and-question-mark", 2, 1),
        ("glob-negated-character-classes", 1, 0),
        ("glob-negated-class-matches-separator", 1, 0),
        ("target-layout", 1, 1),
    ] {
        let case = actual
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        assert_eq!(case["before"]["forbidden"], before);
        assert_eq!(case["after"]["forbidden"], after);
    }
    let conform = |done, of| json!({"conforming":{"done":done,"of":of}});
    let moved = |done, of| json!({"conforming":{"done":1,"of":2},"moved":{"done":done,"of":of}});
    for (name, before, after) in [
        (
            "target-claim-binding",
            ("holds_in", conform(1, 2)),
            ("holds", conform(1, 1)),
        ),
        (
            "target-first-match",
            ("broken", conform(0, 0)),
            ("broken", conform(0, 0)),
        ),
        (
            "target-reachable-claim",
            ("broken", conform(0, 0)),
            ("vacuous", conform(0, 0)),
        ),
        (
            "target-package-claim",
            ("holds_in", json!({"conforming":{"preview":false}})),
            ("broken", json!({"conforming":{"preview":false}})),
        ),
        (
            "target-layout",
            ("holds_in", moved(1, 3)),
            ("holds_in", moved(2, 3)),
        ),
    ] {
        let case = actual
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        for (side, (verdict, progress)) in [("before", before), ("after", after)] {
            let target = &case[side]["target"];
            assert_eq!(target["claims"][0]["verdict"], verdict, "{name} {side}");
            assert_eq!(target["progress"], progress, "{name} {side}");
        }
        if name == "target-package-claim" {
            assert_eq!(
                case["before"]["target"]["claims"][1],
                json!({"id":"no-sql","preview":false})
            );
        }
    }
}
