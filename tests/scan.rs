mod common;

use std::fs;

use common::{archgraph, fixture, scan, scan_recorded, stable, synthetic};
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
    assert_eq!(doc["schema"], "archgraph/v1");
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
            { "path": "src/main.ts", "module": ".", "barrel": false }
        ])
    );
    // main.ts reaches run.ts through the `@/` tsconfig alias; run.ts -> types.ts is `import type`;
    // store.ts imports types.ts both as a type and as a value, so the pair is static.
    assert_eq!(
        doc["edges"],
        json!([
            { "from": "src/app/shell/run.ts", "to": "src/core/model/store.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/app/shell/run.ts", "to": "src/core/model/types.ts", "kind": "type", "verdict": null, "rule": null },
            { "from": "src/app/shell/run.ts", "to": "src/core/util/log.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/app/shell/view.ts", "to": "src/app/shell/run.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/app/shell/view.ts", "to": "src/core/util/log.ts", "kind": "dynamic", "verdict": null, "rule": null },
            { "from": "src/core/model/store.ts", "to": "src/core/model/types.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/core/model/store.ts", "to": "src/core/util/log.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/main.ts", "to": "src/app/shell/run.ts", "kind": "static", "verdict": null, "rule": null }
        ])
    );
    assert_eq!(
        doc["modules"],
        json!([
            { "id": ".", "files": 1, "ca": 0, "ce": 1, "instability": 1.0,
              "internal_edges": 0, "outgoing_edges": 1, "cohesion": 0.0 },
            { "id": "app/shell", "files": 2, "ca": 1, "ce": 2, "instability": 0.6667,
              "internal_edges": 1, "outgoing_edges": 4, "cohesion": 0.2 },
            { "id": "core/model", "files": 2, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 1, "outgoing_edges": 1, "cohesion": 0.5 },
            { "id": "core/util", "files": 1, "ca": 2, "ce": 0, "instability": 0.0,
              "internal_edges": 0, "outgoing_edges": 0, "cohesion": null }
        ])
    );
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": ".", "to": "app/shell", "imports": 1, "forbidden": 0 },
            { "from": "app/shell", "to": "core/model", "imports": 2, "forbidden": 0 },
            { "from": "app/shell", "to": "core/util", "imports": 2, "forbidden": 0 },
            { "from": "core/model", "to": "core/util", "imports": 1, "forbidden": 0 }
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
            ".": m(json!(0.0), 0, 1, json!(1.0)),
            "app/shell": m(json!(0.2), 1, 2, json!(0.6667)),
            "core/model": m(json!(0.5), 1, 1, json!(0.5)),
            "core/util": m(Value::Null, 2, 0, json!(0.0)) })
        )
    );

    // run.ts has 1 edge inside app/shell and 2 into core/model, and only main.ts imports it from
    // another module. Moving it takes app/shell and core/model together from cohesion 2/7 to 3/7.
    // log.ts talks only to app/shell but is the only file of core/util, so it is not misplaced.
    // Label propagation puts all six files in one cluster: no seam. The simulation lists only
    // the modules whose metrics change.
    assert_eq!(
        doc["findings"],
        json!([
            { "kind": "misplaced-file", "subject": "src/app/shell/run.ts",
              "evidence": { "own_module_edges": 1, "target_module": "core/model",
                            "target_module_edges": 2, "via_barrel": false },
              "suggestion": { "move": { "file": "src/app/shell/run.ts", "to": "src/core/model/run.ts" } },
              "simulation": { "after": summary(0, json!({
                  "app/shell": m(json!(0.0), 0, 2, json!(1.0)),
                  "core/model": m(json!(0.6), 2, 1, json!(0.3333)) })) } },
            { "kind": "small-repo", "subject": ".", "evidence": { "files": 6, "modules": 4 } }
        ])
    );
}

/// `{ cohesion, ca, ce, instability }` of one module in a Summary.
fn m(cohesion: Value, ca: usize, ce: usize, instability: Value) -> Value {
    json!({ "cohesion": cohesion, "ca": ca, "ce": ce, "instability": instability })
}

/// A Summary without rules: nothing forbidden or unplanned.
fn summary(cycles: usize, modules: Value) -> Value {
    json!({ "forbidden": 0, "unplanned": 0, "cycles": cycles, "modules": modules })
}

#[test]
fn schema_is_valid_draft_2020_12() {
    let schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.schema.json")).unwrap();
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

    // The same tree somewhere else gives the same bytes: no absolute paths are recorded.
    let copy = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ts-basic-copy");
    let _ = fs::remove_dir_all(&copy);
    copy_dir(&fixture("ts-basic"), &copy);
    let input = copy.join("depcruise.json");
    let (_, moved, _) = scan(&[
        copy.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(stable(&first), stable(&moved));
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
    // a->b and b->c carry 2 imports each, c->a carries 1; d->e carries 2, e->d 1.
    // Inside f, p->q is the only single import whose removal breaks both p<->q and p->q->r->p.
    // The a/b/c and d/e file cycles cross modules, so only the module cycles report them.
    assert_eq!(
        doc["cycles"],
        json!([
            { "level": "module", "members": ["a", "b", "c"], "exact": true,
              "break": [{ "from": "src/c/z.ts", "to": "src/a/x.ts" }] },
            { "level": "module", "members": ["d", "e"], "exact": true,
              "break": [{ "from": "src/e/n.ts", "to": "src/d/m.ts" }] },
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
    // Each of y, z, m, n has no edge inside its module and >= 2 into one other; z and n are
    // alone in their modules, so only y and m are misplaced. y is imported only from a, m only
    // from e.
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
    let e = |target: &str, edges: usize| {
        json!({ "own_module_edges": 0, "target_module": target, "target_module_edges": edges,
                "via_barrel": false })
    };
    assert_eq!(
        misplaced,
        [
            (&json!("src/b/y.ts"), &e("a", 2), &json!("src/a/y.ts")),
            (&json!("src/d/m.ts"), &e("e", 2), &json!("src/e/m.ts")),
        ]
    );
    // m.ts into e: the d <-> e module cycle becomes the file cycle m <-> n inside e, and d keeps
    // only m2 -> n. Only d and e change.
    let sim = &findings
        .iter()
        .find(|f| f["subject"] == "src/d/m.ts")
        .unwrap()["simulation"];
    assert_eq!(doc["summary"]["cycles"], 3);
    assert_eq!(
        sim["after"],
        summary(
            3,
            json!({ "d": m(json!(0.0), 0, 1, json!(1.0)),
                           "e": m(json!(1.0), 1, 0, json!(0.0)) })
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
    assert_eq!(findings.len(), 3 + 2 + 1 + 1);
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
    // `export type { Kind } from "./kinds"` is a re-export too, so index.ts stays a barrel.
    assert_eq!(
        doc["edges"],
        json!([
            { "from": "src/app/main.ts", "to": "src/feature/index.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/app/main.ts", "to": "src/shared/util.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/feature/b.ts", "to": "src/shared/util.ts", "kind": "static", "verdict": null, "rule": null },
            { "from": "src/feature/index.ts", "to": "src/feature/a.ts", "kind": "reexport", "verdict": null, "rule": null },
            { "from": "src/feature/index.ts", "to": "src/feature/b.ts", "kind": "reexport", "verdict": null, "rule": null },
            { "from": "src/feature/index.ts", "to": "src/feature/kinds.ts", "kind": "reexport", "verdict": null, "rule": null }
        ])
    );
    // main -> index counts as main -> a, b and kinds; the barrel's own re-exports are dropped,
    // so feature has no internal edges (cohesion 0.0, not 3/4).
    assert_eq!(
        doc["modules"],
        json!([
            { "id": "app", "files": 1, "ca": 0, "ce": 2, "instability": 1.0,
              "internal_edges": 0, "outgoing_edges": 4, "cohesion": 0.0 },
            { "id": "feature", "files": 4, "ca": 1, "ce": 1, "instability": 0.5,
              "internal_edges": 0, "outgoing_edges": 1, "cohesion": 0.0 },
            { "id": "shared", "files": 1, "ca": 2, "ce": 0, "instability": 0.0,
              "internal_edges": 0, "outgoing_edges": 0, "cohesion": null }
        ])
    );
    // Module edges are raw imports, as cycles and their break sets need.
    assert_eq!(
        doc["module_edges"],
        json!([
            { "from": "app", "to": "feature", "imports": 1, "forbidden": 0 },
            { "from": "app", "to": "shared", "imports": 1, "forbidden": 0 },
            { "from": "feature", "to": "shared", "imports": 1, "forbidden": 0 }
        ])
    );
    // main.ts -> index.ts counts as 3 edges into feature (via the barrel), but main.ts is the
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
            { "level": "module", "members": ["x", "y"], "exact": true,
              "break": [{ "from": "x/a.ts", "to": "y/c.ts" }] },
            { "level": "file", "members": ["x/a.ts", "x/b.ts"], "exact": true,
              "break": [{ "from": "x/a.ts", "to": "x/b.ts" }] }
        ])
    );
}

#[test]
fn files_outside_the_source_root_never_merge_with_its_modules() {
    // depcruise follows imports out of the cruised src/: lib/u.ts and tool.ts sit beside it.
    let edges: Vec<(String, String)> = [
        ("src/main.ts", "src/lib/a.ts"),
        ("src/lib/a.ts", "lib/u.ts"),
        ("src/lib/a.ts", "tool.ts"),
    ]
    .iter()
    .map(|(f, t)| (f.to_string(), t.to_string()))
    .collect();
    let (dir, input) = synthetic("outside-source-root", 4, &edges);
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
            { "path": "src/lib/a.ts", "module": "lib", "barrel": false },
            { "path": "src/main.ts", "module": ".", "barrel": false },
            { "path": "tool.ts", "module": "..", "barrel": false }
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
        doc["edges"],
        json!([{ "from": "src/a/x.ts", "to": "src/b/y.tsx", "kind": "static", "verdict": null,
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
        serde_json::from_str(include_str!("../schema/archgraph.v1.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let (doc, _, _) = scan_recorded("ts-basic", &[]);
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
