mod common;

use common::{archgraph, fixture, scan, validate, write_tmp};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn success(args: &[&str]) -> std::process::Output {
    let out = archgraph(args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn edge_projection(doc: &Value) -> Vec<(String, String, String)> {
    doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            (
                edge["from"].as_str().unwrap().into(),
                edge["to"].as_str().unwrap().into(),
                edge["kind"].as_str().unwrap().into(),
            )
        })
        .collect()
}

#[test]
fn recorded_elixir_reaches_shared_analysis_rules_and_saved_graph_commands() {
    let root = fixture("elixir-basic");
    let input = root.join("elixir.json");
    let rules = write_tmp("elixir-basic-layout.yaml", "version: 1\n");
    let args = [
        root.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
        "--rules",
        &rules,
    ];
    let (doc, stdout, _) = scan(&args);
    assert_eq!(doc["source"]["language"], "elixir");
    assert_eq!(doc["source"]["collector"]["name"], "elixir-ast");
    assert_eq!(doc["config"]["source_root"], ".");
    assert_eq!(doc["files"].as_array().unwrap().len(), 6);
    assert_eq!(edge_projection(&doc).len(), 5);
    assert_eq!(doc["cycles"].as_array().unwrap().len(), 1);
    assert!(!doc["type_only_cycles"].as_array().unwrap().is_empty());
    assert!(
        doc["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "apps/worker/lib/demo_worker.ex")
    );
    let graph = write_tmp("elixir-basic-graph.json", &stdout);
    success(&["check", &graph]);
    let output = success(&[
        "simulate",
        &graph,
        "--move",
        "lib/demo/a.ex",
        "lib/moved/a.ex",
    ]);
    validate(&serde_json::from_slice::<Value>(&output.stdout).unwrap());
    let output = success(&["diff", &graph, &graph]);
    let difference: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate(&difference);
    assert!(difference["new_cycles"].as_array().unwrap().is_empty());
    let html = write_tmp("elixir-basic.html", "");
    success(&["render", &graph, "-o", &html]);
    assert!(fs::read_to_string(html).unwrap().contains("lib/demo/a.ex"));
    for command in [
        vec![
            "context",
            "--graph",
            &graph,
            "--root",
            root.to_str().unwrap(),
            "--task",
            "change Demo.A",
            "--files",
            "lib/demo/a.ex",
            "--budget",
            "4000",
        ],
        vec!["tour", &graph],
        vec!["infer-layout", &graph],
    ] {
        let output = success(&command);
        assert!(!output.stdout.is_empty());
    }
    let (legacy, _, _) = scan(&[&args[..], &["--legacy-v1"]].concat());
    assert_eq!(legacy["schema"], "archgraph/v1");

    let (included, _, _) = scan(&[&args[..], &["--include-tests"]].concat());
    assert_eq!(included["files"].as_array().unwrap().len(), 7);
    assert_eq!(included["cycles"], doc["cycles"]);
    assert_eq!(included["test_coverage"]["covered"], 1);
    assert!(
        included["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "test/demo_test.exs" && file["class"] == "tests")
    );

    let package_rules = write_tmp(
        "elixir-basic-package-layout.yaml",
        "version: 1\npkg-category:\n  runtime: [elixir]\nforbid:\n  - {from: mix.exs, to: 'category:runtime'}\n",
    );
    let checked = write_tmp("elixir-basic-checked.json", "");
    let output = archgraph(&["check", &graph, "--rules", &package_rules, "-o", &checked]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let checked: Value = serde_json::from_slice(&fs::read(checked).unwrap()).unwrap();
    validate(&checked);
    assert_eq!(checked["summary"]["forbidden"], 1);
    assert_eq!(checked["external_edges"][0]["verdict"], "forbidden");

    let (symbols, _, _) = scan(&[&args[..], &["--symbols"]].concat());
    assert_eq!(symbols["symbols"]["complete"], false);
    assert!(
        !symbols["symbols"]["limitations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=archgraph-test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[test]
#[ignore = "needs Elixir 1.14+ / OTP 25+; run with --include-ignored"]
fn live_elixir_matches_recorded_and_history_never_evaluates_the_project() {
    let elixir = std::env::var("ARCHGRAPH_ELIXIR").unwrap_or_else(|_| "elixir".into());
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("elixir-live-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    copy_tree(&fixture("elixir-basic"), &root);
    let input = root.join("elixir.json");
    for flags in [&[][..], &["--include-tests"][..]] {
        let (live, _, _) =
            scan(&[&[root.to_str().unwrap(), "--elixir", &elixir][..], flags].concat());
        let (recorded, _, _) = scan(
            &[
                &[
                    root.to_str().unwrap(),
                    "--collector-input",
                    input.to_str().unwrap(),
                ][..],
                flags,
            ]
            .concat(),
        );
        assert_eq!(edge_projection(&live), edge_projection(&recorded));
        assert_eq!(live["files"], recorded["files"]);
        assert!(!root.join("ELIXIR_PROJECT_EXECUTED").exists());
        assert!(!root.join("_build").exists());
    }
    let poison = root.join("poison.exs");
    fs::write(
        &poison,
        "File.write!(\"ELIXIR_ENV_EXECUTED\", \"must never execute\")\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "scan",
            root.to_str().unwrap(),
            "--language",
            "elixir",
            "--elixir",
            &elixir,
        ])
        .env("ELIXIR_ERL_OPTIONS", format!("-r {}", poison.display()))
        .env("ELIXIR_ERL_PARAMETERS", format!("-r {}", poison.display()))
        .env("ERL_AFLAGS", "-eval 'halt(19).'")
        .env("ERL_LIBS", &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    validate(&serde_json::from_slice::<Value>(&output.stdout).unwrap());
    assert!(!root.join("ELIXIR_ENV_EXECUTED").exists());
    assert!(!root.join("ELIXIR_PROJECT_EXECUTED").exists());
    fs::remove_file(poison).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "before"]);
    fs::write(
        root.join("lib/demo/b.ex"),
        "defmodule Demo.B do\n  def run, do: :ok\nend\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "remove runtime cycle"]);
    let impact = success(&[
        "impact",
        "HEAD~1..HEAD",
        "--root",
        root.to_str().unwrap(),
        "--elixir",
        &elixir,
        "--format",
        "json",
    ]);
    let impact: Value = serde_json::from_slice(&impact.stdout).unwrap();
    validate(&impact);
    assert!(
        impact["selected_tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "test/demo_test.exs")
    );
    let output = root
        .parent()
        .unwrap()
        .join(format!("elixir-history-{}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    success(&[
        "history",
        root.to_str().unwrap(),
        "--elixir",
        &elixir,
        "--sample",
        "commits",
        "--limit",
        "2",
        "--out-dir",
        output.to_str().unwrap(),
    ]);
    let history: Value =
        serde_json::from_slice(&fs::read(output.join("history.json")).unwrap()).unwrap();
    validate(&history);
    assert_eq!(history["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(history["snapshots"][0]["graph"]["summary"]["cycles"], 1);
    assert_eq!(history["snapshots"][1]["graph"]["summary"]["cycles"], 0);
    assert!(!root.join("ELIXIR_PROJECT_EXECUTED").exists());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(output).unwrap();
}
