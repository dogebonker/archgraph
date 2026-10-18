mod common;

use archgraph::model::{Config, Edge, Source};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "context-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(
            root.join("a.ts"),
            "import { b } from './lib/b';\nexport const a = b;\n",
        )
        .unwrap();
        fs::write(root.join("lib/b.ts"), "export const b = 1;\n").unwrap();
        fs::write(
            root.join("layout.yaml"),
            "version: 1\n# Keep module boundaries intact.\n",
        )
        .unwrap();
        let source: Source = serde_json::from_value(json!({
            "language":"typescript", "collector":{"name":"fixture","cruised":2},
            "generated_at":"2026-10-17T00:00:00Z"
        }))
        .unwrap();
        let config: Config = serde_json::from_value(json!({
            "depth":1, "source_root":".", "excludes":[], "include_tests":false
        }))
        .unwrap();
        let doc = archgraph::analyze(
            source,
            config,
            vec!["a.ts".into(), "lib/b.ts".into()],
            vec![Edge {
                from: "a.ts".into(),
                to: "lib/b.ts".into(),
                ..Default::default()
            }],
            vec![],
            None,
        )
        .unwrap();
        fs::write(root.join("graph.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
        Self(root)
    }
    fn run(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .current_dir(&self.0)
            .args([
                "context",
                "--graph",
                "graph.json",
                "--task",
                "Edit the named source",
            ])
            .args(extra)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn parses_budget_suffixes_and_rejects_invalid_zero_and_overflow() {
    let fixture = Fixture::new();
    for (budget, expected) in [("40k", 40_000), ("1M", 1_000_000), ("40000", 40_000)] {
        let value: Value = serde_json::from_str(&success(
            fixture.run(&["--files", "a.ts", "--budget", budget]),
        ))
        .unwrap();
        assert_eq!(value["budget"], expected);
    }
    for budget in ["0", "0k", "40kb", "1.5k", "-1", "18446744073709551615m"] {
        let output = fixture.run(&["--budget", budget]);
        assert!(!output.status.success(), "accepted {budget}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn json_and_markdown_put_constraints_first_within_budget() {
    let fixture = Fixture::new();
    let value: Value = serde_json::from_str(&success(
        fixture.run(&["--files", "a.ts", "--budget", "2k"]),
    ))
    .unwrap();
    common::validate(&value);
    assert_eq!(value["kind"], "context");
    // The fixture graph records no Git provenance, so --root cannot be shown to match it.
    assert!(
        value["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().starts_with("Source root is unverified"))
    );
    assert_eq!(value["entries"][0]["path"], "layout.yaml");
    assert_eq!(value["entries"][0]["mode"], "rules");
    let entries = value["entries"].as_array().unwrap();
    let used = value["used_tokens"].as_u64().unwrap();
    assert!(used <= 2000);
    assert_eq!(
        used,
        value["overhead_tokens"].as_u64().unwrap()
            + entries
                .iter()
                .map(|e| e["tokens"].as_u64().unwrap())
                .sum::<u64>()
    );
    assert!(
        entries
            .iter()
            .any(|e| e["path"] == "a.ts" && e["tier"] == "must-read")
    );
    let text = success(fixture.run(&["--files", "a.ts", "--format", "markdown", "--budget", "2k"]));
    assert!(text.find("layout.yaml").unwrap() < text.find("`a.ts`").unwrap());
    assert!(text.contains("Keep module boundaries intact."));
    assert!(text.contains("Architecture context"));
}

#[test]
fn absolute_paths_are_contained_and_tiny_budgets_fail_without_partial_output() {
    let fixture = Fixture::new();
    let inside = fixture.0.join("a.ts").canonicalize().unwrap();
    let value: Value = serde_json::from_str(&success(
        fixture.run(&["--files", inside.to_str().unwrap()]),
    ))
    .unwrap();
    assert!(
        value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "a.ts")
    );
    let outside = Fixture::new();
    let output = fixture.run(&["--files", outside.0.join("a.ts").to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside context root"));
    let output = fixture.run(&["--files", "a.ts", "--budget", "1"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("require at least"));
}

#[test]
fn reads_legacy_graph_but_rejects_legacy_context_json_output() {
    let fixture = Fixture::new();
    let mut graph: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("graph.json")).unwrap()).unwrap();
    archgraph::compat::v1(&mut graph).unwrap();
    assert_eq!(graph["schema"], "archgraph/v1");
    fs::write(
        fixture.0.join("graph.json"),
        serde_json::to_vec(&graph).unwrap(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&success(fixture.run(&["--files", "a.ts"]))).unwrap();
    assert_eq!(value["schema"], "archgraph/v1.1");
    let output = fixture.run(&["--files", "a.ts", "--legacy-v1"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("context JSON requires archgraph/v1.1")
    );
}
