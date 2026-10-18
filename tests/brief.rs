mod common;

#[path = "../src/cli/brief.rs"]
mod brief;

use serde_json::{Value, json};
use std::process::Command;

#[test]
fn brief_caps_all_array_entries_and_explains_omissions() {
    let value = json!({"kind":"impact", "summary":{"files": 300}, "affected_modules": (0..100).map(|index| json!({"id":format!("module{index}"), "depth":1})).collect::<Vec<_>>(), "selected_tests": ["tests/a.ts"]});
    let projected = brief::project(&value);
    assert_eq!(projected["result"]["summary"]["files"], 300);
    assert_eq!(
        projected["result"]["affected_modules"]
            .as_array()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(projected["omitted"]["/affected_modules"]["items"], 80);
    assert_eq!(projected["omitted"]["/selected_tests"]["items"], 1);
    assert_eq!(
        serde_json::to_vec(&projected).unwrap(),
        serde_json::to_vec(&brief::project(&value)).unwrap()
    );
}

#[test]
fn brief_bounds_nested_inventory_and_long_context_bodies() {
    let value = json!({"kind":"context", "entries": (0..50).map(|_| json!({"content":"x".repeat(100_000), "names":["a", "b", "c"]})).collect::<Vec<_>>()});
    let projected = brief::project(&value);
    assert!(serde_json::to_vec(&projected).unwrap().len() < 16_384);
    assert!(
        projected["omitted"]
            .as_object()
            .unwrap()
            .values()
            .any(|item| item["characters"] == 99_488)
    );
}

#[test]
fn fresh_scans_are_byte_identical_and_timestamp_is_reproducible() {
    // Recorded collection isolates reproducibility from collector installation/version.
    let fixture = common::fixture("ts-cycles");
    let scan = || {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .args([
                "scan",
                fixture.to_str().unwrap(),
                "--collector-input",
                fixture.join("depcruise.json").to_str().unwrap(),
            ])
            .env("SOURCE_DATE_EPOCH", "946684800")
            .output()
            .unwrap()
    };
    let first = scan();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = scan();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    let value: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(value["source"]["generated_at"], "2000-01-01T00:00:00Z");
    let invalid = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "scan",
            fixture.to_str().unwrap(),
            "--collector-input",
            fixture.join("depcruise.json").to_str().unwrap(),
        ])
        .env("SOURCE_DATE_EPOCH", "bad")
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unset it"));
}

#[path = "../src/cli/explain.rs"]
mod explain;

#[test]
fn explain_uses_graph_evidence_and_unknown_subject_says_what_to_do() {
    let (value, _, _) = common::scan_recorded("ts-cycles", &[]);
    let doc = archgraph::restore_scan(serde_json::from_value(value).unwrap()).unwrap();
    let edge = doc.edges.first().unwrap();
    let result = explain::explain(&doc, &edge.from).unwrap();
    assert!(
        result["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value["to"] == edge.to)
    );
    assert_eq!(result["file"]["path"], edge.from);
    let module = &doc
        .files
        .iter()
        .find(|file| file.path == edge.from)
        .unwrap()
        .module;
    let module_result = explain::explain(&doc, module).unwrap();
    assert_eq!(module_result["module"]["id"], *module);
    let error = explain::explain(&doc, "does/not/exist.ts")
        .unwrap_err()
        .to_string();
    assert!(error.contains("use report"));
    assert!(error.contains("repository-relative"));
}

#[test]
fn timestamp_defaults_to_head_and_to_epoch_outside_git() {
    let fixture = common::fixture("ts-cycles");
    let run = |epoch: Option<&str>, outside_git: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_archgraph"));
        command.args([
            "scan",
            fixture.to_str().unwrap(),
            "--collector-input",
            fixture.join("depcruise.json").to_str().unwrap(),
        ]);
        command.env_remove("SOURCE_DATE_EPOCH");
        if let Some(epoch) = epoch {
            command.env("SOURCE_DATE_EPOCH", epoch);
        }
        if outside_git {
            command.env("GIT_CEILING_DIRECTORIES", fixture.parent().unwrap());
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["source"]["generated_at"].clone()
    };
    let git = Command::new("git")
        .args([
            "-C",
            fixture.to_str().unwrap(),
            "show",
            "-s",
            "--format=%ct",
            "HEAD",
        ])
        .output()
        .unwrap();
    assert!(git.status.success());
    let seconds = String::from_utf8(git.stdout).unwrap();
    assert_eq!(run(None, false), run(Some(seconds.trim()), false));
    assert_eq!(run(None, true), "1970-01-01T00:00:00Z");
}

#[test]
fn brief_keeps_findings_ahead_of_large_per_module_metrics() {
    let modules: serde_json::Map<String, Value> = (0..2000)
        .map(|index| (format!("module{index}"), json!({"ca": 3, "ce": 2})))
        .collect();
    let value = json!({"kind":"scan", "summary":{"cycles":1,"modules":modules}, "findings":[{"subject":"src/app", "kind":"cycle"}], "files":["a.ts", "b.ts"]});
    let result = brief::project(&value);
    assert_eq!(result["result"]["findings"][0]["subject"], "src/app");
    assert_eq!(result["omitted"]["/summary/modules"]["fields"], 2000);
    assert_eq!(result["counts"]["files"], 2);
}

#[test]
fn brief_check_preserves_policy_exit_and_report_writes_complete_artifacts() {
    let (_, scan, _) = common::scan_recorded("ts-hexagonal", &[]);
    let graph = common::write_tmp("brief-cli/check-graph.json", &scan);
    let fixture = common::fixture("ts-hexagonal");
    let rules = fixture.join("layout.yaml");
    let check = common::archgraph(&[
        "--brief",
        "check",
        &graph,
        "--rules",
        rules.to_str().unwrap(),
    ]);
    assert_eq!(
        check.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let value: Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(value["kind"], "check");
    assert!(value["result"]["summary"]["forbidden"].as_u64().unwrap() > 0);
    let permissive = common::write_tmp("brief-cli/permissive.yaml", "version: 1\n");
    let clear = common::archgraph(&["--brief", "check", &graph, "--rules", &permissive]);
    assert_eq!(
        clear.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&clear.stderr)
    );
    let clear_value: Value = serde_json::from_slice(&clear.stdout).unwrap();
    assert_eq!(clear_value["result"]["summary"]["forbidden"], 0);
    let missing = common::archgraph(&[
        "--brief",
        "check",
        &graph,
        "--rules",
        "/nonexistent/brief-layout.yaml",
    ]);
    assert_eq!(missing.status.code(), Some(2));
    let output = std::path::Path::new(&graph)
        .parent()
        .unwrap()
        .join("report");
    let report = common::archgraph(&[
        "--brief",
        "report",
        fixture.to_str().unwrap(),
        "--collector-input",
        fixture.join("depcruise.json").to_str().unwrap(),
        "--out-dir",
        output.to_str().unwrap(),
        "--no-provenance",
    ]);
    assert_eq!(report.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(value["kind"], "report");
    let artifact: Value =
        serde_json::from_slice(&std::fs::read(output.join("archgraph.json")).unwrap()).unwrap();
    assert_eq!(artifact["kind"], "scan");
    assert!(output.join("archgraph.html").is_file());
}

#[test]
fn brief_surface_and_context_override_markdown() {
    let fixture = common::fixture("ts-cycles");
    let scanned = common::archgraph(&[
        "scan",
        fixture.to_str().unwrap(),
        "--engine",
        "native",
        "--symbols",
    ]);
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stderr)
    );
    let graph = common::write_tmp(
        "brief-cli/symbols-graph.json",
        std::str::from_utf8(&scanned.stdout).unwrap(),
    );
    for args in [
        vec!["--brief", "surface", graph.as_str(), "--format", "markdown"],
        vec![
            "--brief",
            "context",
            "--graph",
            graph.as_str(),
            "--root",
            fixture.to_str().unwrap(),
            "--task",
            "cycles",
            "--format",
            "markdown",
        ],
    ] {
        let output = common::archgraph(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["schema"], "archgraph/brief/v1");
        assert!(matches!(
            value["kind"].as_str(),
            Some("surface" | "context")
        ));
    }
}

#[test]
fn brief_impact_overrides_markdown_and_preserves_depth_exit() {
    let root = std::env::temp_dir().join(format!("archgraph-brief-impact-{}", std::process::id()));
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.ts"), "export const value = 1;\n").unwrap();
    std::fs::write(
        root.join("src/b.ts"),
        "import {value} from './a'; export const other = value;\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&root)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "initial"]);
    std::fs::write(root.join("src/a.ts"), "export const value = 2;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "change"]);
    let scan = common::archgraph(&["scan", root.to_str().unwrap(), "--engine", "native"]);
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let graph = common::write_tmp(
        "brief-cli/impact-graph.json",
        std::str::from_utf8(&scan.stdout).unwrap(),
    );
    let output = common::archgraph(&[
        "--brief",
        "impact",
        "HEAD~1..HEAD",
        "--root",
        root.to_str().unwrap(),
        "--graph",
        &graph,
        "--format",
        "markdown",
        "--fail-on-depth",
        "0",
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["kind"], "impact");
    assert_eq!(value["result"]["changed_files"][0], "src/a.ts");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn brief_tour_captures_text_as_bounded_json() {
    let graph = common::scan_file("ts-cycles", &[]);
    let output = common::archgraph(&["--brief", "tour", &graph, "--agent"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "archgraph/brief/v1");
    assert!(!value["result"]["text"].as_str().unwrap().is_empty());
    assert!(value["result"]["text"].as_str().unwrap().chars().count() <= 512);
}

#[test]
fn brief_infer_layout_captures_draft_and_apply_keeps_complete_file() {
    let graph = common::scan_file("ts-hexagonal", &[]);
    let output = common::archgraph(&["--brief", "infer-layout", &graph]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "archgraph/brief/v1");
    assert!(
        value["result"]["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("draft")
    );
    let applied = std::path::Path::new(&graph).with_file_name("brief-inferred-layout.yaml");
    if applied.exists() {
        std::fs::remove_file(&applied).unwrap();
    }
    let full = common::archgraph(&["infer-layout", &graph]);
    let output = common::archgraph(&[
        "--brief",
        "infer-layout",
        &graph,
        "--apply",
        applied.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(applied).unwrap(), full.stdout);
}

#[test]
fn brief_services_captures_markdown_but_preserves_complete_report_export() {
    let manifest = common::write_tmp(
        "brief-cli/services.json",
        r#"{"version":1,"services":["api","billing"],"edges":[{"from":"api","to":"billing","kind":"http","source":{"file":"calls.ts","line":1}}]}"#,
    );
    let report = std::path::Path::new(&manifest).with_file_name("services-report.json");
    let output = common::archgraph(&[
        "--brief",
        "services",
        &manifest,
        "-o",
        report.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "archgraph/brief/v1");
    assert!(!value["result"]["text"].as_str().unwrap().is_empty());
    let full: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(full["kind"], "services");
    assert_eq!(full["edges"].as_array().unwrap().len(), 1);
    assert_eq!(full["schema"], "archgraph/v1.2");
}
