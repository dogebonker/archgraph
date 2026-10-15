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
fn ts_misplaced_file_and_its_simulation() {
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
    // discount.ts: 1 edge within orders (place -> discount), 2 into pricing. orders loses its two
    // outgoing edges and keeps place -> discount as one: orders and pricing together go from
    // cohesion 5/7 to 6/7. pricing's own metrics stay, so the simulation leaves it out.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "misplaced-file", "subject": "src/orders/discount.ts",
              "evidence": { "own_module_edges": 1, "target_module": "pricing",
                            "target_module_edges": 2, "via_barrel": false },
              "suggestion": { "move": { "file": "src/orders/discount.ts", "to": "src/pricing/discount.ts" } },
              "simulation": { "after": summary(0, 0, 0, json!({
                  "orders": m(json!(0.6667), 0, 1, json!(1.0)) })) } },
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
                            "target_module_edges": 2, "via_barrel": false },
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
    let (doc, _, _) = scan_recorded("ts-internals", &[]);
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "internals-reach", "subject": "src/a/use.ts -> src/b/make.ts",
              "evidence": { "from": "src/a/use.ts", "to": "src/b/make.ts", "module": "b",
                            "entries": ["src/b/index.ts"] } },
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 5, "modules": 2 } }
        ])
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
                            "target_module_edges": 2, "via_barrel": false },
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
                ".": m(json!(0.0), 0, 2, json!(1.0)),
                "adapters/in": m(json!(0.0), 2, 2, json!(0.5)),
                "adapters/out": m(json!(0.0), 3, 1, json!(0.25)),
                "application": m(json!(0.0), 1, 1, json!(0.5)),
                "domain": m(json!(0.5), 1, 1, json!(0.5)) })),
            "after": summary(2, 0, 0, json!({
                ".": m(json!(0.0), 0, 2, json!(1.0)),
                "adapters/in": m(json!(0.3333), 1, 2, json!(0.6667)),
                "adapters/out": m(Value::Null, 3, 0, json!(0.0)),
                "application": m(json!(0.0), 1, 1, json!(0.5)),
                "domain": m(json!(0.5), 1, 1, json!(0.5)) })),
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

#[test]
fn misplaced_file_needs_a_move_that_adds_no_forbidden_import_or_dependency() {
    // a/f.ts: 1 edge within a (g -> f), 2 into t, imported only from inside a; it also imports
    // x/y.ts.
    let edges = |t_uses_x: bool| {
        let mut edges = vec![
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/a/f.ts", "src/t/t1.ts"),
            ("src/a/f.ts", "src/t/t2.ts"),
            ("src/a/f.ts", "src/x/y.ts"),
        ];
        if t_uses_x {
            edges.push(("src/t/t1.ts", "src/x/z.ts"));
        }
        let edges: Vec<(String, String)> = edges
            .into_iter()
            .map(|(f, t)| (f.to_owned(), t.to_owned()))
            .collect();
        let (dir, input) = common::synthetic(&format!("misplaced-gate-{t_uses_x}"), 6, &edges);
        std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
        (dir, input)
    };
    let misplaced = |doc: &Value| {
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == "misplaced-file")
            .map(|f| f["subject"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let (dir, input) = edges(true);
    let args = [
        dir.as_str(),
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ];
    let (doc, _, _) = common::scan(&args);
    assert_eq!(misplaced(&doc), ["src/a/f.ts"]);

    // In t, f.ts -> x/y.ts would add a forbidden import: no finding.
    let rules = common::write_tmp(
        "rules/t-no-x.yaml",
        "version: 1\nforbid:\n  - { from: \"src/t/**\", to: \"src/x/**\" }\n",
    );
    let (doc, _, _) = common::scan(&[&args[..], &["--rules", &rules]].concat());
    assert_eq!(misplaced(&doc), Vec::<String>::new());

    // When t does not use x yet, the move would give t a new dependency: no finding.
    let (dir, input) = edges(false);
    let (doc, _, _) = common::scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(misplaced(&doc), Vec::<String>::new());
}

#[test]
fn misplaced_file_whose_name_is_taken_has_no_suggestion() {
    // t already has an f.ts, so a/f.ts cannot move there under its name.
    let edges: Vec<(String, String)> = [
        ("src/a/g.ts", "src/a/f.ts"),
        ("src/a/f.ts", "src/t/t1.ts"),
        ("src/a/f.ts", "src/t/f.ts"),
    ]
    .iter()
    .map(|(f, t)| (f.to_string(), t.to_string()))
    .collect();
    let (dir, input) = common::synthetic("misplaced-taken", 4, &edges);
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = common::scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(
        common::finding(&doc, "misplaced-file", "src/a/f.ts"),
        &json!({ "kind": "misplaced-file", "subject": "src/a/f.ts",
                 "evidence": { "own_module_edges": 1, "target_module": "t",
                               "target_module_edges": 2, "via_barrel": false } })
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
