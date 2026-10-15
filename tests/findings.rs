mod common;

use common::{archgraph, layout, run_json, scan_file, scan_recorded};
use serde_json::{Value, json};

/// `{ cohesion, ca, ce, instability }` of one module in a Summary.
fn m(cohesion: Value, ca: usize, ce: usize, instability: Value) -> Value {
    json!({ "cohesion": cohesion, "ca": ca, "ce": ce, "instability": instability })
}

fn summary(forbidden: usize, unplanned: usize, cycles: usize, modules: Value) -> Value {
    json!({ "forbidden": forbidden, "unplanned": unplanned, "cycles": cycles, "modules": modules })
}

#[test]
fn ts_misplaced_file_that_only_imports_another_module_stays() {
    let (doc, _, _) = scan_recorded("ts-misplaced", &[]);
    assert_eq!(
        doc["summary"],
        summary(
            0,
            0,
            0,
            json!({
            "orders": m(json!(0.6), 0, 1, json!(1.0)),
            "pricing": m(json!(1.0), 1, 0, json!(0.0)) })
        )
    );
    // discount.ts has 1 edge within orders (place -> discount) and 2 imports into pricing, but
    // pricing never imports it: it uses pricing, nothing pulls it there.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 7, "modules": 2 } }
        ])
    );
}

#[test]
fn ts_sink_splits_into_its_consumer_groups() {
    let (doc, _, _) = scan_recorded("ts-sink", &[]);
    let dates_moved = json!({
        "after": summary(0, 0, 0, json!({
            "calendar": m(json!(1.0), 0, 0, Value::Null),
            "utils": m(json!(1.0), 2, 0, json!(0.0)) })) });
    // round.ts is imported only from inside utils, so it joins money.ts and its consumers.
    // money.ts serves billing and cart, two modules: a shared provider, not misplaced. Without
    // utils' files as hubs, label propagation finds the same two groups as the sink split.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "misplaced-file", "subject": "src/utils/dates.ts",
              "evidence": { "own_module_edges": 0, "target_module": "calendar",
                            "imports_from_target": 2, "via_barrel": false },
              "suggestion": { "move": { "file": "src/utils/dates.ts", "to": "src/calendar/dates.ts" } },
              "simulation": dates_moved },
            { "kind": "seam", "subject": ".",
              "evidence": { "modularity": 0.4444, "folder_modularity": -0.2361, "hubs": [],
                            "via_barrel": false, "clusters": [
                  { "files": ["src/billing/invoice.ts", "src/billing/tax.ts", "src/cart/total.ts",
                              "src/utils/money.ts", "src/utils/round.ts"],
                    "modules": ["billing", "cart", "utils"] },
                  { "files": ["src/calendar/pick.ts", "src/calendar/view.ts", "src/utils/dates.ts"],
                    "modules": ["calendar", "utils"] } ] } },
            { "kind": "sink-split", "subject": "utils",
              "evidence": { "ca": 3, "via_barrel": false, "components": [
                  { "files": ["src/utils/dates.ts"], "consumers": ["calendar"],
                    "moves": [{ "file": "src/utils/dates.ts", "to": "src/calendar/dates.ts" }] },
                  { "files": ["src/utils/money.ts", "src/utils/round.ts"],
                    "consumers": ["billing", "cart"], "moves": [] } ] },
              "simulation": dates_moved },
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 8, "modules": 4 } }
        ])
    );
}

#[test]
fn ts_internals_reach_past_a_barrel() {
    // Without rules a barrel is a module's public entry only when at least half the imports into
    // the module go through it. Two of the three imports into b do, so use.ts reaching make.ts
    // is reported; both imports into c bypass its barrel, so none is.
    let (doc, _, _) = scan_recorded("ts-internals", &[]);
    let reach: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "internals-reach")
        .collect();
    assert_eq!(
        reach,
        [
            &json!({ "kind": "internals-reach", "subject": "src/a/use.ts -> src/b/make.ts",
                  "evidence": { "from": "src/a/use.ts", "to": "src/b/make.ts", "module": "b",
                                "entries": ["src/b/index.ts"], "barrel_share": 0.6667 } })
        ]
    );
}

#[test]
fn ts_sdp_stable_module_depends_on_a_volatile_one() {
    let (doc, _, _) = scan_recorded("ts-sdp", &[]);
    // core: Ca 2 (x, y), Ce 1 (plugins), I = 1/3. plugins: Ca 1, Ce 2 (z, w), I = 2/3.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 6, "modules": 6 } },
            { "kind": "unstable-dependency", "subject": "core -> plugins",
              "evidence": { "from": "core", "to": "plugins", "from_instability": 0.3333,
                            "to_instability": 0.6667, "imports": 1 } }
        ])
    );
}

#[test]
fn ts_seams_across_folders() {
    let (doc, _, _) = scan_recorded("ts-seams", &[]);
    // Two triangles, each split across data/ and ui/. Six edges, every degree 2:
    // clusters Q = 2 * (3/6 - (6/12)^2) = 0.5; folders Q = 2 * (1/6 - (6/12)^2) = -0.1667.
    // b2.ts talks only to data but nothing imports it: an entry point, not misplaced.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "misplaced-file", "subject": "src/data/a2.ts",
              "evidence": { "own_module_edges": 0, "target_module": "ui",
                            "imports_from_target": 2, "via_barrel": false },
              "suggestion": { "move": { "file": "src/data/a2.ts", "to": "src/ui/a2.ts" } },
              "simulation": { "after": summary(0, 0, 0, json!({
                  "ui": m(json!(0.6), 0, 1, json!(1.0)) })) } },
            { "kind": "seam", "subject": ".",
              "evidence": { "modularity": 0.5, "folder_modularity": -0.1667, "hubs": [],
                            "via_barrel": false, "clusters": [
                  { "files": ["src/data/a2.ts", "src/ui/a1.ts", "src/ui/a3.ts"], "modules": ["data", "ui"] },
                  { "files": ["src/data/b1.ts", "src/data/b3.ts", "src/ui/b2.ts"], "modules": ["data", "ui"] } ] } },
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 6, "modules": 2 } }
        ])
    );
}

#[test]
fn simulate_cli_destination_decides_layer() {
    let hexagonal = layout("ts-hexagonal");
    let embedded = scan_file("ts-hexagonal", &["--rules", &hexagonal]);
    let (sim, code) = run_json(&[
        "simulate",
        &embedded,
        "--move",
        "src/adapters/out/queue.ts",
        "src/adapters/in/queue.ts",
    ]);
    assert_eq!(code, Some(0));
    // queue.ts -> cli.ts becomes internal to adapters/in: forbid[1] no longer matches, the
    // in <-> out module cycle (and with it the only SCC) goes, adapters/in gains cohesion.
    assert_eq!(
        sim,
        json!({
            "schema": "archgraph/v1",
            "kind": "simulation",
            "move": { "file": "src/adapters/out/queue.ts", "to": "src/adapters/in/queue.ts",
                      "module": "adapters/in" },
            "before": summary(3, 0, 1, json!({
                "adapters/in": m(json!(0.0), 2, 2, json!(0.5)),
                "adapters/out": m(json!(0.0), 3, 1, json!(0.25)),
                "application": m(json!(0.0), 1, 1, json!(0.5)),
                "domain": m(json!(0.5), 1, 1, json!(0.5)),
                "main.ts": m(json!(0.0), 0, 2, json!(1.0)) })),
            "after": summary(2, 0, 0, json!({
                "adapters/in": m(json!(0.3333), 1, 2, json!(0.6667)),
                "adapters/out": m(Value::Null, 3, 0, json!(0.0)),
                "application": m(json!(0.0), 1, 1, json!(0.5)),
                "domain": m(json!(0.5), 1, 1, json!(0.5)),
                "main.ts": m(json!(0.0), 0, 2, json!(1.0)) })),
            "new_forbidden": [],
            "removed_forbidden": [
                { "from": "src/adapters/out/queue.ts", "to": "src/adapters/in/cli.ts",
                  "kind": "static", "verdict": "forbidden", "rule": "forbid[1]" } ]
        })
    );

    // rules.ts lands in the adapters layer, so domain/user.ts now imports outward. --rules on a
    // rule-less scan gives the same verdicts as embedded rules.
    let plain = scan_file("ts-hexagonal", &[]);
    let (sim, _) = run_json(&[
        "simulate",
        &plain,
        "--move",
        "src/domain/rules.ts",
        "src/adapters/in/rules.ts",
        "--rules",
        &hexagonal,
    ]);
    assert_eq!(sim["before"]["forbidden"], 3);
    assert_eq!(sim["after"]["forbidden"], 4);
    assert_eq!(
        sim["new_forbidden"],
        json!([{ "from": "src/domain/user.ts", "to": "src/adapters/in/rules.ts", "kind": "static",
                 "verdict": "forbidden", "rule": "layers" }])
    );
    assert_eq!(sim["removed_forbidden"], json!([]));
    assert_eq!(
        sim["after"]["modules"]["domain"],
        m(json!(0.0), 1, 2, json!(0.6667))
    );
    assert_eq!(
        sim["after"]["modules"]["adapters/in"],
        m(json!(0.0), 3, 2, json!(0.4))
    );

    let out = archgraph(&["simulate", &plain, "--move", "src/nope.ts", "src/x.ts"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("src/nope.ts"));
}

#[test]
fn sink_split_skips_moves_that_collide() {
    // a/fmt.ts and b/fmt.ts serve only x, so both would move to src/x/fmt.ts.
    let (dir, input) = common::synthetic(
        "sink-collision",
        6,
        &[
            ("src/x/main.ts", "src/utils/a/fmt.ts"),
            ("src/x/main.ts", "src/utils/b/fmt.ts"),
            ("src/utils/a/fmt.ts", "src/utils/c.ts"),
            ("src/y/m.ts", "src/utils/c.ts"),
            ("src/z/m.ts", "src/utils/c.ts"),
        ]
        .map(|(f, t)| (f.to_owned(), t.to_owned())),
    );
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = common::scan(&[
        &dir,
        "--collector-input",
        &input,
        "--depth",
        "1",
        "--language",
        "typescript",
    ]);
    let sink = common::finding(&doc, "sink-split", "utils");
    assert_eq!(
        sink["evidence"]["components"],
        json!([
            { "files": ["src/utils/a/fmt.ts", "src/utils/b/fmt.ts"], "consumers": ["x"], "moves": [] },
            { "files": ["src/utils/c.ts"], "consumers": ["y", "z"], "moves": [] }
        ])
    );
    assert!(sink.get("simulation").is_none(), "{sink:#}");
}

#[test]
fn sink_split_suggests_no_move_out_of_the_consumer_module() {
    // boot.ts serves only main.ts, a module of its own: src/boot.ts would be another one.
    let doc = scan_edges(
        "sink-root-consumer",
        &[
            ("src/main.ts", "src/utils/boot.ts"),
            ("src/w/m.ts", "src/utils/c.ts"),
            ("src/y/m.ts", "src/utils/c.ts"),
            ("src/z/m.ts", "src/utils/c.ts"),
        ],
        &["--depth", "1"],
    );
    let sink = common::finding(&doc, "sink-split", "utils");
    assert_eq!(
        sink["evidence"]["components"][0],
        json!({ "files": ["src/utils/boot.ts"], "consumers": ["main.ts"], "moves": [] })
    );
    assert!(sink.get("simulation").is_none(), "{sink:#}");
}

#[test]
fn unstable_dependency_follows_barrels_like_ca_and_ce() {
    // s imports b's barrel, which re-exports v: for Ca and Ce s depends on v, not on b.
    // s: Ca 2 (p, q), Ce 1 (v), I = 1/3. v: Ca 1, Ce 2 (w, z), I = 2/3.
    let import: &[&str] = &["local", "import"];
    let (dir, input) = common::synthetic_typed(
        "sdp-barrel",
        7,
        &[
            ("p/a.ts", "s/f.ts", import),
            ("q/a.ts", "s/f.ts", import),
            ("s/f.ts", "b/index.ts", import),
            ("b/index.ts", "v/x.ts", &["local", "export"]),
            ("v/x.ts", "w/y.ts", import),
            ("v/x.ts", "z/k.ts", import),
        ],
    );
    let (doc, _, _) = common::scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let unstable: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "unstable-dependency")
        .collect();
    assert_eq!(
        unstable,
        [&json!({ "kind": "unstable-dependency", "subject": "s -> v",
                  "evidence": { "from": "s", "to": "v", "from_instability": 0.3333,
                                "to_instability": 0.6667, "imports": 1 } })]
    );
}

/// `scan` of synthetic `edges` with source root `src`.
fn scan_edges(name: &str, edges: &[(&str, &str)], extra: &[&str]) -> Value {
    let edges: Vec<(String, String)> = edges
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
    let (dir, input) = common::synthetic(name, edges.len() + 1, &edges);
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let args = [
        dir.as_str(),
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ];
    common::scan(&[&args[..], extra].concat()).0
}

fn misplaced(doc: &Value) -> Vec<&Value> {
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "misplaced-file")
        .collect()
}

#[test]
fn misplaced_file_is_pulled_by_its_importers_not_by_its_imports() {
    // f.ts imports three files of t but only g.ts, in its own module, imports it: f.ts uses t,
    // it does not belong there.
    let doc = scan_edges(
        "misplaced-imports-only",
        &[
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/a/f.ts", "src/t/t1.ts"),
            ("src/a/f.ts", "src/t/t2.ts"),
            ("src/a/f.ts", "src/t/t3.ts"),
        ],
        &[],
    );
    assert_eq!(misplaced(&doc), Vec::<&Value>::new());

    // f.ts is imported by two files of t and by one of its own module: t pulls it.
    let doc = scan_edges(
        "misplaced-imported-by-target",
        &[
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/t2.ts", "src/a/f.ts"),
        ],
        &[],
    );
    assert_eq!(
        misplaced(&doc),
        [&json!({ "kind": "misplaced-file", "subject": "src/a/f.ts",
                  "evidence": { "own_module_edges": 1, "target_module": "t",
                                "imports_from_target": 2, "via_barrel": false },
                  "suggestion": { "move": { "file": "src/a/f.ts", "to": "src/t/f.ts" } },
                  "simulation": { "after": summary(0, 0, 0, json!({
                      "a": m(json!(0.0), 0, 1, json!(1.0)),
                      "t": m(json!(1.0), 1, 0, json!(0.0)) })) } })]
    );

    // Two imports from t do not outweigh three edges within a: f.ts stays.
    let doc = scan_edges(
        "misplaced-own-outweighs",
        &[
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/a/h.ts", "src/a/f.ts"),
            ("src/a/f.ts", "src/a/k.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/t2.ts", "src/a/f.ts"),
        ],
        &[],
    );
    assert_eq!(misplaced(&doc), Vec::<&Value>::new());
}

#[test]
fn misplaced_file_keeps_its_folder_within_the_module() {
    let to = |doc: &Value| {
        misplaced(doc)
            .iter()
            .map(|f| f["suggestion"]["move"]["to"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // a/domain/f.ts is imported by two files of t, which has a domain/ folder: it goes there.
    let edges = |t1: &'static str, t2: &'static str| {
        vec![
            ("src/a/api/h.ts", "src/a/domain/g.ts"),
            ("src/a/domain/g.ts", "src/a/domain/f.ts"),
            (t1, "src/a/domain/f.ts"),
            (t2, "src/a/domain/f.ts"),
        ]
    };
    let doc = scan_edges(
        "misplaced-keeps-folder",
        &edges("src/t/domain/t1.ts", "src/t/api/t2.ts"),
        &["--depth", "1"],
    );
    assert_eq!(to(&doc), ["src/t/domain/f.ts"]);

    // t has no domain/ folder: the module's root.
    let doc = scan_edges(
        "misplaced-folder-missing",
        &edges("src/t/jobs/t1.ts", "src/t/api/t2.ts"),
        &["--depth", "1"],
    );
    assert_eq!(to(&doc), ["src/t/f.ts"]);

    // t's ui/ folder is the module t/ui, not t: the module's root.
    let rules = common::write_tmp(
        "rules/nested-modules.yaml",
        "version: 1\nmodules:\n  - { id: t/ui, path: \"src/t/ui/**\" }\n  \
         - { id: t, path: \"src/t/**\" }\n  - { id: a, path: \"src/a/**\" }\n",
    );
    let doc = scan_edges(
        "misplaced-folder-other-module",
        &[
            ("src/a/api/h.ts", "src/a/ui/g.ts"),
            ("src/a/ui/g.ts", "src/a/ui/f.ts"),
            ("src/t/t1.ts", "src/a/ui/f.ts"),
            ("src/t/api/t2.ts", "src/a/ui/f.ts"),
            ("src/t/ui/u.ts", "src/t/ui/v.ts"),
        ],
        &["--rules", &rules],
    );
    assert_eq!(to(&doc), ["src/t/f.ts"]);
}

#[test]
fn misplaced_file_needs_a_move_that_adds_no_forbidden_import_or_dependency() {
    // t1.ts and t2.ts import a/f.ts, g.ts in a does too; f.ts imports x/y.ts and w/v.ts.
    let edges = |t_uses_x_and_w: bool| {
        let mut edges = vec![
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/t2.ts", "src/a/f.ts"),
            ("src/a/f.ts", "src/x/y.ts"),
            ("src/a/f.ts", "src/w/v.ts"),
        ];
        if t_uses_x_and_w {
            edges.extend([("src/t/t1.ts", "src/x/z.ts"), ("src/t/t1.ts", "src/w/u.ts")]);
        }
        edges
    };
    let subjects = |doc: &Value| {
        misplaced(doc)
            .iter()
            .map(|f| f["subject"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let doc = scan_edges("misplaced-gate-true", &edges(true), &[]);
    assert_eq!(subjects(&doc), ["src/a/f.ts"]);

    // In t, f.ts -> x/y.ts would add a forbidden import: no finding.
    let rules = common::write_tmp(
        "rules/t-no-x.yaml",
        "version: 1\nforbid:\n  - { from: \"src/t/**\", to: \"src/x/**\" }\n",
    );
    let doc = scan_edges("misplaced-gate-true", &edges(true), &["--rules", &rules]);
    assert_eq!(subjects(&doc), Vec::<String>::new());

    // When t uses neither x nor w, the move trades t's dependency on a for two new ones: no
    // finding.
    let doc = scan_edges("misplaced-gate-false", &edges(false), &[]);
    assert_eq!(subjects(&doc), Vec::<String>::new());
}

#[test]
fn misplaced_file_whose_name_is_taken_has_no_suggestion() {
    // t already has an f.ts, so a/f.ts cannot move there under its name.
    let doc = scan_edges(
        "misplaced-taken",
        &[
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/f.ts", "src/a/f.ts"),
        ],
        &[],
    );
    assert_eq!(
        common::finding(&doc, "misplaced-file", "src/a/f.ts"),
        &json!({ "kind": "misplaced-file", "subject": "src/a/f.ts",
                 "evidence": { "own_module_edges": 1, "target_module": "t",
                               "imports_from_target": 2, "via_barrel": false } })
    );
}

#[test]
fn seam_needs_clusters_holding_half_the_files() {
    // ts-seams' two triangles (6 files) plus `lone` files with no kept import: each imports only
    // its own test file, which the default excludes drop.
    for (lone, reported) in [(6, true), (7, false)] {
        let mut edges: Vec<(String, String)> = [
            ("src/ui/a1.ts", "src/data/a2.ts"),
            ("src/ui/a1.ts", "src/ui/a3.ts"),
            ("src/ui/a3.ts", "src/data/a2.ts"),
            ("src/ui/b2.ts", "src/data/b1.ts"),
            ("src/ui/b2.ts", "src/data/b3.ts"),
            ("src/data/b1.ts", "src/data/b3.ts"),
        ]
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
        for i in 0..lone {
            edges.push((
                format!("src/lone/k{i}.ts"),
                format!("src/lone/k{i}.test.ts"),
            ));
        }
        let (dir, input) = common::synthetic(&format!("seam-coverage-{lone}"), 20, &edges);
        std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
        let (doc, _, _) = common::scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        let seam = doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "seam");
        assert_eq!(seam, reported, "6 clustered of {} files", 6 + lone);
    }
}
