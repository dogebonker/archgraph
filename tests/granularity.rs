mod common;
use common::{archgraph, scan, synthetic, write_tmp};
use serde_json::json;
#[test]
fn file_granularity_overrides_grouping_preserves_policy_and_round_trips() {
    let pairs = [
        ("src/a/x.ts", "src/b/y.ts"),
        ("src/b/y.ts", "src/a/x.ts"),
        ("src/a/x.ts", "src/a/private.ts"),
        ("src/a/check.spec.ts", "src/a/x.ts"),
    ]
    .map(|(a, b)| (a.into(), b.into()));
    let (root, input) = synthetic("file-granularity", 4, &pairs);
    let rules = write_tmp(
        "file-granularity-layout.yaml",
        "version: 1\nmodules:\n  - {id: monolith, path: '**'}\ndefault: forbidden\n",
    );
    let args = [
        root.as_str(),
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
        "--include-tests",
        "--granularity",
        "file",
    ];
    let (doc, stdout, _) = scan(&args);
    assert_eq!(doc["config"]["granularity"], "file");
    assert!(
        doc["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["module"] == f["path"] || f["class"] == "tests")
    );
    assert!(
        doc["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["verdict"] == "allowed"),
        "folder policy must survive file grouping"
    );
    assert!(doc["cycles"].as_array().unwrap().iter().any(|c| {
        c["members"]
            .as_array()
            .unwrap()
            .contains(&json!("src/a/x.ts"))
    }));
    let graph = write_tmp("file-granularity.json", &stdout);
    for command in [
        vec!["check", graph.as_str(), "--rules", rules.as_str()],
        vec!["render", graph.as_str()],
        vec!["render", graph.as_str(), "--granularity", "folder"],
        vec![
            "simulate",
            graph.as_str(),
            "--move",
            "src/a/private.ts",
            "src/b/private.ts",
        ],
        vec!["diff", graph.as_str(), graph.as_str()],
    ] {
        let output = archgraph(&command);
        assert!(
            output.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let (legacy, _, _) = scan(&[&args[..], &["--legacy-v1"]].concat());
    assert_eq!(legacy["schema"], "archgraph/v1");
    assert!(legacy["config"].get("granularity").is_none());
    assert!(
        legacy["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["module"] == "monolith")
    );
    archgraph::restore_scan(serde_json::from_value(legacy).unwrap()).unwrap();
}
