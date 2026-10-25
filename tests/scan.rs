mod common;

use std::fs;

use common::{archgraph, fixture, layout, stable, synthetic, validate};
use serde_json::{Value, json};

fn files(doc: &Value) -> Vec<&str> {
    doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect()
}

#[test]
fn ts_basic_metrics_by_hand() {
    let (doc, _, stderr) = scan_recorded("ts-basic", &[]);
    assert_eq!(doc["schema"], "archgraph/v1.2");
    assert_eq!(doc["kind"], "scan");
    assert_eq!(doc["source"]["language"], "typescript");
    assert_eq!(
        doc["source"]["collector"],
        json!({ "name": "dependency-cruiser", "version": "18.4.0", "cruised": 8, "unresolved": 1 })
    );
    assert!(stderr.contains("warning: 1 unresolved"), "{stderr}");
    assert_eq!(doc["config"]["depth"], 2);
    assert_eq!(doc["config"]["source_root"], "src");
    assert_eq!(doc["config"]["include_tests"], false);
    assert_eq!(
        doc["files"],
        json!([
            { "path": "src/app/shell/run.ts", "module": "app/shell", "barrel": false },
            { "path": "src/app/shell/view.ts", "module": "app/shell", "barrel": false },
            { "path": "src/core/model/store.ts", "module": "core/model", "barrel": false },
            { "path": "src/core/model/types.ts", "module": "core/model", "barrel": false },
            { "path": "src/core/util/log.ts", "module": "core/util", "barrel": false },
            { "path": "src/main.ts", "module": "main.ts", "barrel": false }
        ])
    );
    // main.ts reaches run.ts through the `@/` tsconfig alias; run.ts -> types.ts is `import type`;
    // store.ts imports types.ts both as a type and as a value, so the pair is static.
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([
            { "from": "src/app/shell/run.ts", "to": "src/core/model/store.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/app/shell/run.ts", "to": "src/core/model/types.ts", "kind": "type", "weight": 0.1, "verdict": null, "rule": null },
            { "from": "src/app/shell/run.ts", "to": "src/core/util/log.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/app/shell/view.ts", "to": "src/app/shell/run.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/app/shell/view.ts", "to": "src/core/util/log.ts", "kind": "lazy", "weight": 0.3, "verdict": null, "rule": null },
            { "from": "src/core/model/store.ts", "to": "src/core/model/types.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/core/model/store.ts", "to": "src/core/util/log.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/main.ts", "to": "src/app/shell/run.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null }
        ])
    );
    assert_eq!(
        doc["modules"],
        json!([
            { "id": "app/shell", "files": 2, "ca": 1, "ce": 2, "instability": 0.6667,
              "internal_edges": 1, "outgoing_edges": 4, "cohesion": 0.2941,
              "outgoing_to": { "core/model": 2, "core/util": 2 }, "weighted_ca": 1.0, "weighted_ce": 2.0, "weighted_internal_edges": 1.0, "weighted_outgoing_edges": 2.4, "weighted_outgoing_to": { "core/model": 1.1, "core/util": 1.3 } },
            { "id": "core/model", "files": 2, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 1, "outgoing_edges": 1, "cohesion": 0.5,
              "outgoing_to": { "core/util": 1 }, "weighted_ca": 1.0, "weighted_ce": 1.0, "weighted_internal_edges": 1.0, "weighted_outgoing_edges": 1.0, "weighted_outgoing_to": { "core/util": 1.0 } },
            { "id": "core/util", "files": 1, "ca": 2, "ce": 0, "instability": 0.0,
              "internal_edges": 0, "outgoing_edges": 0, "cohesion": null, "outgoing_to": {}, "weighted_ca": 2.0, "weighted_ce": 0.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 0.0, "weighted_outgoing_to": {} },
            { "id": "main.ts", "files": 1, "ca": 0, "ce": 1, "instability": 1.0,
              "internal_edges": 0, "outgoing_edges": 1, "cohesion": 0.0,
              "outgoing_to": { "app/shell": 1 }, "weighted_ca": 0.0, "weighted_ce": 1.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 1.0, "weighted_outgoing_to": { "app/shell": 1.0 } }
        ])
    );
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": "app/shell", "to": "core/model", "imports": 2, "forbidden": 0 },
            { "from": "app/shell", "to": "core/util", "imports": 2, "forbidden": 0 },
            { "from": "core/model", "to": "core/util", "imports": 1, "forbidden": 0 },
            { "from": "main.ts", "to": "app/shell", "imports": 1, "forbidden": 0 }
        ])
    );
    assert_eq!(doc["cycles"], json!([]));
    assert_eq!(doc["config"]["rules"], Value::Null);
    assert_eq!(doc["cochange"], Value::Null);

    assert_eq!(
        doc["summary"],
        summary(
            0,
            json!({
            "app/shell": m(json!(0.2941), 1, 2, json!(0.6667)),
            "core/model": m(json!(0.5), 1, 1, json!(0.5)),
            "core/util": m(Value::Null, 2, 0, json!(0.0)),
            "main.ts": m(json!(0.0), 0, 1, json!(1.0)) })
        )
    );

    // run.ts imports two files of core/model, but core/model never imports it: not misplaced.
    // log.ts is the only file of core/util. Label propagation puts all six files in one cluster:
    // no seam.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 6, "modules": 4 } }
        ])
    );
}

/// `{ cohesion, ca, ce, instability }` of one module in a Summary.
fn m(cohesion: Value, ca: usize, ce: usize, instability: Value) -> Value {
    json!({ "cohesion": cohesion, "ca": ca, "ce": ce, "instability": instability, "weighted_ca": ca as f64, "weighted_ce": ce as f64 })
}

/// A Summary without rules: nothing forbidden or unplanned.
fn summary(cycles: usize, modules: Value) -> Value {
    json!({ "forbidden": 0, "unplanned": 0, "cycles": cycles, "type_only_cycles": 0, "modules": modules })
}

#[test]
fn schema_is_valid_draft_2020_12() {
    let schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.2.schema.json")).unwrap();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    jsonschema::draft202012::meta::validate(&schema).unwrap();
}

#[test]
fn ts_basic_output_is_byte_stable_and_location_free() {
    let (_, first, _) = scan_recorded("ts-basic", &[]);
    let (_, second, _) = scan_recorded("ts-basic", &[]);
    assert_eq!(stable(&first), stable(&second));

    // Relocation preserves graph data; the Git-relative workspace anchor changes by design.
    let copy = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ts-basic-copy");
    let _ = fs::remove_dir_all(&copy);
    copy_dir(&fixture("ts-basic"), &copy);
    let input = copy.join("depcruise.json");
    let (_, moved, _) = scan(&[
        copy.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
    ]);
    let graph_data = |text: &str| {
        let mut doc: Value = serde_json::from_str(text).unwrap();
        let source = doc["source"].as_object_mut().unwrap();
        source.remove("generated_at");
        if let Some(anchor) = source.remove("scan_root") {
            assert!(std::path::Path::new(anchor.as_str().unwrap()).is_relative());
        }
        doc
    };
    assert_eq!(graph_data(&first), graph_data(&moved));
}

#[test]
fn stable_fixture_comparison_ignores_provenance_but_preserves_graph_changes() {
    let first = json!({"source":{"generated_at":"first","scan_root":"original"},"edges":[{"from":"a.ts","to":"b.ts"}]}).to_string();
    let mut copied: Value = serde_json::from_str(&first).unwrap();
    copied["source"]["generated_at"] = json!("second");
    copied["source"]["scan_root"] = json!("copied");
    assert_eq!(stable(&first), stable(&copied.to_string()));
    copied["edges"][0]["to"] = json!("changed.ts");
    assert_ne!(stable(&first), stable(&copied.to_string()));
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn ts_cycles_with_minimum_break_sets() {
    let (doc, _, _) = scan_recorded("ts-cycles", &[]);
    // a->b and b->c carry 2 imports each, c->a carries 1; d->e carries 2, e->d 1: an even pair,
    // both sides tiny, listed in the names' order with no lighter side.
    // Inside f, p->q is the only single import whose removal breaks both p<->q and p->q->r->p.
    // The a/b/c and d/e file cycles cross modules, so only the module cycles report them.
    assert_eq!(
        doc["cycles"],
        json!([
            { "level": "module", "members": ["a", "b", "c"], "exact": true, "grouping": false,
              "break": [{ "from": "src/c/z.ts", "to": "src/a/x.ts" }], "two_way": [] },
            { "level": "module", "members": ["d", "e"], "exact": true, "grouping": false,
              "break": [{ "from": "src/e/n.ts", "to": "src/d/m.ts" }],
              "two_way": [{ "from": "d", "to": "e", "imports": 2, "reverse_imports": 1, "even": true,
                            "edges": [{ "from": "src/d/m.ts", "to": "src/e/n.ts" },
                                      { "from": "src/d/m2.ts", "to": "src/e/n.ts" }] }] },
            { "level": "file", "members": ["src/f/p.ts", "src/f/q.ts", "src/f/r.ts"], "exact": true,
              "break": [{ "from": "src/f/p.ts", "to": "src/f/q.ts" }] }
        ])
    );
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": "a", "to": "b", "imports": 2, "forbidden": 0 },
            { "from": "b", "to": "c", "imports": 2, "forbidden": 0 },
            { "from": "c", "to": "a", "imports": 1, "forbidden": 0 },
            { "from": "d", "to": "e", "imports": 2, "forbidden": 0 },
            { "from": "e", "to": "d", "imports": 1, "forbidden": 0 }
        ])
    );

    let findings = doc["findings"].as_array().unwrap();
    // One cycle finding per reported cycle, the cycle itself as evidence.
    let subjects = ["a, b, c", "d, e", "src/f/p.ts, src/f/q.ts, src/f/r.ts"];
    let expected: Vec<Value> = doc["cycles"]
        .as_array()
        .unwrap()
        .iter()
        .zip(subjects)
        .map(|(c, s)| json!({ "kind": "cycle", "subject": s, "evidence": c }))
        .collect();
    assert_eq!(findings[..3], expected[..]);
    // y.ts is imported by two files of a and by none of b. m.ts is imported by only one file of
    // e, and z and n are alone in their modules: only y is misplaced.
    let misplaced: Vec<(&Value, &Value, &Value)> = findings
        .iter()
        .filter(|f| f["kind"] == "misplaced-file")
        .map(|f| {
            (
                &f["subject"],
                &f["evidence"],
                &f["suggestion"]["move"]["to"],
            )
        })
        .collect();
    assert_eq!(
        misplaced,
        [(
            &json!("src/b/y.ts"),
            &json!({ "own_module_edges": 0, "target_module": "a", "imports_from_target": 2,
                     "via_barrel": false }),
            &json!("src/a/y.ts")
        )]
    );
    // y.ts into a: the a -> b -> c cycle becomes a <-> c (still 3 cycles), w -> y and x -> y turn
    // internal, b keeps only v -> z and c gains a as importer.
    let sim = &findings
        .iter()
        .find(|f| f["subject"] == "src/b/y.ts")
        .unwrap()["simulation"];
    assert_eq!(doc["summary"]["cycles"], 3);
    assert_eq!(
        sim["after"],
        summary(
            3,
            json!({ "a": m(json!(0.6667), 1, 1, json!(0.5)),
                    "b": m(json!(0.0), 0, 1, json!(1.0)),
                    "c": m(json!(0.0), 2, 1, json!(0.3333)) })
        )
    );

    // Q(clusters) = 0.25 + 0.16 + 0.21; Q(folders) = 0.21 - 42/400.
    let seam = findings.iter().find(|f| f["kind"] == "seam").unwrap();
    assert_eq!(
        seam["evidence"],
        json!({ "modularity": 0.62, "folder_modularity": 0.105, "hubs": [], "via_barrel": false,
                "clusters": [
                    { "files": ["src/a/w.ts", "src/a/x.ts", "src/b/v.ts", "src/b/y.ts", "src/c/z.ts"],
                      "modules": ["a", "b", "c"] },
                    { "files": ["src/d/m.ts", "src/d/m2.ts", "src/e/n.ts"], "modules": ["d", "e"] },
                    { "files": ["src/f/p.ts", "src/f/q.ts", "src/f/r.ts"], "modules": ["f"] } ] })
    );
    assert_eq!(
        findings.last().unwrap(),
        &json!({ "kind": "small-repo", "subject": ".", "evidence": { "files": 11, "modules": 6 } })
    );
    assert_eq!(findings.len(), 3 + 1 + 1 + 1);
}

#[test]
fn ts_excludes_and_barrel_attribution() {
    let (doc, _, _) = scan_recorded("ts-excludes", &[]);
    assert_eq!(
        doc["files"],
        json!([
            { "path": "src/app/main.ts", "module": "app", "barrel": false },
            { "path": "src/feature/a.ts", "module": "feature", "barrel": false },
            { "path": "src/feature/b.ts", "module": "feature", "barrel": false },
            { "path": "src/feature/index.ts", "module": "feature", "barrel": true },
            { "path": "src/feature/kinds.ts", "module": "feature", "barrel": false },
            { "path": "src/shared/util.ts", "module": "shared", "barrel": false }
        ])
    );
    // `export type { Kind } from "./kinds"` is a type-only re-export: a re-export, so index.ts
    // stays a barrel, but not a runtime import.
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([
            { "from": "src/app/main.ts", "to": "src/feature/a.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/app/main.ts", "to": "src/feature/b.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/app/main.ts", "to": "src/shared/util.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/feature/b.ts", "to": "src/shared/util.ts", "kind": "runtime", "weight": 1.0, "verdict": null, "rule": null },
            { "from": "src/feature/index.ts", "to": "src/feature/a.ts", "kind": "reexport", "weight": 0.0, "is_reexport": true, "verdict": null, "rule": null },
            { "from": "src/feature/index.ts", "to": "src/feature/b.ts", "kind": "reexport", "weight": 0.0, "is_reexport": true, "verdict": null, "rule": null },
            { "from": "src/feature/index.ts", "to": "src/feature/kinds.ts", "kind": "type", "weight": 0.1, "is_reexport": true, "verdict": null, "rule": null }
        ])
    );
    // Named imports a and b resolve to their definitions; the unused Kind export adds no edge.
    // The barrel's own re-exports are dropped,
    // so feature has no internal edges (cohesion 0.0, not 3/4).
    assert_eq!(
        doc["modules"],
        json!([
            { "id": "app", "files": 1, "ca": 0, "ce": 2, "instability": 1.0,
              "internal_edges": 0, "outgoing_edges": 3, "cohesion": 0.0,
              "outgoing_to": { "feature": 2, "shared": 1 }, "weighted_ca": 0.0, "weighted_ce": 2.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 3.0, "weighted_outgoing_to": { "feature": 2.0, "shared": 1.0 } },
            { "id": "feature", "files": 4, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 0, "outgoing_edges": 1, "cohesion": 0.0,
              "outgoing_to": { "shared": 1 }, "weighted_ca": 1.0, "weighted_ce": 1.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 1.0, "weighted_outgoing_to": { "shared": 1.0 } },
            { "id": "shared", "files": 1, "ca": 2, "ce": 0, "instability": 0.0,
              "internal_edges": 0, "outgoing_edges": 0, "cohesion": null, "outgoing_to": {}, "weighted_ca": 2.0, "weighted_ce": 0.0, "weighted_internal_edges": 0.0, "weighted_outgoing_edges": 0.0, "weighted_outgoing_to": {} }
        ])
    );
    // Module edges count canonical file dependencies after resolving named barrel imports.
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": "app", "to": "feature", "imports": 2, "forbidden": 0 },
            { "from": "app", "to": "shared", "imports": 1, "forbidden": 0 },
            { "from": "feature", "to": "shared", "imports": 1, "forbidden": 0 }
        ])
    );
    // main.ts imports exactly a.ts and b.ts through index.ts, but main.ts is the
    // only file of app, so it is not misplaced.
    assert_eq!(
        doc["findings"],
        json!([{ "kind": "small-repo", "subject": ".", "evidence": { "files": 6, "modules": 3 } }])
    );
    let excludes = doc["config"]["excludes"].as_array().unwrap();
    assert!(
        excludes.contains(&json!("**/*.test.*")) && excludes.contains(&json!("**/generated/**"))
    );

    let (with_tests, _, _) = scan_recorded("ts-excludes", &["--include-tests"]);
    assert_eq!(with_tests["config"]["include_tests"], true);
    assert!(
        !with_tests["config"]["excludes"]
            .as_array()
            .unwrap()
            .contains(&json!("**/*.test.*"))
    );
    let paths = files(&with_tests);
    assert!(paths.contains(&"src/feature/a.test.ts") && paths.contains(&"src/feature/b.spec.ts"));
    assert!(!paths.contains(&"src/generated/api.ts"));

    let (everything, _, _) = scan_recorded("ts-excludes", &["--no-default-excludes"]);
    assert_eq!(everything["config"]["excludes"], json!([]));
    assert_eq!(files(&everything).len(), 11);

    let (no_app, _, _) = scan_recorded("ts-excludes", &["--exclude", "src/app/**"]);
    assert_eq!(
        no_app["config"]["excludes"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "src/app/**"
    );
    assert!(!files(&no_app).contains(&"src/app/main.ts"));
}

#[test]
fn greedy_break_set_above_sixteen_module_edges() {
    // Five modules importing each other in both directions: 20 module edges, one SCC.
    let mut edges = Vec::new();
    for i in 0..5 {
        for j in 0..5 {
            if i != j {
                edges.push((format!("m{i}/f.ts"), format!("m{j}/f.ts")));
            }
        }
    }
    let (dir, input) = synthetic("greedy", 5, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let cycles = doc["cycles"].as_array().unwrap();
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0]["members"], json!(["m0", "m1", "m2", "m3", "m4"]));
    assert_eq!(cycles[0]["exact"], false);
    // What remains must be acyclic: one direction of every pair (a tournament) whose
    // out-degrees are 0, 1, 2, 3, 4, which is exactly the acyclic tournament.
    let removed: Vec<(String, String)> = cycles[0]["break"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap().to_owned(),
                e["to"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for (from, to) in &edges {
        let reverse = (to.clone(), from.clone());
        assert_ne!(
            removed.contains(&(from.clone(), to.clone())),
            removed.contains(&reverse)
        );
    }
    let mut out_degrees: Vec<usize> = (0..5)
        .map(|i| {
            let from = format!("m{i}/f.ts");
            edges
                .iter()
                .filter(|e| e.0 == from && !removed.contains(e))
                .count()
        })
        .collect();
    out_degrees.sort();
    assert_eq!(out_degrees, [0, 1, 2, 3, 4]);
}

#[test]
fn zero_cruised_files_is_an_error() {
    let (dir, input) = synthetic("empty", 0, &[]);
    let out = archgraph(&[
        "scan",
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cruised 0 files"));
}

/// Runs git in `dir` with a throwaway identity and no signing.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Writes each file with fresh content (its path and `round`) and commits them.
fn commit(dir: &std::path::Path, round: usize, files: &[&str]) {
    for f in files {
        let path = dir.join(f);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("// {f} {round}\n")).unwrap();
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", &format!("round {round}")]);
}

#[test]
fn cochange_finds_hidden_coupling_and_records_git() {
    let repo = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cochange-repo");
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    let (x, w, y, z) = ("src/a/x.ts", "src/a/w.ts", "src/b/y.ts", "src/c/z.ts");
    // x and y change together three times, once while y was still src/b/old.ts.
    commit(&repo, 1, &[x, w, "src/b/old.ts", z]);
    git(&repo, &["mv", "src/b/old.ts", y]);
    git(&repo, &["commit", "-q", "-m", "rename"]);
    commit(&repo, 2, &[x, y]);
    commit(&repo, 3, &[x, y, w]);
    // y and z change together three times but y imports z: no hidden coupling.
    commit(&repo, 4, &[y, z]);
    commit(&repo, 5, &[y, z]);
    // x and z: two ordinary commits plus a 31-file commit, which is ignored.
    commit(&repo, 6, &[x, z, w]);
    let bulk: Vec<String> = (0..29).map(|i| format!("src/bulk/f{i}.ts")).collect();
    let mut big: Vec<&str> = bulk.iter().map(String::as_str).collect();
    big.extend([x, z]);
    assert_eq!(big.len(), 31);
    commit(&repo, 7, &big);
    // x and w (same module) also change together three times; same-module pairs are not findings.

    let edges = [(y.to_owned(), z.to_owned()), (w.to_owned(), x.to_owned())];
    // The collector input lives outside the repository, so the tree stays clean.
    let (_, input) = synthetic("cochange-input", 4, &edges);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let root = repo.to_str().unwrap();
    let (doc, _, _) = scan(&[root, "--collector-input", &input, "--cochange"]);

    assert_eq!(
        doc["source"]["git"],
        json!({ "commit": head, "dirty": false })
    );
    // 8 commits: 7 read, the 31-file one skipped. Pairs with >= 3 co-changes, any module.
    assert_eq!(
        doc["cochange"],
        json!({ "limit": 300, "commits": 7, "skipped": 1, "pairs": [
            { "files": [w, x], "commits": 3 },
            { "files": [x, y], "commits": 3 },
            { "files": [y, z], "commits": 3 } ] })
    );
    let hidden: Vec<&Value> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "hidden-coupling")
        .collect();
    assert_eq!(
        hidden,
        [
            &json!({ "kind": "hidden-coupling", "subject": "src/a/x.ts <-> src/b/y.ts",
                  "evidence": { "files": [x, y], "modules": ["a", "b"], "commits": 3 } })
        ]
    );

    // Archgraph's own outputs in the repository do not make it dirty on the next scan.
    fs::create_dir_all(repo.join(".archgraph")).unwrap();
    fs::write(repo.join(".archgraph/archgraph.json"), "{}").unwrap();
    let output = repo.join("graph.json");
    for _ in 0..2 {
        let out = archgraph(&[
            "scan",
            root,
            "--collector-input",
            &input,
            "-o",
            output.to_str().unwrap(),
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let doc: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(doc["source"]["git"]["dirty"], false);
    }

    // --cochange=N reads only the last N commits; an untracked file makes the tree dirty.
    fs::write(repo.join("untracked.txt"), "").unwrap();
    let (doc, _, _) = scan(&[root, "--collector-input", &input, "--cochange=3"]);
    assert_eq!(doc["source"]["git"]["dirty"], true);
    assert_eq!(doc["cochange"]["limit"], 3);
    assert_eq!(doc["cochange"]["commits"], 2);
    assert_eq!(doc["cochange"]["skipped"], 1);
    assert_eq!(doc["cochange"]["pairs"], json!([]));

    // Without --cochange nothing is read.
    let (doc, _, _) = scan(&[root, "--collector-input", &input]);
    assert_eq!(doc["cochange"], Value::Null);
}

#[test]
fn cochange_reads_only_commits_under_the_root_and_matches_quoted_paths() {
    let repo = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cochange-subdir-repo");
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    let (x, q) = ("src/a/x.ts", "src/b/q\"t.ts");
    let in_app = ["app/src/a/x.ts", "app/src/b/q\"t.ts"];
    for round in 1..=3 {
        commit(&repo, round, &in_app);
    }
    // Newer commits outside the root and a merge must not use up --cochange=3.
    git(&repo, &["checkout", "-q", "-b", "side"]);
    commit(&repo, 4, &["other/z.ts"]);
    git(&repo, &["checkout", "-q", "-"]);
    commit(&repo, 5, &["other/w.ts"]);
    git(&repo, &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    let (_, input) = synthetic("cochange-subdir-input", 2, &[(x.to_owned(), q.to_owned())]);
    let root = repo.join("app");
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--collector-input",
        &input,
        "--cochange=3",
    ]);
    assert_eq!(
        doc["cochange"],
        json!({ "limit": 3, "commits": 3, "skipped": 0, "pairs": [
            { "files": [x, q], "commits": 3 } ] })
    );
}

/// Edge pairs from a cycle's `break` list.
fn links(cycle: &Value) -> Vec<(String, String)> {
    cycle["break"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap().to_owned(),
                e["to"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Kahn's algorithm over `edges` without `removed`.
fn acyclic_without(edges: &[(String, String)], removed: &[(String, String)]) -> bool {
    let kept: Vec<&(String, String)> = edges.iter().filter(|e| !removed.contains(e)).collect();
    let mut nodes: Vec<&str> = edges
        .iter()
        .flat_map(|(f, t)| [f.as_str(), t.as_str()])
        .collect();
    nodes.sort();
    nodes.dedup();
    let mut indegree: std::collections::HashMap<&str, usize> =
        nodes.iter().map(|n| (*n, 0)).collect();
    for (_, t) in &kept {
        *indegree.get_mut(t.as_str()).unwrap() += 1;
    }
    let mut ready: Vec<&str> = nodes.iter().copied().filter(|n| indegree[n] == 0).collect();
    let mut seen = 0;
    while let Some(n) = ready.pop() {
        seen += 1;
        for (f, t) in &kept {
            if f == n {
                let d = indegree.get_mut(t.as_str()).unwrap();
                *d -= 1;
                if *d == 0 {
                    ready.push(t);
                }
            }
        }
    }
    seen == nodes.len()
}

#[test]
fn greedy_break_set_leaves_an_acyclic_graph() {
    let mut edges = Vec::new();
    for i in 0..5 {
        for j in 0..5 {
            if i != j {
                edges.push((format!("m{i}/f.ts"), format!("m{j}/f.ts")));
            }
        }
    }
    let (dir, input) = synthetic("greedy-acyclic", 5, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert!(acyclic_without(&edges, &links(&doc["cycles"][0])));
}

#[test]
fn exact_break_set_up_to_sixteen_module_edges_greedy_at_seventeen() {
    for (n, exact) in [(16, true), (17, false)] {
        // A ring of n one-file modules: one SCC with exactly n module edges.
        let edges: Vec<(String, String)> = (0..n)
            .map(|i| (format!("m{i:02}/f.ts"), format!("m{:02}/f.ts", (i + 1) % n)))
            .collect();
        let (dir, input) = synthetic(&format!("ring-{n}"), n, &edges);
        let (doc, _, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        let cycles = doc["cycles"].as_array().unwrap();
        assert_eq!(cycles.len(), 1, "{n}");
        assert_eq!(cycles[0]["members"].as_array().unwrap().len(), n);
        assert_eq!(cycles[0]["exact"], exact, "{n}");
        let removed = links(&cycles[0]);
        assert_eq!(removed.len(), 1, "{n}");
        assert!(acyclic_without(&edges, &removed), "{n}");
    }
}

#[test]
fn file_cycle_inside_a_module_is_reported_alongside_a_module_cycle() {
    // x/a <-> x/b is a cycle inside x; x/a -> y/c -> x/b also puts all three in one file SCC
    // that crosses modules.
    let edges: Vec<(String, String)> = [
        ("x/a.ts", "x/b.ts"),
        ("x/b.ts", "x/a.ts"),
        ("x/a.ts", "y/c.ts"),
        ("y/c.ts", "x/b.ts"),
    ]
    .iter()
    .map(|(f, t)| (f.to_string(), t.to_string()))
    .collect();
    let (dir, input) = synthetic("file-cycle-in-module", 3, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(
        doc["cycles"],
        json!([
            { "level": "module", "members": ["x", "y"], "exact": true, "grouping": false,
              "break": [{ "from": "x/a.ts", "to": "y/c.ts" }],
              "two_way": [{ "from": "x", "to": "y", "imports": 1, "reverse_imports": 1, "even": true,
                            "edges": [{ "from": "x/a.ts", "to": "y/c.ts" }] }] },
            { "level": "file", "members": ["x/a.ts", "x/b.ts"], "exact": true,
              "break": [{ "from": "x/a.ts", "to": "x/b.ts" }] }
        ])
    );
}

#[test]
fn cycles_follow_runtime_imports_only() {
    // a -> b and c/f -> c/g are static; the back edges decide. Type-only imports and re-exports
    // and dynamic imports do not run at load time, so they close no cycle, though module_edges
    // still count them. Pure re-exports are structural and also close no runtime cycle.
    let import: &[&str] = &["local", "import"];
    for (name, back, cyclic) in [
        (
            "runtime-cycle-type",
            &["local", "type-only", "import"][..],
            false,
        ),
        (
            "runtime-cycle-dynamic",
            &["local", "dynamic-import"][..],
            false,
        ),
        (
            "runtime-cycle-type-export",
            &["local", "type-only", "export"][..],
            false,
        ),
        ("runtime-cycle-export", &["local", "export"][..], false),
        ("runtime-cycle-static", import, true),
    ] {
        let (dir, input) = common::synthetic_typed(
            name,
            4,
            &[
                ("a/x.ts", "b/y.ts", import),
                ("b/y.ts", "a/x.ts", back),
                ("c/f.ts", "c/g.ts", import),
                ("c/g.ts", "c/f.ts", back),
            ],
        );
        let (doc, _, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        let members: Vec<&Value> = doc["cycles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| &c["members"])
            .collect();
        let expected = if cyclic {
            vec![json!(["a", "b"]), json!(["c/f.ts", "c/g.ts"])]
        } else {
            vec![]
        };
        assert_eq!(members, expected.iter().collect::<Vec<_>>(), "{name}");
        assert_eq!(doc["summary"]["cycles"], expected.len(), "{name}");
        assert_eq!(
            doc["summary"]["type_only_cycles"],
            if cyclic { 0 } else { 2 },
            "{name}"
        );
        assert_eq!(
            doc["module_edges"],
            json!([
                { "from": "a", "to": "b", "imports": 1, "forbidden": 0 },
                { "from": "b", "to": "a", "imports": 1, "forbidden": 0 }
            ]),
            "{name}"
        );
    }
}

#[test]
fn break_sets_and_two_way_pairs_count_runtime_imports_only() {
    // a -> b: one static and one type-only import; b -> a: three static. Over runtime imports
    // a -> b weighs 1, so it is the break and the lighter side, with only the static import.
    let import: &[&str] = &["local", "import"];
    let (dir, input) = common::synthetic_typed(
        "runtime-break",
        5,
        &[
            ("a/x.ts", "b/y.ts", import),
            ("a/x2.ts", "b/y.ts", &["local", "type-only", "import"]),
            ("b/y.ts", "a/x.ts", import),
            ("b/y2.ts", "a/x.ts", import),
            ("b/y3.ts", "a/x.ts", import),
        ],
    );
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(
        doc["cycles"],
        json!([{ "level": "module", "members": ["a", "b"], "exact": true, "grouping": false,
                 "break": [{ "from": "a/x.ts", "to": "b/y.ts" }],
                 "two_way": [{ "from": "a", "to": "b", "imports": 1, "reverse_imports": 3,
                               "edges": [{ "from": "a/x.ts", "to": "b/y.ts" }] }] }])
    );
}

#[test]
fn package_json_entries_map_to_scanned_sources() {
    // Output paths (dist/, lib/, .mjs, .d.ts) map back to the sources they are built from; a
    // wildcard export and package.json itself map to nothing.
    let (dir, input) = synthetic(
        "pkg-entries",
        5,
        &[
            ("src/index.ts", "src/a.ts"),
            ("src/internal.ts", "src/a.ts"),
            ("src/adapters/http/index.ts", "src/a.ts"),
            ("src/sub/main.ts", "src/a.ts"),
        ]
        .map(|(f, t)| (f.to_owned(), t.to_owned())),
    );
    let root = std::path::Path::new(&dir);
    fs::create_dir_all(root.join("src/sub")).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{ "main": "./dist/index.cjs", "types": "./dist/index.d.ts",
             "exports": { ".": { "import": { "types": "./dist/index.d.mts", "default": "./dist/index.mjs" } },
                          "./internal": "./internal.js",
                          "./adapters/http": "./dist/adapters/http/index.mjs",
                          "./package.json": "./package.json", "./*": "./*.js" } }"#,
    )
    .unwrap();
    fs::write(
        root.join("src/sub/package.json"),
        r#"{ "main": "./lib/main.js" }"#,
    )
    .unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(
        doc["package_entries"],
        json!([
            "src/adapters/http/index.ts",
            "src/index.ts",
            "src/internal.ts",
            "src/sub/main.ts"
        ])
    );
}

#[test]
fn a_cycle_is_never_cut_at_an_entry_re_export() {
    // a's barrel re-exports b/x.ts (1 import), two files of b import a/k.ts. When package.json
    // publishes the barrel, the re-export is public API: the cut goes to b -> a. An internal
    // barrel re-exporting another module's file is itself the cut, though the pair is small.
    // Explicit comparison mode preserves this cut-selection regression for structural re-exports.
    let import: &[&str] = &["local", "import"];
    for (name, published) in [("entry-cut-published", true), ("entry-cut-internal", false)] {
        let (dir, input) = common::synthetic_typed(
            name,
            6,
            &[
                ("a/index.ts", "b/x.ts", &["local", "export"]),
                ("b/y.ts", "a/k.ts", import),
                ("b/z.ts", "a/k.ts", import),
            ],
        );
        let package = std::path::Path::new(&dir).join("package.json");
        if published {
            fs::write(&package, r#"{ "main": "./dist/a/index.js" }"#).unwrap();
        } else {
            let _ = fs::remove_file(&package);
        }
        let (doc, _, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
            "--all-kinds",
        ]);
        let expected = if published {
            json!([{ "level": "module", "members": ["a", "b"], "exact": true, "grouping": true,
                     "break": [{ "from": "b/y.ts", "to": "a/k.ts" }, { "from": "b/z.ts", "to": "a/k.ts" }],
                     "two_way": [{ "from": "b", "to": "a", "imports": 2, "reverse_imports": 1,
                                   "edges": [{ "from": "b/y.ts", "to": "a/k.ts" },
                                             { "from": "b/z.ts", "to": "a/k.ts" }] }] }])
        } else {
            json!([{ "level": "module", "members": ["a", "b"], "exact": true, "grouping": true,
                     "break": [{ "from": "a/index.ts", "to": "b/x.ts" }],
                     "two_way": [{ "from": "a", "to": "b", "imports": 1, "reverse_imports": 2,
                                   "edges": [{ "from": "a/index.ts", "to": "b/x.ts" }] }] }])
        };
        assert_eq!(doc["cycles"], expected, "{name}");
    }
}

#[test]
fn a_python_package_init_re_export_is_never_the_cut() {
    // fapi/__init__.py re-exports fapi.app.a (the package's public API); app's two files import
    // telemetry, whose _asgi.py imports the package for its version. The break leaves the
    // re-export alone and takes telemetry -> __init__.py.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("py-init-entry");
    let _ = fs::remove_dir_all(&root);
    for f in [
        "fapi/__init__.py",
        "fapi/app/__init__.py",
        "fapi/app/a.py",
        "fapi/app/b.py",
        "fapi/telemetry/__init__.py",
        "fapi/telemetry/_asgi.py",
    ] {
        fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
        fs::write(root.join(f), "").unwrap();
    }
    let imports = [
        ("fapi", "fapi.app.a"),
        ("fapi.app.a", "fapi.telemetry._asgi"),
        ("fapi.app.b", "fapi.telemetry._asgi"),
        ("fapi.telemetry._asgi", "fapi"),
    ];
    let dump = json!({ "grimp_version": "3.17",
        "modules": ["fapi", "fapi.app", "fapi.app.a", "fapi.app.b", "fapi.telemetry", "fapi.telemetry._asgi"],
        "imports": imports.iter().map(|(a, b)| json!({ "importer": a, "imported": b, "kind": "static" }))
            .collect::<Vec<_>>() });
    let input = root.join("grimp.json");
    fs::write(&input, dump.to_string()).unwrap();
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
        "--language",
        "python",
        "--depth",
        "1",
    ]);
    let cycle = doc["cycles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["level"] == "module")
        .unwrap();
    assert_eq!(
        cycle["break"],
        json!([{ "from": "fapi/telemetry/_asgi.py", "to": "fapi/__init__.py" }]),
        "{cycle:#}"
    );
}

#[test]
fn a_failed_type_marking_run_is_recorded() {
    // A depcruise that fails unless pre-compilation dependencies are on: the scan keeps the
    // edges unmarked and says so in source.collector.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("type-marking");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    let recorded = fs::read_to_string(fixture("ts-elided").join("depcruise.json")).unwrap();
    fs::write(dir.join("out.json"), &recorded).unwrap();
    let script = dir.join("depcruise");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = --config ] && config=$2; shift; done\n\
             grep -q '\"tsPreCompilationDeps\": true' \"$config\" || {{ echo crash >&2; exit 1; }}\n\
             cat {}\n",
            dir.join("out.json").display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let (doc, _, stderr) = scan(&[
        dir.to_str().unwrap(),
        "--language",
        "typescript",
        "--depcruise",
        script.to_str().unwrap(),
    ]);
    assert_eq!(doc["source"]["collector"]["type_marking"], "unavailable");
    assert!(stderr.contains("imports of types only"), "{stderr}");
}

#[test]
fn a_tool_running_past_the_tool_timeout_is_killed_and_reported() {
    // Keep repository/Git probes outside the one-second child-process timeout check.
    let fixture = boundrail::collect::TempDir::new("tool-timeout").unwrap();
    let dir = fixture.path();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
    let pid = dir.join("pid");
    let script = dir.join("depcruise");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --probe ]; then printf ready; exit 0; fi\necho $$ > '{}'\nexec sleep 30\n",
            pid.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    // macOS may delay the first execution of a newly written script beyond the test deadline.
    let probe = std::process::Command::new(&script)
        .arg("--probe")
        .output()
        .unwrap();
    assert!(probe.status.success(), "{probe:?}");
    assert_eq!(probe.stdout, b"ready");
    assert!(!pid.exists(), "the probe must not run the timed branch");
    let started = std::time::Instant::now();
    let out = archgraph(&[
        "--tool-timeout",
        "1",
        "scan",
        dir.to_str().unwrap(),
        "--language",
        "typescript",
        "--depcruise",
        script.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("timed out after 1 s"), "{stderr}");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    let pid = fs::read_to_string(&pid).unwrap_or_else(|error| panic!("{error}: {stderr}"));
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .unwrap();
    assert!(!alive.success(), "the timed-out tool is still running");
}

#[test]
fn a_registrar_side_of_a_two_way_pair_is_not_the_cut() {
    // a/drivers.ts imports five drivers of b (5 imports), each driver imports a/base.ts and
    // b/x.ts does too (6). The lighter side only registers drivers: the pair is cut b -> a.
    // Once b -> a also comes from one registrar, both sides are wiring: a registry pair.
    for (name, registrar_back) in [("registrar-one-side", false), ("registrar-both", true)] {
        let mut edges: Vec<(String, String)> = (1..=5)
            .map(|i| ("a/drivers.ts".to_owned(), format!("b/d{i}.ts")))
            .collect();
        if registrar_back {
            edges.extend((1..=6).map(|i| ("b/reg.ts".to_owned(), format!("a/k{i}.ts"))));
        } else {
            edges.extend((1..=5).map(|i| (format!("b/d{i}.ts"), "a/base.ts".to_owned())));
            edges.push(("b/x.ts".to_owned(), "a/base.ts".to_owned()));
        }
        let (dir, input) = synthetic(name, edges.len() + 2, &edges);
        let (doc, _, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        let pair = &doc["cycles"][0]["two_way"][0];
        let got = (
            &pair["from"],
            &pair["to"],
            &pair["imports"],
            pair.get("registry"),
        );
        if registrar_back {
            assert_eq!(
                got,
                (&json!("a"), &json!("b"), &json!(5), Some(&json!(true))),
                "{name}"
            );
        } else {
            assert_eq!(got, (&json!("b"), &json!("a"), &json!(6), None), "{name}");
        }
    }
}

#[test]
fn build_output_is_excluded_at_every_depth_and_can_be_included_except_when_ignored() {
    let (dir, input) = synthetic(
        "build-output",
        4,
        &[
            ("src/a.ts", "src/build/x.ts"),
            ("src/a.ts", "dist/y.ts"),
            ("src/a.ts", "src/gen/build/z.ts"),
        ]
        .map(|(f, t)| (f.to_owned(), t.to_owned())),
    );
    let root = std::path::Path::new(&dir);
    fs::create_dir_all(root.join("src")).unwrap();
    let _ = fs::remove_dir_all(root.join(".git"));
    git(root, &["init", "-q"]);
    fs::write(root.join(".gitignore"), "src/gen/build/\n").unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(files(&doc), ["src/a.ts"]);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--include-generated",
    ]);
    assert_eq!(files(&doc), ["dist/y.ts", "src/a.ts", "src/build/x.ts"]);
}

#[test]
fn a_registrar_move_that_empties_its_module_reports_the_cycle_it_leaves() {
    // p/reg/pick.ts, alone in its module, wires five sibling modules that each import it. Moving
    // it to p/pick/ only renames its module: the 6-module cycle stays, and cycle_after says so.
    let mut edges = Vec::new();
    for m in ["a", "b", "c", "d", "e"] {
        edges.push(("p/reg/pick.ts".to_owned(), format!("p/{m}/x.ts")));
        edges.push((format!("p/{m}/x.ts"), "p/reg/pick.ts".to_owned()));
    }
    let (dir, input) = synthetic("emptying-registrar", edges.len(), &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let moves = &doc["cycles"][0]["registrar_moves"];
    assert_eq!(moves[0]["to"], "p/pick/pick.ts", "{:#}", doc["cycles"]);
    assert_eq!(moves[0]["pairs_removed"], 0);
    assert_eq!(
        (&moves[0]["cycle_before"], &moves[0]["cycle_after"]),
        (&json!(6), &json!(6))
    );
    // A move that removes no pair resolves none: each stays a cut candidate.
    let pairs = doc["cycles"][0]["two_way"].as_array().unwrap();
    assert_eq!(pairs.len(), 5);
    assert!(pairs.iter().all(|p| p.get("resolved_by_move").is_none()));
}

#[test]
fn a_registrar_move_out_of_a_cycle_reports_it_gone_for_its_module() {
    // The fan-out of a_fan_out_registrar_side_is_not_the_cut, plus driver/pg and driver/my
    // importing each other: after the move driver (which keeps base.ts) is in no cycle, so its
    // cycle is gone (0), whatever pg and my still do.
    let mut edges = vec![("app/use.ts".to_owned(), "driver/pick.ts".to_owned())];
    for m in ["pg", "my", "sq", "ms", "or"] {
        edges.push(("driver/pick.ts".to_owned(), format!("driver/{m}/a.ts")));
        for f in ["a", "b", "c"] {
            edges.push((format!("driver/{m}/{f}.ts"), "driver/base.ts".to_owned()));
        }
    }
    edges.push(("driver/pg/b.ts".to_owned(), "driver/my/b.ts".to_owned()));
    edges.push(("driver/my/c.ts".to_owned(), "driver/pg/c.ts".to_owned()));
    let (dir, input) = synthetic("fan-out-leftover", edges.len() + 2, &edges);
    let (default, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(
        default["cycles"][0]["members"],
        json!(["driver/my", "driver/pg"])
    );
    assert_eq!(default["grabbags"].as_array().unwrap().len(), 1);
    // Exercise registrar ranking independently of the grab-bag classifier.
    let rules = common::write_tmp(
        "a_registrar_move_out_of_a_cycle_reports_it_gone_for_its_module-layout.yaml",
        "version: 1\ngrabbag: {cohesion_max: 0.0}\n",
    );
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    let moves = &doc["cycles"][0]["registrar_moves"];
    assert_eq!(moves[0]["file"], "driver/pick.ts", "{:#}", doc["cycles"]);
    assert_eq!(moves[0]["cycle_after"], 0);
}

#[test]
fn a_fan_out_registrar_side_is_not_the_cut() {
    // driver/pick.ts makes one import into each of five sibling modules under driver/, each of
    // which imports driver/base.ts three times. The single imports only pick a driver: every
    // pair is cut driver/<x> -> driver.
    // app/use.ts imports pick.ts, so it is no composition root.
    let mut edges = vec![("app/use.ts".to_owned(), "driver/pick.ts".to_owned())];
    for m in ["pg", "my", "sq", "ms", "or"] {
        edges.push(("driver/pick.ts".to_owned(), format!("driver/{m}/a.ts")));
        for f in ["a", "b", "c"] {
            edges.push((format!("driver/{m}/{f}.ts"), "driver/base.ts".to_owned()));
        }
    }
    let (dir, input) = synthetic("fan-out", edges.len() + 2, &edges);
    let (default, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert!(default["cycles"].as_array().unwrap().is_empty());
    assert_eq!(default["grabbags"].as_array().unwrap().len(), 1);
    // Exercise registrar ranking independently of the grab-bag classifier.
    let rules = common::write_tmp(
        "a_fan_out_registrar_side_is_not_the_cut-layout.yaml",
        "version: 1\ngrabbag: {cohesion_max: 0.0}\n",
    );
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    let pairs: Vec<(&Value, &Value)> = doc["cycles"][0]["two_way"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (&p["to"], &p["imports"]))
        .collect();
    assert_eq!(pairs.len(), 5, "{:#}", doc["cycles"]);
    assert!(
        pairs.iter().all(|p| *p == (&json!("driver"), &json!(3))),
        "{pairs:?}"
    );
    // The move resolves every pair: none is a cut to offer.
    assert!(
        doc["cycles"][0]["two_way"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["resolved_by_move"] == true),
        "{:#}",
        doc["cycles"][0]["two_way"]
    );
    // report prints the move and no cut.
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("report/fan-out");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let (stdout, stderr, _) = report(&[
        root.to_str().unwrap(),
        "--language",
        "typescript",
        "--collector-input",
        &input,
        "--rules",
        &rules,
    ]);
    assert!(
        stdout.contains("move driver/pick.ts to driver/pick/pick.ts: removes 5 two-way pairs"),
        "{stdout}{stderr}"
    );
    assert!(!stdout.contains("lightest two-way cut"), "{stdout}");
    assert!(!stdout.contains("no lighter side to cut"), "{stdout}");
    // pick.ts alone forms the other side of all five pairs: moved to a folder of its own
    // (driver/pick/, a module of its own at depth 2) it takes all five pairs and the cycle away.
    assert_eq!(
        doc["cycles"][0]["registrar_moves"],
        json!([{ "file": "driver/pick.ts", "module": "driver", "pairs": 5,
                 "with": ["driver/ms", "driver/my", "driver/or", "driver/pg", "driver/sq"],
                 "to": "driver/pick/pick.ts", "pairs_removed": 5,
                 "cycle_before": 6, "cycle_after": 0 }])
    );

    // A file nobody imports is an entry point build config names: proposed for a move only when
    // the cycle comes from grouping (no file cycle), and flagged.
    let edges: Vec<(String, String)> = edges
        .into_iter()
        .filter(|(f, _)| f != "app/use.ts")
        .collect();
    let (dir, input) = synthetic("fan-out-entry", edges.len() + 2, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    assert_eq!(doc["cycles"][0]["grouping"], true);
    assert_eq!(
        doc["cycles"][0]["registrar_moves"][0]["entrypoint"], true,
        "{:#}",
        doc["cycles"][0]
    );
    // driver/base.ts importing driver/pg/b.ts closes a file cycle across two members: not
    // grouping, and the entry point stays out.
    let mut cyclic = edges.clone();
    cyclic.push(("driver/base.ts".to_owned(), "driver/pg/b.ts".to_owned()));
    let (dir, input) = synthetic("fan-out-entry-cycle", cyclic.len() + 2, &cyclic);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    assert_eq!(doc["cycles"][0]["grouping"], false);
    assert!(
        doc["cycles"][0].get("registrar_moves").is_none(),
        "{:#}",
        doc["cycles"][0]
    );

    // Nor is a module's index: moving it would move the module's entry.
    let edges: Vec<(String, String)> = edges
        .iter()
        .map(|(f, t)| (f.replace("pick.ts", "index.ts"), t.clone()))
        .chain([("app/use.ts".to_owned(), "driver/index.ts".to_owned())])
        .collect();
    let (dir, input) = synthetic("fan-out-index", edges.len() + 2, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    assert!(
        doc["cycles"][0].get("registrar_moves").is_none(),
        "{:#}",
        doc["cycles"][0]
    );

    // Top-level modules share no parent folder: a file using five of them picks nothing, and
    // core -> a stays the lighter side.
    let mut edges = vec![("app/use.ts".to_owned(), "core/x.ts".to_owned())];
    for m in ["a", "b", "c", "d", "e"] {
        edges.push(("core/x.ts".to_owned(), format!("{m}/one.ts")));
    }
    for f in ["one", "two", "three"] {
        edges.push((format!("a/{f}.ts"), "core/y.ts".to_owned()));
    }
    let (dir, input) = synthetic("fan-out-top", edges.len() + 2, &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    let pair = &doc["cycles"][0]["two_way"][0];
    assert_eq!(
        (&pair["from"], &pair["to"], pair.get("registry")),
        (&json!("core"), &json!("a"), None)
    );
}

#[test]
fn report_on_a_composition_root_in_a_shared_folder() {
    // Three api modules import app/deps.ts and app/routing.ts; only app/main.ts, which nothing
    // imports, imports them back. No file cycle: the cycle is main.ts sharing the app folder.
    let mut edges = vec![("app/main.ts".to_owned(), "app/deps.ts".to_owned())];
    for m in ["a", "b", "c"] {
        edges.push((format!("{m}/api/x.ts"), "app/deps.ts".to_owned()));
        edges.push((format!("{m}/api/x.ts"), "app/routing.ts".to_owned()));
        edges.push(("app/main.ts".to_owned(), format!("{m}/api/x.ts")));
    }
    let report_of = |name: &str, edges: &[(String, String)]| {
        let (dir, input) = synthetic(name, edges.len() + 1, edges);
        let (doc, _, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("report/{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let (stdout, stderr, _) = report(&[
            root.to_str().unwrap(),
            "--language",
            "typescript",
            "--collector-input",
            &input,
        ]);
        (doc, stdout + &stderr)
    };
    let (doc, stdout) = report_of("composition-root", &edges);
    let cycle = &doc["cycles"][0];
    assert_eq!(cycle["grouping"], true, "{cycle:#}");
    assert_eq!(
        cycle["registrar_moves"],
        json!([{ "file": "app/main.ts", "module": "app", "pairs": 3,
                 "with": ["a/api", "b/api", "c/api"], "to": "app/main/main.ts",
                 "pairs_removed": 3, "cycle_before": 4, "cycle_after": 0, "entrypoint": true }])
    );
    for line in [
        "  cycle comes from grouping: no file-level cycle; app/main.ts shares app with app/deps.ts, app/routing.ts\n  move app/main.ts to app/main/main.ts: removes 3 two-way pairs; the 4-module cycle goes; no new cycle\n",
        "  nothing imports app/main.ts: update deploy and config references to its path\n",
    ] {
        assert!(stdout.contains(line), "{line:?} missing from\n{stdout}");
    }
    assert!(!stdout.contains("break the"), "{stdout}");
    // The move resolves every x/api -> app pair, and no file cycle backs the module cycle: no
    // recipe proposes cutting an x/api -> app import.
    let (dir, input) = synthetic("composition-root", edges.len() + 1, &edges);
    let (raw, _, _) = common::scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let finding = raw["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| &d["finding"])
        .find(|f| f["kind"] == "cycle")
        .unwrap();
    assert!(finding.get("recipe").is_none(), "{finding:#}");

    // A second, uneven pair (a/api -> e once, back four times) stays in the cycle: the break
    // line names only its import, not the app/main.ts imports the move already takes out.
    let mut extra = edges.clone();
    extra.extend((1..=4).map(|i| (format!("e/y{i}.ts"), "a/api/z.ts".to_owned())));
    extra.push(("a/api/w.ts".to_owned(), "e/v.ts".to_owned()));
    let (_, stdout) = report_of("composition-root-extra-pair", &extra);
    assert!(
        stdout.contains("  break the 5-module cycle: cut 1 import from a/api/w.ts (1)\n"),
        "{stdout}"
    );

    // Two composition roots in one grouping cycle: two moves, one grouping sentence.
    let mut two = edges.clone();
    for m in ["d", "e", "f"] {
        two.push((format!("{m}/api/x.ts"), "srv/lib.ts".to_owned()));
        two.push((format!("{m}/api/x.ts"), "srv/util.ts".to_owned()));
        two.push(("srv/boot.ts".to_owned(), format!("{m}/api/x.ts")));
    }
    two.extend(
        [
            ("srv/lib.ts", "app/routing.ts"),
            ("app/deps.ts", "srv/lib2.ts"),
            ("srv/boot.ts", "srv/lib.ts"),
        ]
        .map(|(f, t)| (f.to_owned(), t.to_owned())),
    );
    let (_, stdout) = report_of("two-composition-roots", &two);
    assert_eq!(stdout.matches("comes from grouping").count(), 1, "{stdout}");
    assert_eq!(stdout.matches("\n  move ").count(), 2, "{stdout}");

    // app/deps.ts importing a/api/x.ts closes a file cycle across modules: no grouping, no move,
    // and the report shows the break set grouped by importing file.
    let mut cyclic = edges.clone();
    cyclic.push(("app/deps.ts".to_owned(), "a/api/x.ts".to_owned()));
    let (doc, stdout) = report_of("composition-root-cycle", &cyclic);
    assert_eq!(doc["cycles"][0]["grouping"], false);
    assert!(doc["cycles"][0].get("registrar_moves").is_none());
    assert!(
        stdout.contains(
            "  break the 4-module cycle: cut 4 imports from a/api/x.ts (2), app/main.ts (2)\n"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains("comes from grouping"), "{stdout}");

    // Two modules importing each other through different files, with no registrar to move: the
    // report still names the grouping.
    let pair =
        [("a/x.ts", "b/y.ts"), ("b/z.ts", "a/w.ts")].map(|(f, t)| (f.to_owned(), t.to_owned()));
    let (doc, stdout) = report_of("grouping-pair", &pair);
    assert_eq!(doc["cycles"][0]["grouping"], true);
    assert!(doc["cycles"][0].get("registrar_moves").is_none());
    assert!(
        stdout.contains("  cycle of a, b comes from grouping: no file-level cycle; moving files between these modules can end it without cutting imports\n"),
        "{stdout}"
    );
    // Five such pairs: three lines, then a count.
    let pairs: Vec<(String, String)> = (0..5)
        .flat_map(|i| {
            [
                (format!("a{i}/x.ts"), format!("b{i}/y.ts")),
                (format!("b{i}/z.ts"), format!("a{i}/w.ts")),
            ]
        })
        .collect();
    let (_, stdout) = report_of("grouping-pairs", &pairs);
    assert_eq!(stdout.matches("comes from grouping").count(), 3, "{stdout}");
    assert!(
        stdout.contains("  and 2 more grouping cycles\n"),
        "{stdout}"
    );

    // 17 modules in a ring, plus one lighter pair: over 16 module edges the break set is a
    // heuristic, and the report says so.
    let mut ring: Vec<(String, String)> = (0..17)
        .map(|i| (format!("m{i}/x.ts"), format!("m{}/x.ts", (i + 1) % 17)))
        .collect();
    ring.extend((0..3).map(|i| (format!("m1/y{i}.ts"), "m0/x.ts".to_owned())));
    let (doc, stdout) = report_of("ring-17", &ring);
    assert_eq!(doc["cycles"][0]["exact"], false);
    assert!(
        stdout.contains("  break the 17-module cycle (approximate): cut "),
        "{stdout}"
    );
}

#[test]
fn a_near_even_two_way_pair_names_no_lighter_side() {
    // a -> b once, b -> a twice: both sides are tiny and close, a mutual dependency rather
    // than a soft side. 1 against 3 still has a lighter side.
    for (name, back, even) in [("even-1-2", 2, true), ("uneven-1-3", 3, false)] {
        let mut edges = vec![("a/x.ts".to_owned(), "b/y.ts".to_owned())];
        edges.extend((0..back).map(|i| (format!("b/z{i}.ts"), "a/x.ts".to_owned())));
        let (dir, input) = synthetic(name, edges.len() + 1, &edges);
        let (doc, _, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        let pair = &doc["cycles"][0]["two_way"][0];
        assert_eq!(
            pair.get("even") == Some(&json!(true)),
            even,
            "{name}: {pair:#}"
        );
        assert_eq!(
            (&pair["from"], &pair["to"]),
            (&json!("a"), &json!("b")),
            "{name}"
        );
    }
}

#[test]
fn an_import_of_types_only_is_type_without_import_type() {
    // use.ts imports the interface ParamData without `import type`: the compiler drops the
    // import, so it is `type`. version.ts imports a value: `static`.
    let (doc, _, _) = scan_recorded("ts-elided", &[]);
    let kinds: Vec<(&Value, &Value)> = doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (&e["from"], &e["kind"]))
        .collect();
    assert_eq!(
        kinds,
        [
            (&json!("src/use.ts"), &json!("type")),
            (&json!("src/version.ts"), &json!("runtime"))
        ]
    );
}

#[test]
fn module_cycle_lists_its_two_way_pairs_lighter_side_first() {
    // x -> y 3, y -> x 1; x -> z 2, z -> x 1; y <-> z 2 each; p -> q 22, q -> p 21. x/z and y/z
    // are even (both sides at most 2), in the names' order and after the others. Sorted by the
    // lighter side's imports, then names; at most 20 edges.
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut imports = |from: &str, to: &str, n: usize| {
        for i in 0..n {
            edges.push((format!("{from}/f{i:02}.ts"), format!("{to}/g.ts")));
        }
    };
    imports("x", "y", 3);
    imports("y", "x", 1);
    imports("x", "z", 2);
    imports("z", "x", 1);
    imports("y", "z", 2);
    imports("z", "y", 2);
    imports("p", "q", 22);
    imports("q", "p", 21);
    let (dir, input) = synthetic("two-way-pairs", edges.len(), &edges);
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let link = |from: &str, i: usize, to: &str| {
        let (from, to) = (format!("{from}/f{i:02}.ts"), format!("{to}/g.ts"));
        json!({ "from": from, "to": to })
    };
    let pq: Vec<Value> = (0..20).map(|i| link("q", i, "p")).collect();
    assert_eq!(doc["cycles"][0]["members"], json!(["p", "q"]));
    assert_eq!(
        doc["cycles"][0]["two_way"],
        json!([{ "from": "q", "to": "p", "imports": 21, "reverse_imports": 22, "edges": pq }])
    );
    assert_eq!(doc["cycles"][1]["members"], json!(["x", "y", "z"]));
    assert_eq!(
        doc["cycles"][1]["two_way"],
        json!([
            { "from": "y", "to": "x", "imports": 1, "reverse_imports": 3,
              "edges": [link("y", 0, "x")] },
            { "from": "x", "to": "z", "imports": 2, "reverse_imports": 1, "even": true,
              "edges": [link("x", 0, "z"), link("x", 1, "z")] },
            { "from": "y", "to": "z", "imports": 2, "reverse_imports": 2, "even": true,
              "edges": [link("y", 0, "z"), link("y", 1, "z")] }
        ])
    );
}

#[test]
fn files_outside_the_source_root_never_merge_with_its_modules() {
    // depcruise follows imports out of the cruised src/: lib/u.ts and tool.ts sit beside it.
    // Each file directly in a root is its own module, named with its extension, so src/lib.ts
    // never merges with src/lib/ nor with src/main.ts.
    let edges: Vec<(String, String)> = [
        ("src/main.ts", "src/lib/a.ts"),
        ("src/main.ts", "src/lib.ts"),
        ("src/lib/a.ts", "lib/u.ts"),
        ("src/lib/a.ts", "tool.ts"),
    ]
    .iter()
    .map(|(f, t)| (f.to_string(), t.to_string()))
    .collect();
    let (dir, input) = synthetic("outside-source-root", 5, &edges);
    fs::create_dir_all(std::path::Path::new(&dir).join("src")).unwrap();
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(doc["config"]["source_root"], "src");
    assert_eq!(
        doc["files"],
        json!([
            { "path": "lib/u.ts", "module": "../lib", "barrel": false },
            { "path": "src/lib.ts", "module": "lib.ts", "barrel": false },
            { "path": "src/lib/a.ts", "module": "lib", "barrel": false },
            { "path": "src/main.ts", "module": "main.ts", "barrel": false },
            { "path": "tool.ts", "module": "../tool.ts", "barrel": false }
        ])
    );
}

#[test]
fn only_source_files_become_files() {
    let (dir, input) = common::synthetic_typed(
        "source-files-only",
        4,
        &[
            ("src/a/x.ts", "src/a/x.css", &["local", "import"]),
            ("src/a/x.ts", "src/b/y.tsx", &["local", "import"]),
            ("src/b/y.tsx", "src/b/data.json", &["local", "import"]),
        ],
    );
    let (doc, _, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    assert_eq!(files(&doc), ["src/a/x.ts", "src/b/y.tsx"]);
    assert_eq!(
        common::without_import_metadata(&doc["edges"]),
        json!([{ "from": "src/a/x.ts", "to": "src/b/y.tsx", "kind": "runtime", "weight": 1.0, "verdict": null,
                 "rule": null }])
    );
}

#[test]
fn cochange_of_zero_commits_is_a_usage_error() {
    let dir = fixture("ts-basic");
    let out = archgraph(&["scan", dir.to_str().unwrap(), "--cochange=0"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--cochange"));
}

#[test]
fn git_runs_without_the_repository_fsmonitor_hook() {
    // A repository's core.fsmonitor names a program git runs on `status`; scan must not run it.
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fsmonitor");
    let _ = fs::remove_dir_all(&base);
    let repo = base.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let marker = base.join("RAN");
    let hook = base.join("hook.sh");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    git(&repo, &["init", "-q"]);
    commit(&repo, 1, &["src/a/x.ts", "src/b/y.ts"]);
    git(&repo, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
    let (_, input) = synthetic(
        "fsmonitor-input",
        2,
        &[("src/a/x.ts".to_owned(), "src/b/y.ts".to_owned())],
    );
    scan(&[
        repo.to_str().unwrap(),
        "--collector-input",
        &input,
        "--cochange",
    ]);
    assert!(!marker.exists(), "scan ran the fsmonitor hook");
}

#[test]
fn schema_accepts_unknown_finding_kinds_and_types_known_ones() {
    let schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.2.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let (doc, _, _) = common::scan_recorded("ts-basic", &[]);
    let with = |finding: Value| {
        let mut d = doc.clone();
        d["findings"].as_array_mut().unwrap().push(finding);
        validator.is_valid(&d)
    };
    assert!(with(
        json!({ "kind": "future-kind", "subject": "x", "evidence": { "anything": 1 } })
    ));
    assert!(!with(
        json!({ "kind": "small-repo", "subject": ".", "evidence": {} })
    ));
}

/// Runs `report`, returning stdout, stderr and the exit code.
fn report(args: &[&str]) -> (String, String, Option<i32>) {
    let out = archgraph(&[&["report"], args].concat());
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code(),
    )
}

#[test]
fn report_finds_layout_yaml_writes_both_files_and_exits_1_on_forbidden() {
    let dir = fixture("ts-fda");
    let input = dir.join("depcruise.json");
    let out_dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("report/ts-fda");
    let _ = fs::remove_dir_all(&out_dir);
    let (stdout, stderr, code) = report(&[
        dir.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
        "--out-dir",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    let layout_yaml = dir.join("layout.yaml");
    assert!(
        stdout.contains(&format!("rules: {}", layout_yaml.display())),
        "{stdout}"
    );
    for line in [
        "12 files, 4 modules, 13 imports",
        "5 forbidden: browser-no-server 3, public 1, shared-no-domain 1",
        "1 unplanned: none 1",
        "no module cycles",
        "internals-reach (1)",
        "    src/domains/checkout/**\n",
    ] {
        assert!(stdout.contains(line), "{line:?} missing from\n{stdout}");
    }
    assert!(!stdout.contains('\x1b'), "color on a pipe");

    // The JSON is the scan document `scan --rules layout.yaml` writes; the page embeds it.
    let (json, html) = (
        out_dir.join("archgraph.json"),
        out_dir.join("archgraph.html"),
    );
    let written = fs::read_to_string(&json).unwrap();
    validate(&serde_json::from_str(&written).unwrap());
    let (_, scanned, _) = scan_recorded("ts-fda", &["--rules", &layout("ts-fda")]);
    assert_eq!(stable(&written), stable(&scanned));
    assert!(fs::read_to_string(&html).unwrap().contains(r#"id="graph""#));
    assert!(stdout.contains(&json.display().to_string()));
    assert!(
        stdout.trim_end().ends_with(&html.display().to_string()),
        "{stdout}"
    );
}

#[test]
fn report_without_rules_exits_0_and_writes_into_the_root_by_default() {
    // An empty root read as TypeScript with ts-cycles' recorded output: modules are src/a ...
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("report/norules");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let input = fixture("ts-cycles").join("depcruise.json");
    let (stdout, stderr, code) = report(&[
        root.to_str().unwrap(),
        "--language",
        "typescript",
        "--collector-input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    for line in [
        "rules: none",
        "11 files, 6 modules, 12 imports",
        "2 module cycles, largest 3 modules",
        "no lighter side to cut: the two-way pairs are even or wiring",
        "cycle (2)",
        "src/a, src/b, src/c",
        "misplaced-file (1)",
        "seam (1)\n    clusters across folders\n",
    ] {
        assert!(stdout.contains(line), "{line:?} missing from\n{stdout}");
    }
    assert!(!stdout.contains("forbidden"), "{stdout}");
    let out_dir = root.join(".archgraph");
    assert!(out_dir.join("archgraph.json").is_file());
    assert!(out_dir.join("archgraph.html").is_file());
}

#[cfg(unix)]
#[test]
fn report_refuses_to_write_through_symlinks_the_scanned_repository_planted() {
    use std::os::unix::fs::symlink;
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("report/symlinks");
    let _ = fs::remove_dir_all(&base);
    let input = fixture("ts-cycles").join("depcruise.json");
    let victim = base.join("victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("keep.txt"), "untouched").unwrap();
    let run = |root: &std::path::Path| {
        report(&[
            root.to_str().unwrap(),
            "--language",
            "typescript",
            "--collector-input",
            input.to_str().unwrap(),
        ])
    };

    // The repository ships .archgraph as a link to a folder elsewhere.
    let root = base.join("dir-link");
    fs::create_dir_all(&root).unwrap();
    symlink(&victim, root.join(".archgraph")).unwrap();
    let (_, stderr, code) = run(&root);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("symbolic link"), "{stderr}");
    assert!(!victim.join("archgraph.html").exists());

    // The repository ships .archgraph/archgraph.html as a link to a file elsewhere.
    let root = base.join("file-link");
    fs::create_dir_all(root.join(".archgraph")).unwrap();
    symlink(
        victim.join("keep.txt"),
        root.join(".archgraph/archgraph.html"),
    )
    .unwrap();
    let (_, stderr, code) = run(&root);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("symbolic link"), "{stderr}");
    assert_eq!(
        fs::read_to_string(victim.join("keep.txt")).unwrap(),
        "untouched"
    );

    // Outputs are replaced, never truncated in place: a hard link to the old file keeps it.
    let root = base.join("hard-link");
    fs::create_dir_all(root.join(".archgraph")).unwrap();
    fs::hard_link(
        victim.join("keep.txt"),
        root.join(".archgraph/archgraph.json"),
    )
    .unwrap();
    let (_, stderr, code) = run(&root);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        fs::read_to_string(root.join(".archgraph/archgraph.json"))
            .unwrap()
            .contains("\"scan\"")
    );
    assert_eq!(
        fs::read_to_string(victim.join("keep.txt")).unwrap(),
        "untouched"
    );
}

#[test]
fn report_with_unreadable_rules_is_an_error() {
    let dir = fixture("ts-cycles");
    let input = dir.join("depcruise.json");
    let out_dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("report/bad-rules");
    let (_, stderr, code) = report(&[
        dir.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
        "--rules",
        "/nonexistent/layout.yaml",
        "--out-dir",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("/nonexistent/layout.yaml"), "{stderr}");
}

#[test]
fn depcruise_path_that_does_not_exist_is_a_usage_error() {
    let out = archgraph(&[
        "scan",
        fixture("ts-basic").to_str().unwrap(),
        "--depcruise",
        "/nonexistent/depcruise",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--depcruise /nonexistent/depcruise"),
        "{stderr}"
    );
}

fn scan(args: &[&str]) -> (Value, String, String) {
    let (doc, out, err) = common::scan(args);
    (common::evidence_contract(doc), out, err)
}
fn scan_recorded(name: &str, args: &[&str]) -> (Value, String, String) {
    let (doc, out, err) = common::scan_recorded(name, args);
    (common::evidence_contract(doc), out, err)
}
