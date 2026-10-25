#[path = "../src/cli/explain.rs"]
mod explain;

use boundrail::model::Document;
use serde_json::{Value, json};

fn document() -> Document {
    boundrail::analyze(
        serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","version":null,"cruised":2},"git":null,"generated_at":"test"})).unwrap(),
        serde_json::from_value(json!({"depth":2,"source_root":".","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"fixture","doc":{"version":1,"modules":[{"id":"domain","path":"domain/**"},{"id":"adapters","path":"adapters/**"}],"layers":["domain/**","adapters/**"],"forbid":[{"id":"boundary","from":"domain/**","to":"adapters/**","reason":"Keep domain independent"}]}}})).unwrap(),
        vec!["domain/a.ts".into(), "adapters/b.ts".into()],
        vec![serde_json::from_value(json!({"from":"adapters/b.ts","to":"domain/a.ts","kind":"static","verdict":null,"rule":null})).unwrap()],
        vec![],
        None,
    ).unwrap()
}

#[test]
fn whole_edit_has_graph_and_policy_evidence_in_a_bounded_summary() {
    let doc = document();
    let value = explain::for_agent(&doc, &["domain/a.ts".into(), "domain/new.ts".into()]).unwrap();
    let summary = value["summary"].as_str().unwrap();
    for fact in [
        "files:2",
        "unscanned:1",
        "imports:0",
        "importers:1",
        "rules:3",
        "modules:domain",
        "layers:domain/**",
        "forbid:domain/**->adapters/**",
    ] {
        assert!(summary.contains(fact), "missing {fact}: {summary}");
    }
    assert_eq!(value["omitted"], true);
    assert!(serde_json::to_vec(&value).unwrap().len() <= 400);
    // The ordinary explanation retains its existing full payload.
    assert_eq!(
        explain::explain(&doc, "domain/a.ts").unwrap()["subject"],
        "domain/a.ts"
    );
}

#[test]
fn summaries_are_deterministic_with_unknown_long_and_multibyte_paths() {
    let mut doc = document();
    doc.config.rules = None;
    doc.files[0].module = format!("{}\"\\", "界".repeat(10_000));
    let paths = vec![
        "domain/a.ts".into(),
        format!("new/{}.ts", "界".repeat(500)),
        "domain/a.ts".into(),
    ];
    let first = explain::for_agent(&doc, &paths).unwrap();
    let mut reversed = paths;
    reversed.reverse();
    assert_eq!(first, explain::for_agent(&doc, &reversed).unwrap());
    let bytes = serde_json::to_vec(&first).unwrap();
    assert!(bytes.len() <= 400);
    let round_trip: Value = serde_json::from_slice(&bytes).unwrap();
    let summary = round_trip["summary"].as_str().unwrap();
    for fact in [
        "files:2",
        "unscanned:1",
        "rules:unavailable",
        "unknown_modules:1",
        "unclassified:2",
    ] {
        assert!(summary.contains(fact), "missing {fact}: {summary}");
    }
}

#[test]
fn invalid_paths_do_not_become_policy_evidence() {
    let doc = document();
    for path in ["../secret.ts", "/tmp/file.ts", "domain/*.ts"] {
        assert!(explain::for_agent(&doc, &[path.into()]).is_err());
    }
    assert!(explain::for_agent(&doc, &[]).is_err());
}

#[test]
fn cli_flag_emits_compact_json_for_a_new_file() {
    let graph = std::env::temp_dir().join(format!(
        "archgraph-agent-explain-{}.json",
        std::process::id()
    ));
    std::fs::write(&graph, serde_json::to_vec(&document()).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args([
            "explain",
            "domain/new.ts",
            "--for-agent",
            "--brief",
            "--graph",
        ])
        .arg(&graph)
        .output()
        .unwrap();
    std::fs::remove_file(graph).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 401); // Compact payload plus the CLI newline.
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "archgraph/explain-agent/v1");
    assert!(value["summary"].as_str().unwrap().contains("unscanned:1"));
}
