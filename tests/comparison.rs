mod common;

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use archgraph::model::{Config, Edge, GlobalUse, ImportRecord, Source};
use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const OPEN: &str = "<script type=\"application/json\" id=\"comparison\">";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "archgraph-comparison-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join(".archgraph")).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, text: &str) -> PathBuf {
        let target = self.0.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, text).unwrap();
        target
    }
    fn save(&self, path: &str, value: &Value) -> PathBuf {
        self.write(path, &value.to_string())
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    fn view(&self, before: &Value, after: &Value) -> Value {
        self.save("before.json", before);
        self.save("after.json", after);
        let out = self.run(&[
            "diff",
            "before.json",
            "after.json",
            "--html",
            "changes.html",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        embedded(&fs::read_to_string(self.0.join("changes.html")).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn embedded(html: &str) -> Value {
    let text = html
        .split_once(OPEN)
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    assert!(
        !text.contains('<'),
        "repository strings may not terminate the data element"
    );
    serde_json::from_str(text).unwrap()
}

fn policy(forbid: Value) -> Value {
    json!({"version":1,"modules":[
        {"id":"domain","path":"src/domain/**"},
        {"id":"adapters","path":"src/adapters/**"},
        {"id":"shared","path":"src/shared/**"}
    ],"forbid":forbid})
}

fn rules() -> Value {
    policy(
        json!([{"id":"no-adapters","from":"src/domain/**","to":"src/adapters/**",
        "reason":"Keep domain independent.","source":"docs/adr/boundary.md"}]),
    )
}

fn edge(line: usize) -> Edge {
    serde_json::from_value(json!({"from":"src/domain/a.ts","to":"src/adapters/db.ts",
        "kind":"runtime","line":line,"specifier":"../adapters/db","verdict":null,"rule":null}))
    .unwrap()
}

fn document(edges: Vec<Edge>, rules: Value) -> Value {
    let mut config: Config = serde_json::from_value(json!({
        "depth":2,"source_root":"src","excludes":[],"include_tests":false,
        "rules":{"path":"layout.yaml","sha256":"fixture","doc":rules},
        "collection":{"engine":"native","python_package":null,"rust_features":null,"rust_build_scripts":false}
    })).unwrap();
    // An explicit empty ledger is valid evidence that this fixture has no package imports.
    config.import_records = Some(vec![]);
    config.globals = Some(vec![]);
    analyzed(config, edges)
}

fn analyzed(config: Config, edges: Vec<Edge>) -> Value {
    let source: Source = serde_json::from_value(json!({
        "language":"typescript","collector":{"name":"fixture","version":"1","cruised":4},
        "generated_at":"2026-10-25T00:00:00Z","scan_root":".",
        "git":{"commit":"same-recorded-head","dirty":false}
    }))
    .unwrap();
    serde_json::to_value(
        archgraph::analyze(
            source,
            config,
            vec![
                "src/domain/a.ts".into(),
                "src/adapters/db.ts".into(),
                "src/shared/one.ts".into(),
                "src/shared/two.ts".into(),
            ],
            edges,
            vec![],
            None,
        )
        .unwrap(),
    )
    .unwrap()
}

fn projected(before: &Value, after: &Value) -> Value {
    let (old, old_provenance) =
        archgraph::comparison::read(&before.to_string(), "before.json").unwrap();
    let (new, new_provenance) =
        archgraph::comparison::read(&after.to_string(), "after.json").unwrap();
    let old = archgraph::restore_scan(old).unwrap();
    let new = archgraph::restore_scan(new).unwrap();
    serde_json::to_value(
        archgraph::comparison::project(&old, &old_provenance, &new, &new_provenance).unwrap(),
    )
    .unwrap()
}

#[test]
fn native_code_edit_keeps_removed_evidence_and_preserves_the_existing_gate_output() {
    let f = Fixture::new();
    f.write("package.json", "{\"type\":\"module\"}");
    f.write("layout.yaml", &serde_yaml_ng::to_string(&rules()).unwrap());
    f.write(
        "src/domain/a.ts",
        "import {value} from '../shared/one'; export const order = value;\n",
    );
    f.write("src/shared/one.ts", "export const value = 1;\n");
    f.write("src/adapters/db.ts", "export const db = 2;\n");
    let collect = |name| {
        let out = f.run(&[
            "scan",
            ".",
            "--engine",
            "native",
            "--symbols",
            "--rules",
            "layout.yaml",
            "-o",
            name,
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    collect(".archgraph/before.json");
    fs::remove_file(f.0.join("src/shared/one.ts")).unwrap();
    f.write(
        "src/domain/a.ts",
        "import {db} from '../adapters/db'; export const order = db;\n",
    );
    f.write("src/extra/new.ts", "export const extra = 3;\n");
    collect(".archgraph/after.json");
    let args = [
        "diff",
        ".archgraph/before.json",
        ".archgraph/after.json",
        "--fail-on",
        "new-forbidden",
    ];
    let normal = f.run(&args);
    let html = f.run(&[args.as_slice(), &["--html", ".archgraph/changes.html"]].concat());
    assert_eq!(normal.status.code(), Some(1));
    assert_eq!(html.status.code(), Some(1));
    assert_eq!(
        normal.stdout, html.stdout,
        "HTML must not change the existing JSON diff"
    );
    let view = embedded(&fs::read_to_string(f.0.join(".archgraph/changes.html")).unwrap());
    let imports = view["imports"].as_array().unwrap();
    let added = imports.iter().find(|row| row["status"] == "added").unwrap();
    assert_eq!(added["to"], "src/adapters/db.ts");
    assert_eq!(added["after"]["evidence"][0]["verdict"], "forbidden");
    assert_eq!(
        added["after"]["evidence"][0]["reasons"][0],
        "Keep domain independent."
    );
    assert_eq!(
        added["after"]["evidence"][0]["sources"][0],
        "docs/adr/boundary.md"
    );
    let removed = imports
        .iter()
        .find(|row| row["status"] == "removed")
        .unwrap();
    assert_eq!(removed["to"], "src/shared/one.ts");
    assert_eq!(removed["before"]["evidence"][0]["line"], 1);
    assert!(removed["after"].is_null());
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["label"] == "shared" && node["status"] == "removed")
    );
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["label"] == "extra" && node["status"] == "added")
    );
}

#[test]
fn line_movement_is_evidence_only_and_recorded_dirty_state_survives_restoration() {
    let old = document(vec![edge(3)], rules());
    let mut new = document(vec![edge(18)], rules());
    new["source"]["git"]["dirty"] = json!(true);
    let view = projected(&old, &new);
    assert_eq!(view["imports"][0]["status"], "changed");
    assert_eq!(view["imports"][0]["changes"], json!(["evidence"]));
    assert_eq!(view["imports"][0]["before"]["evidence"][0]["line"], 3);
    assert_eq!(view["imports"][0]["after"]["evidence"][0]["line"], 18);
    assert_eq!(
        view["comparison"]["before"]["commit"],
        view["comparison"]["after"]["commit"]
    );
    assert_eq!(view["comparison"]["before"]["dirty"], false);
    assert_eq!(view["comparison"]["after"]["dirty"], true);
    assert_eq!(view["comparison"]["policy_changed"], false);
    assert_eq!(view["comparison"]["status"], "recorded-compatible");
}

#[test]
fn policy_only_and_module_assignment_changes_do_not_invent_added_imports() {
    let old = document(vec![edge(3)], policy(json!([])));
    let mut changed = rules();
    changed["modules"][0]["id"] = json!("domain-renamed");
    let new = document(vec![edge(3)], changed);
    let view = projected(&old, &new);
    assert_eq!(view["comparison"]["policy_changed"], true);
    assert_eq!(view["imports"][0]["status"], "changed");
    assert_eq!(
        view["imports"][0]["changes"],
        json!(["verdict", "rule", "module"])
    );
    assert_eq!(view["imports"][0]["before"]["from_node"], "module:domain");
    assert_eq!(
        view["imports"][0]["after"]["from_node"],
        "module:domain-renamed"
    );
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == "module:domain" && node["status"] == "removed")
    );
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == "module:domain-renamed" && node["status"] == "added")
    );

    let mut known = document(vec![edge(3)], rules());
    known["config"]["rules"]["known"] = json!({"path":"layout.known.yaml","entries":[
        {"from":"src/domain/a.ts","to":"src/adapters/db.ts","rule":"no-adapters"}
    ]});
    let view = projected(&document(vec![edge(3)], rules()), &known);
    assert_eq!(view["comparison"]["policy_changed"], true);
    assert_eq!(view["imports"][0]["changes"], json!(["rule"]));
    assert_eq!(view["imports"][0]["after"]["evidence"][0]["known"], true);
}

#[test]
fn duplicate_destinations_retain_distinct_policy_and_original_destination_evidence() {
    let mut a = edge(3);
    a.original_to = Some("src/shared/one.ts".into());
    let mut b = edge(3);
    b.original_to = Some("src/shared/two.ts".into());
    let policy = policy(
        json!([{"id":"one-only","from":"src/domain/**","to":"src/shared/one.ts","reason":"Use the other entry."}]),
    );
    let old = document(vec![a.clone(), b.clone()], policy.clone());
    let view = projected(&old, &document(vec![b, a], policy));
    assert_eq!(view["imports"].as_array().unwrap().len(), 1);
    let evidence = view["imports"][0]["before"]["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 2);
    assert!(
        evidence
            .iter()
            .any(|e| e["original_to"] == "src/shared/one.ts" && e["verdict"] == "forbidden")
    );
    assert!(
        evidence
            .iter()
            .any(|e| e["original_to"] == "src/shared/two.ts" && e["verdict"] != "forbidden")
    );
    assert_eq!(view["imports"][0]["status"], "unchanged");
}

#[test]
fn decisions_remain_associated_with_their_original_import_occurrences() {
    let mut one = edge(3);
    one.original_to = Some("src/shared/one.ts".into());
    one.specifier = Some("../shared/one".into());
    let mut two = edge(5);
    two.original_to = Some("src/shared/two.ts".into());
    two.specifier = Some("../shared/two".into());
    for both_forbidden in [false, true] {
        let mut before_policy = policy(json!([{
            "id":"one-entry","from":"src/domain/**","to":"src/shared/one.ts","reason":"First decision."
        }]));
        let second = json!({"id":"two-entry","from":"src/domain/**","to":"src/shared/two.ts","reason":"Second decision."});
        if both_forbidden {
            before_policy["forbid"].as_array_mut().unwrap().push(second);
        } else {
            before_policy["allow"] = json!([second]);
        }
        let mut after_policy = before_policy.clone();
        after_policy["forbid"][0]["to"] = json!("src/shared/two.ts");
        if both_forbidden {
            after_policy["forbid"][1]["to"] = json!("src/shared/one.ts");
        } else {
            after_policy["allow"][0]["to"] = json!("src/shared/one.ts");
        }
        let before = document(vec![one.clone(), two.clone()], before_policy);
        let after = document(vec![one.clone(), two.clone()], after_policy);
        let view = projected(&before, &after);
        assert_eq!(view["comparison"]["policy_changed"], true);
        assert_eq!(view["imports"][0]["status"], "changed");
        assert_eq!(
            view["imports"][0]["changes"],
            if both_forbidden {
                json!(["rule"])
            } else {
                json!(["verdict", "rule"])
            }
        );
        let row = &view["imports"][0];
        let decision = |side: &str| {
            row[side]["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["original_to"] == "src/shared/one.ts")
                .unwrap()["rule"]
                .clone()
        };
        assert_eq!(decision("before"), "one-entry");
        assert_eq!(decision("after"), "two-entry");
    }
}

#[test]
fn spelling_or_original_destination_edits_with_identical_decisions_are_evidence_only() {
    for written_target_changes in [false, true] {
        let mut before_edge = edge(3);
        let mut after_edge = edge(3);
        let selected_policy = if written_target_changes {
            before_edge.original_to = Some("src/shared/one.ts".into());
            after_edge.original_to = Some("src/shared/two.ts".into());
            let mut selected = policy(json!([]));
            selected["allow"] =
                json!([{"id":"shared-entry","from":"src/domain/**","to":"src/shared/**"}]);
            selected
        } else {
            rules()
        };
        after_edge.specifier = Some("@app/entry".into());
        let before = document(vec![before_edge], selected_policy.clone());
        let after = document(vec![after_edge], selected_policy);
        let view = projected(&before, &after);
        assert_eq!(view["comparison"]["policy_changed"], false);
        assert_eq!(view["imports"][0]["status"], "changed");
        assert_eq!(view["imports"][0]["changes"], json!(["evidence"]));
    }
}

#[test]
fn packages_and_globals_keep_authoritative_kinds_and_are_not_hidden_from_the_view() {
    let mut config: Config = serde_json::from_value(json!({
        "depth":2,"source_root":"src","excludes":[],"include_tests":false,
        "rules":{"path":"layout.yaml","sha256":"fixture","doc":policy(json!([
            {"id":"no-pg","from":"src/domain/**","to":"pkg:pg"},
            {"id":"no-window","from":"src/domain/**","to":"global:window"}
        ]))}
    }))
    .unwrap();
    config.import_records = Some(vec![ImportRecord {
        from: "src/domain/a.ts".into(),
        specifier: "pg".into(),
        external: Some("pg".into()),
        line: Some(2),
        ..Default::default()
    }]);
    config.globals = Some(vec![GlobalUse {
        from: "src/domain/a.ts".into(),
        name: "window".into(),
        line: 3,
    }]);
    let doc = analyzed(config, vec![]);
    let view = projected(&doc, &doc);
    assert_eq!(view["imports"].as_array().unwrap().len(), 2);
    for (kind, label) in [("package", "pkg:pg"), ("global", "global:window")] {
        let node = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == kind)
            .unwrap();
        assert_eq!(node["label"], label);
        assert!(node["id"].as_str().unwrap().starts_with("external:"));
        assert_eq!(node["before"]["files"], json!([]));
        let row = view["imports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["after"]["to_node"] == node["id"])
            .unwrap();
        assert_eq!(row["after"]["evidence"][0]["verdict"], "forbidden");
    }
}

#[test]
fn missing_and_different_original_analyzers_are_visible_after_current_recomputation() {
    let mut old = document(vec![edge(3)], rules());
    let mut new = old.clone();
    old["tool"]["version"] = json!("old-analyzer");
    new["tool"]["version"] = json!("new-analyzer");
    let view = projected(&old, &new);
    assert_eq!(view["comparison"]["status"], "unverified");
    assert_eq!(view["comparison"]["before"]["tool_version"], "old-analyzer");
    assert_eq!(view["comparison"]["after"]["tool_version"], "new-analyzer");
    old.as_object_mut().unwrap().remove("tool");
    old["source"]["collector"]["version"] = Value::Null;
    old["source"]["scan_root"] = Value::Null;
    old["config"]["collection"] = Value::Null;
    let view = projected(&old, &new);
    assert_eq!(view["comparison"]["status"], "unverified");
    assert!(view["comparison"]["before"]["tool_version"].is_null());
    let warnings = view["comparison"]["warnings"].to_string();
    for missing in [
        "analyzer version",
        "collector version",
        "scan root",
        "collection settings",
    ] {
        assert!(warnings.contains(missing), "{missing}: {warnings}");
    }
}

#[test]
fn incompatible_recorded_collection_inputs_fail_before_creating_html() {
    let f = Fixture::new();
    let old = document(vec![edge(3)], rules());
    for (path, value) in [
        ("/source/language", json!("python")),
        ("/source/collector/name", json!("other")),
        ("/source/collector/version", json!("different")),
        ("/source/collector/type_marking", json!("unavailable")),
        ("/source/scan_root", json!("subproject")),
        ("/config/depth", json!(3)),
        ("/config/excludes", json!(["src/missing/**"])),
        ("/config/collection/engine", json!("reference")),
    ] {
        let mut new = old.clone();
        // Optional metadata may not be serialized in the original document.
        if path == "/source/collector/type_marking" {
            new["source"]["collector"]["type_marking"] = value;
        } else {
            *new.pointer_mut(path).unwrap() = value;
        }
        f.save("before.json", &old);
        f.save("after.json", &new);
        let out = f.run(&[
            "diff",
            "before.json",
            "after.json",
            "--html",
            "changes.html",
        ]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{path}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!f.0.join("changes.html").exists(), "{path}");
        // Existing no-HTML comparisons retain their previous semantics even across scopes.
        let normal = f.run(&["diff", "before.json", "after.json"]);
        assert!(
            normal.status.success(),
            "{path}: {}",
            String::from_utf8_lossy(&normal.stderr)
        );
    }
    let mut new = old.clone();
    new["config"]["excluded_files"] = json!(["derived/changed.ts"]);
    new["source"]["collector"]["cruised"] = json!(999);
    assert_eq!(
        f.view(&old, &new)["comparison"]["status"],
        "recorded-compatible"
    );
}

#[test]
fn enabling_host_global_capture_is_a_collection_change_even_without_a_code_edit() {
    let f = Fixture::new();
    let before = document(vec![edge(3)], rules());
    let mut global_policy = rules();
    global_policy["forbid"].as_array_mut().unwrap().push(json!({
        "id":"no-window","from":"src/domain/**","to":"global:window"
    }));
    let after = document(vec![edge(3)], global_policy);
    f.save("before.json", &before);
    f.save("after.json", &after);
    let out = f.run(&[
        "diff",
        "before.json",
        "after.json",
        "--html",
        "changes.html",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("collection selection"));
    assert!(!f.0.join("changes.html").exists());
}

#[test]
fn html_preserves_full_evidence_before_brief_gate_and_recomputes_forged_verdicts() {
    let f = Fixture::new();
    let old = document(vec![], rules());
    let mut new = document((1..=30).map(edge).collect(), rules());
    new["summary"]["forbidden"] = json!(0);
    for edge in new["edges"].as_array_mut().unwrap() {
        edge["verdict"] = json!("allowed");
        edge["rule"] = Value::Null;
    }
    f.save("before.json", &old);
    f.save("after.json", &new);
    let out = f.run(&[
        "--brief",
        "diff",
        "before.json",
        "after.json",
        "--html",
        "changes.html",
        "--fail-on",
        "new-forbidden",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(brief["schema"], "archgraph/brief/v1");
    let view = embedded(&fs::read_to_string(f.0.join("changes.html")).unwrap());
    assert_eq!(view["imports"].as_array().unwrap().len(), 1);
    assert_eq!(
        view["imports"][0]["after"]["evidence"]
            .as_array()
            .unwrap()
            .len(),
        30
    );
    assert_eq!(
        view["imports"][0]["after"]["evidence"][0]["verdict"],
        "forbidden"
    );
}

#[test]
fn html_output_never_replaces_an_input_and_rejects_unsupported_modes() {
    let f = Fixture::new();
    let old = document(vec![edge(3)], rules());
    let new = document(vec![edge(9)], rules());
    let old_path = f.save("before.json", &old);
    let new_path = f.save("after.json", &new);
    let old_bytes = fs::read(&old_path).unwrap();
    let new_bytes = fs::read(&new_path).unwrap();
    for output in ["before.json", "./after.json", ".archgraph/../before.json"] {
        let out = f.run(&["diff", "before.json", "after.json", "--html", output]);
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(fs::read(&old_path).unwrap(), old_bytes);
        assert_eq!(fs::read(&new_path).unwrap(), new_bytes);
    }
    let legacy = f.run(&[
        "--legacy-v1",
        "diff",
        "before.json",
        "after.json",
        "--html",
        "changes.html",
    ]);
    assert_eq!(legacy.status.code(), Some(2));
    assert!(!f.0.join("changes.html").exists());
    let service = archgraph::services::analyze(
        serde_json::from_value(json!({"version":1,"services":[],"edges":[]})).unwrap(),
        "fixture",
    )
    .unwrap();
    f.save("services.json", &serde_json::to_value(service).unwrap());
    for before in ["before.json", "services.json"] {
        let out = f.run(&["diff", before, "services.json", "--html", "changes.html"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(!f.0.join("changes.html").exists());
    }
    let mut malformed = old.clone();
    malformed["tool"]["version"] = json!(42);
    f.save("bad-metadata.json", &malformed);
    assert_eq!(
        f.run(&[
            "diff",
            "before.json",
            "bad-metadata.json",
            "--html",
            "changes.html"
        ])
        .status
        .code(),
        Some(2)
    );
    assert!(!f.0.join("changes.html").exists());
}

#[cfg(unix)]
#[test]
fn output_aliases_through_links_and_linked_parents_preserve_both_inputs() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let doc = document(vec![edge(3)], rules());
    f.save("before.json", &doc);
    f.save("after.json", &doc);
    symlink(f.0.join("before.json"), f.0.join("alias.html")).unwrap();
    symlink(&f.0, f.0.join("linked-parent")).unwrap();
    for output in ["alias.html", "linked-parent/before.json"] {
        let out = f.run(&["diff", "before.json", "after.json", "--html", output]);
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(
            fs::read_to_string(f.0.join("before.json")).unwrap(),
            doc.to_string()
        );
        assert_eq!(
            fs::read_to_string(f.0.join("after.json")).unwrap(),
            doc.to_string()
        );
    }
}

#[test]
fn embedded_repository_strings_are_literal_and_the_report_stays_offline() {
    let evil = "src/domain/</script><script>wrong()</script>{{COMPARISON}}.ts";
    let (root, input) = common::synthetic(
        "comparison-escaped",
        2,
        &[(evil.into(), "src/adapters/db.ts".into())],
    );
    let (_, text, _) = common::scan(&[
        &root,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let f = Fixture::new();
    f.write("before.json", &text);
    f.write("after.json", &text);
    let out = f.run(&[
        "diff",
        "before.json",
        "after.json",
        "--html",
        "changes.html",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let html = fs::read_to_string(f.0.join("changes.html")).unwrap();
    let view = embedded(&html);
    assert_eq!(view["imports"][0]["from"], evil);
    assert!(!html.contains(evil));
    assert!(html.contains("default-src 'none'"));
    let template = include_str!("../assets/comparison.html");
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "new Function",
        "fetch(",
        "Math.random",
        "Date.now",
    ] {
        assert!(!template.contains(forbidden), "{forbidden}");
    }
}
