mod common;

use common::{archgraph, validate};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

#[test]
fn package_verdicts_reach_check_simulate_diff_and_legacy_export() {
    let root =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("exposure-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let graph = root.join("graph.json");
    let checked = root.join("checked.json");
    let layout = root.join("layout.yaml");
    let draft = root.join("draft.yaml");
    let input = json!({
        "schema":"archgraph/v1.1", "kind":"scan",
        "source":{"language":"typescript", "collector":{"name":"dependency-cruiser","cruised":2}, "generated_at":"2026-10-18T00:00:00Z"},
        "config":{"depth":2,"source_root":"src","excludes":[],"include_tests":false},
        "files":[{"path":"src/domain/a.ts","module":"domain","barrel":false},
                 {"path":"src/adapter/b.ts","module":"adapter","barrel":false}],
        "edges":[],
        "imports":[{"from":"src/domain/a.ts","specifier":"typeorm","external":"typeorm","kind":"runtime","line":3}]
    });
    fs::write(&graph, serde_json::to_vec(&input).unwrap()).unwrap();
    fs::write(&layout, "version: 1\npkg-category:\n  orm: [typeorm, prisma]\nforbid:\n  - {from: 'src/domain/**', to: 'category:orm'}\n").unwrap();
    let args = [
        "check",
        graph.to_str().unwrap(),
        "--rules",
        layout.to_str().unwrap(),
        "--output",
        checked.to_str().unwrap(),
    ];
    let output = archgraph(&args);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("forbidden src/domain/a.ts -> pkg:typeorm"),
        "{text}"
    );
    assert!(text.contains("1 forbidden, 0 unplanned"));
    let doc: Value = serde_json::from_slice(&fs::read(&checked).unwrap()).unwrap();
    validate(&doc);
    assert_eq!(
        doc["external_exposure"]["modules"][1]["external_percent"],
        100.0
    );
    let simulation = archgraph(&[
        "simulate",
        checked.to_str().unwrap(),
        "--move",
        "src/domain/a.ts",
        "src/adapter/a.ts",
    ]);
    assert!(
        simulation.status.success(),
        "{}",
        String::from_utf8_lossy(&simulation.stderr)
    );
    let simulation: Value = serde_json::from_slice(&simulation.stdout).unwrap();
    validate(&simulation);
    assert_eq!(simulation["removed_forbidden"].as_array().unwrap().len(), 1);
    let difference = archgraph(&[
        "diff",
        graph.to_str().unwrap(),
        checked.to_str().unwrap(),
        "--fail-on",
        "new-forbidden",
    ]);
    assert_eq!(difference.status.code(), Some(1));
    let difference: Value = serde_json::from_slice(&difference.stdout).unwrap();
    validate(&difference);
    assert_eq!(difference["new_forbidden"].as_array().unwrap().len(), 1);
    let legacy = archgraph(&[&args[..], &["--legacy-v1"]].concat());
    assert_eq!(legacy.status.code(), Some(1));
    validate(&serde_json::from_slice(&fs::read(&checked).unwrap()).unwrap());
    fs::write(
        &draft,
        "version: 1\nforbid:\n  - {from: '**', to: 'pkg:**'}\n",
    )
    .unwrap();
    let mut no_ledger = input;
    no_ledger.as_object_mut().unwrap().remove("imports");
    fs::write(&graph, serde_json::to_vec(&no_ledger).unwrap()).unwrap();
    for args in [
        vec![
            "check",
            graph.to_str().unwrap(),
            "--rules",
            draft.to_str().unwrap(),
        ],
        vec![
            "simulate",
            graph.to_str().unwrap(),
            "--rules",
            draft.to_str().unwrap(),
            "--move",
            "src/domain/a.ts",
            "src/adapter/a.ts",
        ],
    ] {
        let output = archgraph(&args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("require import accounting"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn global_verdicts_reach_check_simulate_diff_and_legacy_export() {
    let root =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("global-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = |name: &str| root.join(name).to_str().unwrap().to_owned();
    let (graph, checked, layout) = (
        path("graph.json"),
        path("checked.json"),
        path("layout.yaml"),
    );
    let input = json!({
        "schema":"archgraph/v1.2", "kind":"scan",
        "source":{"language":"typescript", "collector":{"name":"tree-sitter-typescript","cruised":3}, "generated_at":"2026-10-22T00:00:00Z"},
        "config":{"depth":2,"source_root":"src","excludes":[],"include_tests":false},
        "files":[{"path":"src/domain/a.ts","module":"domain","barrel":false},
                 {"path":"src/domain/process","module":"domain","barrel":false},
                 {"path":"src/adapter/b.ts","module":"adapter","barrel":false}],
        "edges":[{"from":"src/domain/a.ts","to":"src/domain/process","kind":"runtime"}],
        "imports":[{"from":"src/domain/a.ts","specifier":"typeorm","external":"typeorm","kind":"runtime","line":3}],
        "globals":[{"from":"src/domain/a.ts","name":"process","line":4},
                   {"from":"src/domain/a.ts","name":"console","line":9},
                   {"from":"src/adapter/b.ts","name":"process","line":2}]
    });
    fs::write(&graph, serde_json::to_vec(&input).unwrap()).unwrap();
    let check = |rules: &str, extra: &[&str]| {
        fs::write(&layout, format!("version: 1\n{rules}")).unwrap();
        archgraph(
            &[
                &["check", &graph, "--rules", &layout, "--output", &checked],
                extra,
            ]
            .concat(),
        )
    };
    let forbidden = |output: &std::process::Output| -> Vec<String> {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.starts_with("forbidden "))
            .map(String::from)
            .collect()
    };

    // A layout without `global:` gains no node or edge for the recorded globals.
    let output = check(
        "forbid:\n  - {from: 'src/domain/**', to: 'pkg:typeorm'}\n",
        &[],
    );
    assert_eq!(output.status.code(), Some(1));
    let doc: Value = serde_json::from_slice(&fs::read(&checked).unwrap()).unwrap();
    assert_eq!(
        doc["external_nodes"],
        json!([{"id":"external:typescript:typeorm","external":true,"package":"typeorm"}])
    );
    assert_eq!(doc["external_edges"].as_array().unwrap().len(), 1);

    let rule = "forbid:\n  - {id: domain-no-env, from: 'src/domain/**', to: 'global:process'}\n";
    let output = check(rule, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        forbidden(&output),
        ["forbidden src/domain/a.ts -> global:process (domain-no-env)"]
    );
    let doc: Value = serde_json::from_slice(&fs::read(&checked).unwrap()).unwrap();
    validate(&doc);
    assert_eq!(
        doc["external_nodes"],
        json!([{"id":"external:typescript:typeorm","external":true,"package":"typeorm"},
               {"id":"external:typescript:global:process","external":true,"package":"process","global":true}])
    );
    let edge = doc["external_edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["from"] == "src/domain/a.ts" && e["specifier"] == "process")
        .unwrap();
    assert_eq!(
        (&edge["kind"], &edge["line"]),
        (&json!("runtime"), &json!(4))
    );
    let sarif = check(rule, &["--format", "sarif"]);
    let sarif: Value = serde_json::from_slice(&sarif.stdout).unwrap();
    assert_eq!(
        sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"]["startLine"],
        4
    );

    // `pkg:**` never selects a global, and `global:*` never selects a package.
    let output = check(
        "forbid:\n  - {from: 'src/domain/**', to: 'pkg:**'}\nallow:\n  - {from: '**', to: 'global:*'}\n",
        &[],
    );
    assert_eq!(
        forbidden(&output),
        ["forbidden src/domain/a.ts -> pkg:typeorm (forbid[0])"]
    );
    let output = check(
        "forbid:\n  - {from: 'src/domain/**', to: 'global:*'}\nallow:\n  - {from: '**', to: 'pkg:**'}\n",
        &[],
    );
    assert_eq!(
        forbidden(&output),
        [
            "forbidden src/domain/a.ts -> global:process (forbid[0])",
            "forbidden src/domain/a.ts -> global:console (forbid[0])"
        ]
    );

    check(rule, &[]);
    let simulation = archgraph(&[
        "simulate",
        &checked,
        "--move",
        "src/domain/a.ts",
        "src/adapter/a.ts",
    ]);
    assert!(
        simulation.status.success(),
        "{}",
        String::from_utf8_lossy(&simulation.stderr)
    );
    let simulation: Value = serde_json::from_slice(&simulation.stdout).unwrap();
    validate(&simulation);
    assert_eq!(simulation["removed_forbidden"].as_array().unwrap().len(), 1);
    let difference = archgraph(&["diff", &graph, &checked, "--fail-on", "new-forbidden"]);
    assert_eq!(difference.status.code(), Some(1));
    let difference: Value = serde_json::from_slice(&difference.stdout).unwrap();
    validate(&difference);
    assert_eq!(
        difference["new_forbidden"][0]["to"],
        "external:typescript:global:process"
    );
    assert_eq!(difference["new_forbidden"].as_array().unwrap().len(), 1);
    assert_eq!(check(rule, &["--legacy-v1"]).status.code(), Some(1));
    validate(&serde_json::from_slice(&fs::read(&checked).unwrap()).unwrap());
    assert_eq!(
        check(rule, &["--update-known", "accept"]).status.code(),
        Some(0)
    );
    let known = fs::read_to_string(root.join("layout.known.yaml")).unwrap();
    assert!(known.contains(r#"to: "global:process""#), "{known}");
    fs::remove_file(root.join("layout.known.yaml")).unwrap();

    // `report` counts packages only, not the global node.
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src/domain")).unwrap();
    fs::create_dir_all(repo.join("node_modules/typeorm")).unwrap();
    fs::write(
        repo.join("package.json"),
        r#"{"name":"app","dependencies":{"typeorm":"1"}}"#,
    )
    .unwrap();
    fs::write(
        repo.join("node_modules/typeorm/package.json"),
        r#"{"name":"typeorm","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(repo.join("node_modules/typeorm/index.js"), "").unwrap();
    fs::write(
        repo.join("src/domain/a.ts"),
        "import { Entity } from 'typeorm';\nexport const env = process.env;\n",
    )
    .unwrap();
    fs::write(repo.join("layout.yaml"), format!("version: 1\n{rule}")).unwrap();
    let report = archgraph(&[
        "report",
        repo.to_str().unwrap(),
        "--engine",
        "native",
        "--no-provenance",
        "--out-dir",
        &path("report"),
    ]);
    let text = String::from_utf8_lossy(&report.stdout);
    assert!(
        text.contains("  1 known external packages"),
        "{text}{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let reported: Value =
        serde_json::from_slice(&fs::read(root.join("report/archgraph.json")).unwrap()).unwrap();
    assert_eq!(reported["external_nodes"].as_array().unwrap().len(), 2);

    // A scan without collected globals cannot enforce a global rule.
    let mut reference = input;
    reference.as_object_mut().unwrap().remove("globals");
    fs::write(&graph, serde_json::to_vec(&reference).unwrap()).unwrap();
    let output = check(rule, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "global rules require a native JavaScript/TypeScript scan, rescan with --engine native"
    ));
    fs::remove_dir_all(root).unwrap();
}
