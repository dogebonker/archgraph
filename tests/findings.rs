mod common;

use common::{archgraph, layout, scan_file};
use serde_json::{Value, json};

/// `{ cohesion, ca, ce, instability }` of one module in a Summary.
fn m(cohesion: Value, ca: usize, ce: usize, instability: Value) -> Value {
    json!({ "cohesion": cohesion, "ca": ca, "ce": ce, "instability": instability, "weighted_ca": ca as f64, "weighted_ce": ce as f64 })
}

fn summary(forbidden: usize, unplanned: usize, cycles: usize, modules: Value) -> Value {
    json!({ "forbidden": forbidden, "unplanned": unplanned, "cycles": cycles, "type_only_cycles": 0, "modules": modules })
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
    let moved = |file: &str| {
        let to = file.replace("shared", "calendar");
        json!({ "file": file, "to": to })
    };
    // round.ts is imported only from inside shared, so it joins money.ts and its consumers.
    // money.ts serves billing and cart, two modules: a shared provider. Both groups hold two
    // files. clock.ts and dates.ts import nothing and sit in shared: leaf utilities, not misplaced,
    // though the sink split moves them with their group. Without shared' files as hubs, label
    // propagation finds the same two groups as the sink split.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "seam", "subject": ".",
              "evidence": { "modularity": 0.5, "folder_modularity": -0.2734, "hubs": [],
                            "via_barrel": false, "clusters": [
                  { "files": ["src/billing/invoice.ts", "src/billing/tax.ts", "src/cart/total.ts",
                              "src/shared/money.ts", "src/shared/round.ts"],
                    "modules": ["billing", "cart", "shared"] },
                  { "files": ["src/calendar/pick.ts", "src/calendar/view.ts", "src/shared/clock.ts",
                              "src/shared/dates.ts"],
                    "modules": ["calendar", "shared"] } ] } },
            { "kind": "sink-split", "subject": "shared",
              "evidence": { "ca": 3, "via_barrel": false, "components": [
                  { "files": ["src/shared/clock.ts", "src/shared/dates.ts"], "consumers": ["calendar"],
                    "moves": [moved("src/shared/clock.ts"), moved("src/shared/dates.ts")] },
                  { "files": ["src/shared/money.ts", "src/shared/round.ts"],
                    "consumers": ["billing", "cart"], "moves": [] } ] },
              "simulation": { "after": summary(0, 0, 0, json!({
                  "calendar": m(json!(1.0), 0, 0, Value::Null),
                  "shared": m(json!(1.0), 2, 0, json!(0.0)) })) } },
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 9, "modules": 4 } }
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
        [&json!({ "kind": "internals-reach", "subject": "b",
                  "evidence": { "module": "b", "entries": ["src/b/index.ts"], "imports": 1,
                                "importers": 1, "importing_files": 1, "barrel_share": 0.6667,
                                "edges": [{ "from": "src/a/use.ts", "to": "src/b/make.ts" }] } })]
    );
}

#[test]
fn ts_sdp_stable_module_depends_on_a_volatile_one() {
    let (doc, _, _) = scan_recorded("ts-sdp", &[]);
    // core: Ca 2 (x, y), Ce 1 (plugins), I = 1/3. plugins: Ca 1, Ce 2 (z, w), I = 2/3. api.ts and
    // boot.ts both import plugins: two imports, the minimum.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 7, "modules": 6 } },
            { "kind": "unstable-dependency", "subject": "core -> plugins",
              "evidence": { "from": "core", "to": "plugins", "from_instability": 0.3333,
                            "to_instability": 0.6667, "imports": 2 } }
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
        common::without_import_metadata(&sim),
        json!({
            "schema": "archgraph/v1.2",
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
                  "kind": "runtime", "weight": 1.0, "verdict": "forbidden", "rule": "forbid[1]" } ]
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
        common::without_import_metadata(&sim["new_forbidden"]),
        json!([{ "from": "src/domain/user.ts", "to": "src/adapters/in/rules.ts", "kind": "runtime", "weight": 1.0, "verdict": "forbidden", "rule": "layers" }])
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

    // Repeated --move applies both moves together: the first move's fix and the second move's
    // layer violation, with the cycle the second one closes through adapters/in.
    let (both, code) = run_json(&[
        "simulate",
        &embedded,
        "--move",
        "src/adapters/out/queue.ts",
        "src/adapters/in/queue.ts",
        "--move",
        "src/domain/rules.ts",
        "src/adapters/in/rules.ts",
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(
        both["move"],
        json!({ "file": "src/adapters/out/queue.ts", "to": "src/adapters/in/queue.ts", "module": "adapters/in" })
    );
    assert_eq!(
        both["moves"],
        json!([
            { "file": "src/adapters/out/queue.ts", "to": "src/adapters/in/queue.ts", "module": "adapters/in" },
            { "file": "src/domain/rules.ts", "to": "src/adapters/in/rules.ts", "module": "adapters/in" } ])
    );
    let counts = |s: &Value| (s["forbidden"].clone(), s["cycles"].clone());
    assert_eq!(counts(&both["before"]), (json!(3), json!(1)));
    assert_eq!(counts(&both["after"]), (json!(3), json!(1)));
    let edges = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|e| (e["from"].clone(), e["to"].clone(), e["rule"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        edges(&both["new_forbidden"]),
        [(
            json!("src/domain/user.ts"),
            json!("src/adapters/in/rules.ts"),
            json!("layers")
        )]
    );
    assert_eq!(
        edges(&both["removed_forbidden"]),
        [(
            json!("src/adapters/out/queue.ts"),
            json!("src/adapters/in/cli.ts"),
            json!("forbid[1]")
        )]
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
            ("src/x/main.ts", "src/shared/a/fmt.ts"),
            ("src/x/main.ts", "src/shared/b/fmt.ts"),
            ("src/y/m.ts", "src/shared/c.ts"),
            ("src/z/m.ts", "src/shared/c.ts"),
            ("src/z/m.ts", "src/shared/d.ts"),
        ]
        .map(|(f, t)| (f.to_owned(), t.to_owned())),
    );
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--depth",
        "1",
        "--language",
        "typescript",
    ]);
    let sink = common::finding(&doc, "sink-split", "shared");
    assert_eq!(
        sink["evidence"]["components"],
        json!([
            { "files": ["src/shared/a/fmt.ts", "src/shared/b/fmt.ts"], "consumers": ["x"], "moves": [] },
            { "files": ["src/shared/c.ts", "src/shared/d.ts"], "consumers": ["y", "z"], "moves": [] }
        ])
    );
    assert!(sink.get("simulation").is_none(), "{sink:#}");
}

#[test]
fn sink_split_needs_two_groups_of_two_files() {
    // shared serves x with two files and y, z with one: a lone file is no second group.
    let doc = scan_edges(
        "sink-one-real-group",
        &[
            ("src/x/m.ts", "src/shared/a.ts"),
            ("src/x/m.ts", "src/shared/b.ts"),
            ("src/y/m.ts", "src/shared/c.ts"),
            ("src/z/m.ts", "src/shared/c.ts"),
        ],
        &["--depth", "1"],
    );
    assert!(
        !doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "sink-split"),
        "{:#}",
        doc["findings"]
    );
}

/// The sink-split components of `doc`'s shared, as (files, consumers).
fn components(doc: &Value) -> Value {
    let sink = common::finding(doc, "sink-split", "shared");
    let c: Vec<Value> = sink["evidence"]["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| json!([c["files"], c["consumers"]]))
        .collect();
    json!(c)
}

#[test]
fn sink_split_leaves_out_composition_roots() {
    // main.ts, which nobody imports, wires shared, y and z together; it uses u1 and u3 only to
    // register them. Without it shared serves y with u1, u2 and w, z with u3, u4.
    let doc = scan_edges(
        "sink-root",
        &[
            ("src/y/a.ts", "src/shared/u1.ts"),
            ("src/y/a.ts", "src/shared/u2.ts"),
            ("src/z/b.ts", "src/shared/u3.ts"),
            ("src/w/c.ts", "src/shared/u4.ts"),
            ("src/z/b.ts", "src/shared/u4.ts"),
            ("src/main.ts", "src/shared/u1.ts"),
            ("src/main.ts", "src/shared/u3.ts"),
            ("src/main.ts", "src/y/a.ts"),
            ("src/main.ts", "src/z/b.ts"),
        ],
        &["--depth", "1"],
    );
    assert_eq!(
        components(&doc),
        json!([
            [["src/shared/u1.ts", "src/shared/u2.ts"], ["y"]],
            [["src/shared/u3.ts", "src/shared/u4.ts"], ["w", "z"]]
        ])
    );
}

#[test]
fn sink_split_counts_runtime_consumers_only() {
    // x's type-only import of b1.ts uses no code of that group.
    let import: &[&str] = &["local", "import"];
    let (dir, input) = common::synthetic_typed(
        "sink-type-consumer",
        8,
        &[
            ("src/x/m.ts", "src/shared/a1.ts", import),
            ("src/x/m.ts", "src/shared/a2.ts", import),
            ("src/y/m.ts", "src/shared/b1.ts", import),
            ("src/z/m.ts", "src/shared/b2.ts", import),
            ("src/y/m.ts", "src/shared/b2.ts", import),
            (
                "src/x/m.ts",
                "src/shared/b1.ts",
                &["local", "type-only", "import"],
            ),
        ],
    );
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--depth",
        "1",
    ]);
    assert_eq!(
        components(&doc),
        json!([
            [["src/shared/a1.ts", "src/shared/a2.ts"], ["x"]],
            [["src/shared/b1.ts", "src/shared/b2.ts"], ["y", "z"]]
        ])
    );
}

#[test]
fn sink_split_never_moves_public_api() {
    // The package's entry re-exports a1.ts: it stays where importers of the package find it,
    // and only a2.ts moves to x.
    let import: &[&str] = &["local", "import"];
    let (dir, input) = common::synthetic_typed(
        "sink-public",
        9,
        &[
            ("src/index.ts", "src/shared/a1.ts", &["local", "export"]),
            ("src/x/m.ts", "src/shared/a1.ts", import),
            ("src/x/m.ts", "src/shared/a2.ts", import),
            ("src/y/m.ts", "src/shared/b1.ts", import),
            ("src/z/m.ts", "src/shared/b2.ts", import),
            ("src/y/m.ts", "src/shared/b2.ts", import),
        ],
    );
    let root = std::path::Path::new(&dir);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{ "main": "./dist/index.js" }"#,
    )
    .unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--depth",
        "1",
    ]);
    let sink = common::finding(&doc, "sink-split", "shared");
    assert_eq!(
        sink["evidence"]["components"][0]["moves"],
        json!([{ "file": "src/shared/a2.ts", "to": "src/x/a2.ts" }])
    );
}

#[test]
fn unstable_dependency_leaves_out_composition_roots() {
    // boot (Ca 4, Ce 3, I 0.4286) imports commands (I 0.75) twice, but only from cli.ts, which
    // nobody imports and which wires commands, lib and util together: no finding.
    let doc = scan_edges(
        "sdp-root",
        &[
            ("src/x/a.ts", "src/boot/env.ts"),
            ("src/y/a.ts", "src/boot/env.ts"),
            ("src/z/a.ts", "src/boot/env.ts"),
            ("src/w/a.ts", "src/boot/env.ts"),
            ("src/boot/cli.ts", "src/commands/a.ts"),
            ("src/boot/cli.ts", "src/commands/b.ts"),
            ("src/boot/cli.ts", "src/lib/l.ts"),
            ("src/boot/cli.ts", "src/util/u.ts"),
            ("src/commands/a.ts", "src/p/p.ts"),
            ("src/commands/a.ts", "src/q/q.ts"),
            ("src/commands/b.ts", "src/r/r.ts"),
        ],
        &["--depth", "1"],
    );
    let instability = |id: &str| {
        doc["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()["instability"]
            .clone()
    };
    assert_eq!(
        (instability("boot"), instability("commands")),
        (json!(0.4286), json!(0.75))
    );
    assert!(
        !doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "unstable-dependency"),
        "{:#}",
        doc["findings"]
    );
}

fn has_sink_split(doc: &Value) -> bool {
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["kind"] == "sink-split")
}

#[test]
fn sink_split_ignores_the_parent_module_as_a_consumer() {
    // app/main.ts registers app/t's controller files; app is the folder above app/t. Without
    // it, app/t serves one group: no split.
    let doc = scan_edges(
        "sink-parent",
        &[
            ("src/app/main.ts", "src/app/t/reg.ts"),
            ("src/app/main.ts", "src/app/t/ctl.ts"),
            ("src/app/a/m.ts", "src/app/t/t1.ts"),
            ("src/app/b/m.ts", "src/app/t/t2.ts"),
            ("src/app/c/m.ts", "src/app/t/t1.ts"),
            ("src/app/a/m.ts", "src/app/t/t2.ts"),
        ],
        &[],
    );
    assert!(!has_sink_split(&doc), "{:#}", doc["findings"]);
}

#[test]
fn sink_split_counts_a_file_importing_a_non_index_named_index() {
    // main.ts imports x, y and shared's index.styles.ts, which is no index: it
    // consumes shared, it does not register it.
    let doc = scan_edges(
        "sink-index-styles",
        &[
            ("src/boot.ts", "src/main.ts"),
            ("src/main.ts", "src/x/m.ts"),
            ("src/main.ts", "src/y/m.ts"),
            ("src/main.ts", "src/shared/index.styles.ts"),
            ("src/x/m.ts", "src/shared/a1.ts"),
            ("src/x/m.ts", "src/shared/a2.ts"),
            ("src/y/m.ts", "src/shared/b1.ts"),
            ("src/z/m.ts", "src/shared/b2.ts"),
            ("src/y/m.ts", "src/shared/b2.ts"),
        ],
        &["--depth", "1"],
    );
    assert_eq!(
        components(&doc),
        json!([
            [["src/shared/a1.ts", "src/shared/a2.ts"], ["x"]],
            [["src/shared/b1.ts", "src/shared/b2.ts"], ["y", "z"]],
            [["src/shared/index.styles.ts"], ["main.ts"]]
        ])
    );
}

#[test]
fn sink_split_ignores_a_wiring_file_that_imports_only_the_entry() {
    // main.ts imports shared only through its barrel, alongside x and y: it registers shared, it
    // does not consume a1 or b1. Without it, a1, a2 serve x and b1, b2 serve y and z.
    let import: &[&str] = &["local", "import"];
    let export: &[&str] = &["local", "export"];
    let (dir, input) = common::synthetic_typed(
        "sink-entry-wiring",
        12,
        &[
            ("src/shared/index.ts", "src/shared/a1.ts", export),
            ("src/shared/index.ts", "src/shared/b1.ts", export),
            ("src/boot.ts", "src/main.ts", import),
            ("src/main.ts", "src/shared/index.ts", import),
            ("src/main.ts", "src/x/m.ts", import),
            ("src/main.ts", "src/y/m.ts", import),
            ("src/x/m.ts", "src/shared/a1.ts", import),
            ("src/x/m.ts", "src/shared/a2.ts", import),
            ("src/y/m.ts", "src/shared/b1.ts", import),
            ("src/z/m.ts", "src/shared/b2.ts", import),
            ("src/y/m.ts", "src/shared/b2.ts", import),
        ],
    );
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--depth",
        "1",
    ]);
    assert_eq!(
        components(&doc),
        json!([
            [["src/shared/a1.ts", "src/shared/a2.ts"], ["x"]],
            [["src/shared/b1.ts", "src/shared/b2.ts"], ["y", "z"]]
        ])
    );
}

#[test]
fn sink_split_skips_a_module_named_by_kind() {
    // services and errors are folders of one kind of code: their files serve whoever needs
    // that kind, and separate groups of importers are expected. A domain folder splits.
    for (sink, reported) in [("services", false), ("errors", false), ("billing", true)] {
        let f = |n: &str| format!("src/{sink}/{n}.ts");
        let (s1, s2, s3, s4) = (f("s1"), f("s2"), f("s3"), f("s4"));
        let doc = scan_edges(
            &format!("sink-kind-{sink}"),
            &[
                ("src/p/p.ts", s1.as_str()),
                ("src/p/p.ts", s2.as_str()),
                ("src/x/m.ts", s3.as_str()),
                ("src/y/m.ts", s4.as_str()),
                ("src/x/m.ts", s4.as_str()),
            ],
            &["--depth", "1"],
        );
        assert_eq!(
            has_sink_split(&doc),
            reported,
            "{sink}: {:#}",
            doc["findings"]
        );
    }
}

#[test]
fn sink_split_drops_groups_that_import_each_other() {
    // a1.ts imports b1.ts: splitting shared along the groups would make the halves import each
    // other.
    let doc = scan_edges(
        "sink-cross-group",
        &[
            ("src/x/m.ts", "src/shared/a1.ts"),
            ("src/x/m.ts", "src/shared/a2.ts"),
            ("src/y/m.ts", "src/shared/b1.ts"),
            ("src/z/m.ts", "src/shared/b2.ts"),
            ("src/y/m.ts", "src/shared/b2.ts"),
            ("src/shared/a1.ts", "src/shared/b1.ts"),
        ],
        &["--depth", "1"],
    );
    assert!(!has_sink_split(&doc), "{:#}", doc["findings"]);
}

#[test]
fn unstable_dependency_skips_a_facade_over_its_private_implementation() {
    // pkg/api (I 1/3) imports pkg/_impl (I 2/3) twice: a facade over its own private code.
    // Named pkg/impl the same imports are reported.
    for (imp, reported) in [("_impl", false), ("impl", true)] {
        let i = format!("src/pkg/{imp}/i.ts");
        let edges = [
            ("src/x/a.ts", "src/pkg/api/f.ts"),
            ("src/y/b.ts", "src/pkg/api/f.ts"),
            ("src/pkg/api/f.ts", i.as_str()),
            ("src/pkg/api/g.ts", i.as_str()),
            (i.as_str(), "src/z/c.ts"),
            (i.as_str(), "src/w/d.ts"),
        ];
        let doc = scan_edges(&format!("sdp-facade{imp}"), &edges, &[]);
        let found = doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "unstable-dependency");
        assert_eq!(found, reported, "{imp}: {:#}", doc["findings"]);
    }
}

#[test]
fn sink_split_suggests_no_move_out_of_the_consumer_module() {
    // boot.ts and env.ts serve only main.ts, a module of its own: src/boot.ts would be another.
    let doc = scan_edges(
        "sink-root-consumer",
        &[
            ("src/main.ts", "src/shared/boot.ts"),
            ("src/main.ts", "src/shared/env.ts"),
            ("src/w/m.ts", "src/shared/c2.ts"),
            ("src/w/m.ts", "src/shared/c.ts"),
            ("src/y/m.ts", "src/shared/c.ts"),
            ("src/z/m.ts", "src/shared/c.ts"),
        ],
        &["--depth", "1"],
    );
    let sink = common::finding(&doc, "sink-split", "shared");
    assert_eq!(
        sink["evidence"]["components"][0],
        json!({ "files": ["src/shared/boot.ts", "src/shared/env.ts"], "consumers": ["main.ts"],
                "moves": [] })
    );
    assert!(sink.get("simulation").is_none(), "{sink:#}");
}

/// Unstable-dependency findings of x, y -> core -> plugins -> z, w (core I = 1/3, plugins I =
/// 2/3, as in ts-sdp) plus `extra` edges.
fn unstable(name: &str, extra: &[(&str, &str, &[&str])]) -> Vec<Value> {
    let import: &[&str] = &["local", "import"];
    let mut edges = vec![
        ("x/a.ts", "core/api.ts", import),
        ("y/b.ts", "core/api.ts", import),
        ("plugins/loader.ts", "z/c.ts", import),
        ("plugins/loader.ts", "w/d.ts", import),
    ];
    edges.extend_from_slice(extra);
    let (dir, input) = common::synthetic_typed(name, edges.len() + 1, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "unstable-dependency")
        .cloned()
        .collect()
}

#[test]
fn unstable_dependency_counts_at_least_two_runtime_imports() {
    let import: &[&str] = &["local", "import"];
    let (type_only, dynamic): (&[&str], &[&str]) = (
        &["local", "type-only", "import"],
        &["local", "dynamic-import"],
    );
    // One import is below the minimum.
    let one = unstable("sdp-one", &[("core/api.ts", "plugins/loader.ts", import)]);
    assert_eq!(one, Vec::<Value>::new());

    // A type-only and a dynamic import set core's Ce, but only the static one is counted.
    let mixed = unstable(
        "sdp-mixed",
        &[
            ("core/api.ts", "plugins/loader.ts", import),
            ("core/boot.ts", "plugins/loader.ts", dynamic),
            ("core/kinds.ts", "plugins/loader.ts", type_only),
        ],
    );
    assert_eq!(mixed, Vec::<Value>::new());

    // Through a barrel: Ca and Ce count core -> plugins, but no file of core imports plugins.
    let barrel = unstable(
        "sdp-barrel",
        &[
            ("core/api.ts", "b/index.ts", import),
            ("core/boot.ts", "b/index.ts", import),
            ("b/index.ts", "plugins/loader.ts", &["local", "export"]),
        ],
    );
    assert_eq!(barrel, Vec::<Value>::new());

    // Pure re-exports have no runtime coupling weight.
    let reexports = unstable(
        "sdp-reexport",
        &[
            ("core/api.ts", "plugins/loader.ts", &["local", "export"]),
            ("core/boot.ts", "plugins/loader.ts", &["local", "export"]),
        ],
    );
    assert!(reexports.is_empty(), "{reexports:#?}");

    let two = unstable(
        "sdp-two",
        &[
            ("core/api.ts", "plugins/loader.ts", import),
            ("core/boot.ts", "plugins/loader.ts", import),
        ],
    );
    assert_eq!(
        two,
        [
            json!({ "kind": "unstable-dependency", "subject": "core -> plugins",
                 "evidence": { "from": "core", "to": "plugins", "from_instability": 0.3333,
                               "to_instability": 0.6667, "imports": 2 } })
        ]
    );
}

#[test]
fn unstable_dependency_skips_a_module_and_its_own_subfolder() {
    // core's own files importing core/plugins is the folder's depth, not a dependency between
    // two modules: same instabilities as ts-sdp, no finding.
    let import: &[&str] = &["local", "import"];
    let edges = [
        ("x/a.ts", "core/api.ts", import),
        ("y/b.ts", "core/api.ts", import),
        ("core/api.ts", "core/plugins/loader.ts", import),
        ("core/boot.ts", "core/plugins/loader.ts", import),
        ("core/plugins/loader.ts", "z/c.ts", import),
        ("core/plugins/loader.ts", "w/d.ts", import),
    ];
    let (dir, input) = common::synthetic_typed("sdp-subfolder", 8, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let instability = |id: &str| {
        doc["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()["instability"]
            .clone()
    };
    assert_eq!(
        (instability("core"), instability("core/plugins")),
        (json!(0.3333), json!(0.6667))
    );
    assert!(
        !doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "unstable-dependency"),
        "{:#}",
        doc["findings"]
    );

    // The other way round: core/plugins' files importing core's own files, same numbers.
    let edges = [
        ("x/a.ts", "core/plugins/api.ts", import),
        ("y/b.ts", "core/plugins/api.ts", import),
        ("core/plugins/api.ts", "core/loader.ts", import),
        ("core/plugins/boot.ts", "core/loader.ts", import),
        ("core/loader.ts", "z/c.ts", import),
        ("core/loader.ts", "w/d.ts", import),
    ];
    let (dir, input) = common::synthetic_typed("sdp-parent", 8, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert!(
        !doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "unstable-dependency"),
        "{:#}",
        doc["findings"]
    );
}

#[test]
fn folder_ancestry_ignores_the_root_ids() {
    // With --depth 0, lib/p.ts and lib/q.ts outside src are module `..`, tool.ts outside src is
    // `../tool.ts`: a root file, not a folder below `..`. `..` (I 1/3) imports it (I 2/3) twice.
    let doc = scan_edges(
        "ancestry-root-ids",
        &[
            ("src/x/a.ts", "lib/p.ts"),
            ("src/y.ts", "lib/q.ts"),
            ("lib/p.ts", "tool.ts"),
            ("lib/q.ts", "tool.ts"),
            ("tool.ts", "src/z.ts"),
            ("tool.ts", "src/w.ts"),
        ],
        &["--depth", "0"],
    );
    let unstable: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "unstable-dependency")
        .map(|f| &f["subject"])
        .collect();
    assert_eq!(
        unstable,
        [&json!(".. -> ../tool.ts")],
        "{:#}",
        doc["modules"]
    );
}

#[test]
fn internals_reach_counts_only_imports_the_entry_re_exports() {
    // b's barrel re-exports make.ts, not helper.ts: importing make.ts bypasses the barrel,
    // importing helper.ts uses something the barrel does not offer.
    let import: &[&str] = &["local", "import"];
    let (dir, input) = common::synthetic_typed(
        "reach-re-exported",
        8,
        &[
            ("src/b/index.ts", "src/b/make.ts", &["local", "export"]),
            ("src/a/x.ts", "src/b/make.ts", import),
            ("src/a/y.ts", "src/b/helper.ts", import),
            ("src/c/p.ts", "src/b/index.ts", import),
            ("src/c/q.ts", "src/b/index.ts", import),
        ],
    );
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let reach: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "internals-reach")
        .map(|f| &f["evidence"]["edges"])
        .collect();
    assert_eq!(
        reach,
        [&json!([{ "from": "src/a/x.ts", "to": "src/b/make.ts" }])]
    );
}

#[test]
fn internals_reach_treats_package_entries_as_public() {
    // package.json exports b/internal.ts as "./internal": importing it is using an entry, and it
    // counts toward the share of imports through entries (3 of 4).
    let import: &[&str] = &["local", "import"];
    let export: &[&str] = &["local", "export"];
    let (dir, input) = common::synthetic_typed(
        "reach-package-entry",
        8,
        &[
            ("src/b/index.ts", "src/b/make.ts", export),
            ("src/b/index.ts", "src/b/internal.ts", export),
            ("src/a/x.ts", "src/b/internal.ts", import),
            ("src/a/y.ts", "src/b/make.ts", import),
            ("src/c/p.ts", "src/b/index.ts", import),
            ("src/c/q.ts", "src/b/index.ts", import),
        ],
    );
    let root = std::path::Path::new(&dir);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{ "exports": { "./internal": "./dist/b/internal.js" } }"#,
    )
    .unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let reach: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "internals-reach")
        .collect();
    assert_eq!(
        reach,
        [&json!({ "kind": "internals-reach", "subject": "b",
                  "evidence": { "module": "b", "entries": ["src/b/index.ts", "src/b/internal.ts"],
                                "imports": 1, "importers": 1, "importing_files": 1,
                                "barrel_share": 0.75,
                                "edges": [{ "from": "src/a/y.ts", "to": "src/b/make.ts" }] } })]
    );
}

#[test]
fn internals_reach_is_one_finding_per_module_listing_at_most_twenty_imports() {
    // 22 files of a and one of c import b's internals, 23 files of d its barrel (share 0.5):
    // one finding for b counting all 23 imports from 2 modules, listing the first 20.
    let import: &[&str] = &["local", "import"];
    let names: Vec<(String, String)> = (0..23)
        .map(|i| (format!("src/a/u{i:02}.ts"), format!("src/d/w{i:02}.ts")))
        .collect();
    let mut edges: Vec<(&str, &str, &[&str])> = vec![
        ("src/b/index.ts", "src/b/make.ts", &["local", "export"]),
        ("src/b/index.ts", "src/b/k.ts", &["local", "export"]),
        ("src/c/v.ts", "src/b/k.ts", import),
    ];
    for (i, (a, d)) in names.iter().enumerate() {
        if i < 22 {
            edges.push((a, "src/b/make.ts", import));
        }
        edges.push((d, "src/b/index.ts", import));
    }
    let (dir, input) = common::synthetic_typed("reach-grouped", edges.len() + 1, &edges);
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let reach: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "internals-reach")
        .collect();
    let listed: Vec<Value> = names[..20]
        .iter()
        .map(|(a, _)| json!({ "from": a, "to": "src/b/make.ts" }))
        .collect();
    assert_eq!(
        reach,
        [&json!({ "kind": "internals-reach", "subject": "b",
                  "evidence": { "module": "b", "entries": ["src/b/index.ts"], "imports": 23,
                                "importers": 2, "importing_files": 23, "barrel_share": 0.5,
                                "edges": listed } })]
    );
}

/// Internals-reach findings of a TypeScript scan of `edges` under `rules` (YAML).
fn reach_with_rules(name: &str, rules: &str, edges: &[(&str, &str, &[&str])]) -> Vec<Value> {
    let rules = common::write_tmp(&format!("rules/{name}.yaml"), rules);
    let (dir, input) = common::synthetic_typed(name, edges.len() + 1, edges);
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--depth",
        "1",
        "--rules",
        &rules,
    ]);
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "internals-reach")
        .cloned()
        .collect()
}

#[test]
fn internals_reach_with_rules_reports_what_check_forbids() {
    let import: &[&str] = &["local", "import"];
    let export: &[&str] = &["local", "export"];
    // The public module is common's own files only, so catch.ts below it is outside: `check`
    // forbids its import of constants.ts, and so does the finding, though common is a folder
    // above the importer's module.
    let reach = reach_with_rules(
        "reach-rules-subfolder",
        "version: 1\npublic:\n  - { module: \"src/common/*\", entries: [\"src/common/index.ts\"] }\n",
        &[
            ("src/common/index.ts", "src/common/constants.ts", export),
            (
                "src/common/decorators/core/catch.ts",
                "src/common/constants.ts",
                import,
            ),
        ],
    );
    assert_eq!(
        reach,
        [
            json!({ "kind": "internals-reach", "subject": "src/common/*",
                 "evidence": { "module": "common", "entries": ["src/common/index.ts"],
                               "imports": 1, "importers": 1, "importing_files": 1,
                               "edges": [{ "from": "src/common/decorators/core/catch.ts",
                                           "to": "src/common/constants.ts" }] } })
        ]
    );

    // One finding per bound public module, though both sit in module features.
    let reach = reach_with_rules(
        "reach-rules-instances",
        "version: 1\npublic:\n  - { module: \"src/features/<name>/**\", \
         entries: [\"src/features/<name>/index.ts\"] }\n",
        &[
            ("src/features/a/index.ts", "src/features/a/impl.ts", export),
            ("src/features/b/index.ts", "src/features/b/impl.ts", export),
            ("src/app/x.ts", "src/features/a/impl.ts", import),
            ("src/app/x.ts", "src/features/b/impl.ts", import),
            ("src/app/y.ts", "src/features/a/index.ts", import),
        ],
    );
    let found = |n: &str| {
        json!({ "kind": "internals-reach", "subject": format!("src/features/{n}/**"),
                "evidence": { "module": "features",
                              "entries": [format!("src/features/{n}/index.ts")],
                              "imports": 1, "importers": 1, "importing_files": 1,
                              "edges": [{ "from": "src/app/x.ts",
                                          "to": format!("src/features/{n}/impl.ts") }] } })
    };
    assert_eq!(reach, [found("a"), found("b")]);
}

#[test]
fn internals_reach_skips_imports_from_a_subfolder_of_the_module() {
    // Half the imports into common use its barrel. catch.ts sits in common/decorators/core, below
    // common: importing common's barrel, which re-exports constants.ts, would be a cycle, so its
    // import of constants.ts is not reported. z.ts, in app, is.
    let import: &[&str] = &["local", "import"];
    let (dir, input) = common::synthetic_typed(
        "reach-subfolder",
        7,
        &[
            (
                "src/common/index.ts",
                "src/common/constants.ts",
                &["local", "export"],
            ),
            (
                "src/common/decorators/core/catch.ts",
                "src/common/constants.ts",
                import,
            ),
            ("src/app/x.ts", "src/common/index.ts", import),
            ("src/app/y.ts", "src/common/index.ts", import),
            ("src/app/z.ts", "src/common/constants.ts", import),
        ],
    );
    std::fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--depth",
        "3",
    ]);
    let reach: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "internals-reach")
        .collect();
    assert_eq!(
        reach,
        [&json!({ "kind": "internals-reach", "subject": "common",
                  "evidence": { "module": "common", "entries": ["src/common/index.ts"],
                                "imports": 1, "importers": 1, "importing_files": 1,
                                "barrel_share": 0.5,
                                "edges": [{ "from": "src/app/z.ts",
                                            "to": "src/common/constants.ts" }] } })]
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
    scan(&[&args[..], extra].concat()).0
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

    // f.ts is imported by three files of t and by one of its own module: t pulls it.
    let doc = scan_edges(
        "misplaced-imported-by-target",
        &[
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/t2.ts", "src/a/f.ts"),
            ("src/t/t3.ts", "src/a/f.ts"),
        ],
        &[],
    );
    assert_eq!(
        misplaced(&doc),
        [&json!({ "kind": "misplaced-file", "subject": "src/a/f.ts",
                  "evidence": { "own_module_edges": 1, "target_module": "t",
                                "imports_from_target": 3, "via_barrel": false },
                  "suggestion": { "move": { "file": "src/a/f.ts", "to": "src/t/f.ts" } },
                  "simulation": { "after": summary(0, 0, 0, json!({
                      "a": m(json!(0.0), 0, 1, json!(1.0)),
                      "t": m(json!(1.0), 1, 0, json!(0.0)) })) } })]
    );

    // Two imports from t are not more than twice the one edge within a: f.ts stays.
    let doc = scan_edges(
        "misplaced-two-against-one",
        &[
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/t2.ts", "src/a/f.ts"),
        ],
        &[],
    );
    assert_eq!(misplaced(&doc), Vec::<&Value>::new());

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
fn misplaced_file_stays_when_both_folders_name_a_kind_of_code() {
    // f.ts is imported by three files of `to` and imports g.ts of its own module, which pulls it
    // to `to`
    // between domain folders. Between two folders named by kind (repositories/, services/), the
    // codebase is organised by kind and the move would change what the file is: no finding.
    // Kinds match in either case, singular or plural.
    for (i, (from, to, reported)) in [
        ("repositories", "services", false),
        ("Repository", "Controller", false),
        ("migrations", "management", false),
        ("menus", "components", false),
        ("Layout", "containers", false),
        ("widgets", "icons", false),
        ("styles", "ui", false),
        ("chains", "signer", true),
        ("utils", "calendar", true),
        ("chains", "services", true),
    ]
    .into_iter()
    .enumerate()
    {
        let (f, g, t1, t2, t3) = (
            format!("src/{from}/f.ts"),
            format!("src/{from}/g.ts"),
            format!("src/{to}/t1.ts"),
            format!("src/{to}/t2.ts"),
            format!("src/{to}/t3.ts"),
        );
        let doc = scan_edges(
            &format!("misplaced-kind-{i}"),
            &[(&f, &g), (&t1, &f), (&t2, &f), (&t3, &f)],
            &[],
        );
        let subjects: Vec<&Value> = misplaced(&doc).iter().map(|m| &m["subject"]).collect();
        let expected = if reported { vec![json!(f)] } else { vec![] };
        assert_eq!(
            subjects,
            expected.iter().collect::<Vec<_>>(),
            "{from} -> {to}"
        );
    }
}

#[test]
fn misplaced_file_skips_leaf_utilities() {
    // f.ts is pulled to t by three importers. In utils/ and importing nothing it is a leaf
    // utility, kept where shared helpers live; once it imports a file it is not; in billing/ the
    // folder is no utility folder.
    for (name, folder, imports, reported) in [
        ("leaf-utils", "utils", false, false),
        ("leaf-storage", "Storage", false, false),
        ("leaf-imports", "utils", true, true),
        ("leaf-domain", "billing", false, true),
    ] {
        let (f, g) = (format!("src/{folder}/f.ts"), format!("src/{folder}/g.ts"));
        let mut edges = vec![
            ("src/t/t1.ts", f.as_str()),
            ("src/t/t2.ts", f.as_str()),
            ("src/t/t3.ts", f.as_str()),
        ];
        edges.push(if imports {
            (f.as_str(), g.as_str())
        } else {
            (g.as_str(), "src/x/k.ts")
        });
        let doc = scan_edges(name, &edges, &[]);
        assert_eq!(
            !misplaced(&doc).is_empty(),
            reported,
            "{name}: {:#}",
            doc["findings"]
        );
    }
}

/// `n` internal imports in a chain through module `m`'s files k0 .. kn.
fn chain(m: &str, n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| (format!("src/{m}/k{i}.ts"), format!("src/{m}/k{}.ts", i + 1)))
        .collect()
}

#[test]
fn misplaced_file_needs_a_cohesion_gain_of_two_hundredths() {
    // f.ts in a imports a/k0.ts and three files of t import it, each module with `n` internal
    // imports. t, receiving f.ts, goes from n / (n + 3) to (n + 3) / (n + 4): +0.0242 at n = 80,
    // +0.0195 at n = 100, below 0.02.
    for (n, reported) in [(80, true), (100, false)] {
        let mut edges = chain("a", n);
        edges.extend(chain("t", n));
        for (f, t) in [
            ("src/a/f.ts", "src/a/k0.ts"),
            ("src/t/k0.ts", "src/a/f.ts"),
            ("src/t/k1.ts", "src/a/f.ts"),
            ("src/t/k2.ts", "src/a/f.ts"),
        ] {
            edges.push((f.to_owned(), t.to_owned()));
        }
        let edges: Vec<(&str, &str)> = edges
            .iter()
            .map(|(f, t)| (f.as_str(), t.as_str()))
            .collect();
        let doc = scan_edges(&format!("misplaced-gain-{n}"), &edges, &[]);
        let subjects: Vec<&Value> = misplaced(&doc).iter().map(|f| &f["subject"]).collect();
        let expected = if reported {
            vec![json!("src/a/f.ts")]
        } else {
            vec![]
        };
        assert_eq!(subjects, expected.iter().collect::<Vec<_>>(), "n = {n}");
    }
}

#[test]
fn sink_split_moves_need_a_cohesion_gain_of_two_hundredths() {
    // shared serves x with a1.ts and a2.ts, y and z with b1.ts and b2.ts. Moving a1 and a2 into
    // x takes x from n / (n + 2) to 1: +0.0323 at n = 60, +0.0164 at n = 120.
    for (n, moved) in [(60, true), (120, false)] {
        let mut edges = chain("x", n);
        for (f, t) in [
            ("src/x/k0.ts", "src/shared/a1.ts"),
            ("src/x/k1.ts", "src/shared/a2.ts"),
            ("src/y/m.ts", "src/shared/b1.ts"),
            ("src/z/m.ts", "src/shared/b2.ts"),
            ("src/y/m.ts", "src/shared/b2.ts"),
        ] {
            edges.push((f.to_owned(), t.to_owned()));
        }
        let edges: Vec<(&str, &str)> = edges
            .iter()
            .map(|(f, t)| (f.as_str(), t.as_str()))
            .collect();
        let doc = scan_edges(&format!("sink-gain-{n}"), &edges, &["--depth", "1"]);
        let sink = common::finding(&doc, "sink-split", "shared");
        let moves = sink["evidence"]["components"][0]["moves"]
            .as_array()
            .unwrap();
        assert_eq!(!moves.is_empty(), moved, "n = {n}: {sink:#}");
        assert_eq!(sink.get("simulation").is_some(), moved, "n = {n}");
    }
}

#[test]
fn misplaced_file_never_moves_between_a_module_and_its_subfolder() {
    // server/lib/x.ts is pulled by three files of server, the folder above its module; lib/f.ts
    // by three files of lib/typescript, the folder below. Either move only changes folder depth.
    for (name, edges) in [
        (
            "misplaced-to-parent",
            [
                ("src/server/lib/g.ts", "src/server/lib/x.ts"),
                ("src/server/a.ts", "src/server/lib/x.ts"),
                ("src/server/b.ts", "src/server/lib/x.ts"),
                ("src/server/c.ts", "src/server/lib/x.ts"),
            ],
        ),
        (
            "misplaced-to-child",
            [
                ("src/lib/g.ts", "src/lib/f.ts"),
                ("src/lib/typescript/a.ts", "src/lib/f.ts"),
                ("src/lib/typescript/b.ts", "src/lib/f.ts"),
                ("src/lib/typescript/c.ts", "src/lib/f.ts"),
            ],
        ),
    ] {
        let doc = scan_edges(name, &edges, &[]);
        assert_eq!(misplaced(&doc), Vec::<&Value>::new(), "{name}");
    }
}

#[test]
fn misplaced_file_keeps_its_folder_within_the_module() {
    let to = |doc: &Value| {
        misplaced(doc)
            .iter()
            .map(|f| f["suggestion"]["move"]["to"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // a/domain/f.ts is imported by three files of t, which has a domain/ folder: it goes there.
    let edges = |t1: &'static str, t2: &'static str| {
        vec![
            ("src/a/api/h.ts", "src/a/domain/g.ts"),
            ("src/a/domain/g.ts", "src/a/domain/f.ts"),
            (t1, "src/a/domain/f.ts"),
            (t2, "src/a/domain/f.ts"),
            ("src/t/t3.ts", "src/a/domain/f.ts"),
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
            ("src/t/t3.ts", "src/a/ui/f.ts"),
            ("src/t/ui/u.ts", "src/t/ui/v.ts"),
        ],
        &["--rules", &rules],
    );
    assert_eq!(to(&doc), ["src/t/f.ts"]);
}

#[test]
fn misplaced_file_needs_a_move_that_adds_no_forbidden_import_or_dependency() {
    // t1.ts, t2.ts and t3.ts import a/f.ts, g.ts in a does too; f.ts imports x/y.ts and w/v.ts.
    let edges = |t_uses_x_and_w: bool| {
        let mut edges = vec![
            ("src/a/g.ts", "src/a/f.ts"),
            ("src/t/t1.ts", "src/a/f.ts"),
            ("src/t/t2.ts", "src/a/f.ts"),
            ("src/t/t3.ts", "src/a/f.ts"),
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
            ("src/t/t2.ts", "src/a/f.ts"),
            ("src/t/f.ts", "src/a/f.ts"),
        ],
        &[],
    );
    assert_eq!(
        common::finding(&doc, "misplaced-file", "src/a/f.ts"),
        &json!({ "kind": "misplaced-file", "subject": "src/a/f.ts",
                 "evidence": { "own_module_edges": 1, "target_module": "t",
                               "imports_from_target": 3, "via_barrel": false } })
    );
}

#[test]
fn seam_skips_a_layout_by_kind() {
    // Two triangles, all of a and b1 in one folder, b2 and b3 in another (folders Q 0.1111,
    // clusters Q 0.5). Named data/ and ui/ the clusters are a seam; named controllers/ and
    // services/ the codebase is organised by kind, and clusters across kinds restate it.
    for (data, ui, reported) in [("data", "ui", true), ("controllers", "services", false)] {
        let (a, b) = (
            |f: &str| format!("src/{data}/{f}.ts"),
            |f: &str| format!("src/{ui}/{f}.ts"),
        );
        let edges = [
            (a("a1"), a("a2")),
            (a("a1"), a("a3")),
            (a("a3"), a("a2")),
            (b("b2"), a("b1")),
            (b("b2"), b("b3")),
            (a("b1"), b("b3")),
        ];
        let edges: Vec<(&str, &str)> = edges
            .iter()
            .map(|(f, t)| (f.as_str(), t.as_str()))
            .collect();
        let doc = scan_edges(&format!("seam-kind-{data}"), &edges, &[]);
        let seam = doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "seam");
        assert_eq!(seam, reported, "{data}, {ui}");
    }
}

#[test]
fn seam_skips_a_flat_package_of_single_file_modules() {
    // Two triangles with every file directly in src/: six modules of one file each.
    // The clusters beat the folders (0.5 against -0.1667), but with more than half the files in
    // single-file modules the folders say nothing to compare with: no seam.
    let doc = scan_edges(
        "seam-flat",
        &[
            ("src/a1.ts", "src/a2.ts"),
            ("src/a1.ts", "src/a3.ts"),
            ("src/a3.ts", "src/a2.ts"),
            ("src/b2.ts", "src/b1.ts"),
            ("src/b2.ts", "src/b3.ts"),
            ("src/b1.ts", "src/b3.ts"),
        ],
        &[],
    );
    assert!(
        !doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "seam"),
        "{:#}",
        doc["findings"]
    );
}

#[test]
fn seam_counts_files_not_modules_in_single_file_modules() {
    // ts-seams' triangles in data/ and ui/ plus three root files, modules of their own: three of
    // five modules hold one file, but only three of nine files sit in them. The package is not
    // flat, so the seam stands. Each root file imports only its test file, which is excluded.
    let mut edges = vec![
        ("src/ui/a1.ts", "src/data/a2.ts"),
        ("src/ui/a1.ts", "src/ui/a3.ts"),
        ("src/ui/a3.ts", "src/data/a2.ts"),
        ("src/ui/b2.ts", "src/data/b1.ts"),
        ("src/ui/b2.ts", "src/data/b3.ts"),
        ("src/data/b1.ts", "src/data/b3.ts"),
    ];
    edges.extend([
        ("src/r1.ts", "src/r1.test.ts"),
        ("src/r2.ts", "src/r2.test.ts"),
        ("src/r3.ts", "src/r3.test.ts"),
    ]);
    let doc = scan_edges("seam-root-files", &edges, &[]);
    assert_eq!(doc["modules"].as_array().unwrap().len(), 5);
    assert!(
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "seam"),
        "{:#}",
        doc["findings"]
    );
}

#[test]
fn seam_stands_with_exactly_half_the_files_in_single_file_modules() {
    // The triangles of the test above plus six root files: 6 of 12 files are loose,
    // not more than half, and the clusters hold the other half.
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
    for i in 0..6 {
        edges.push((format!("src/r{i}.ts"), format!("src/r{i}.test.ts")));
    }
    let edges: Vec<(&str, &str)> = edges
        .iter()
        .map(|(f, t)| (f.as_str(), t.as_str()))
        .collect();
    let doc = scan_edges("seam-half-loose", &edges, &[]);
    assert!(
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "seam"),
        "{:#}",
        doc["findings"]
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
        let (doc, _, _) = scan(&[
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

fn run_json(args: &[&str]) -> (Value, Option<i32>) {
    let (doc, code) = common::run_json(args);
    (common::evidence_contract(doc), code)
}

fn scan(args: &[&str]) -> (Value, String, String) {
    let (doc, out, err) = common::scan(args);
    (common::evidence_contract(doc), out, err)
}
fn scan_recorded(name: &str, args: &[&str]) -> (Value, String, String) {
    let (doc, out, err) = common::scan_recorded(name, args);
    (common::evidence_contract(doc), out, err)
}
