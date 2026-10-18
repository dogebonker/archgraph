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
