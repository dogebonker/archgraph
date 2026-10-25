mod common;

use boundrail::model::{Config, Edge, Source};
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
        let doc = boundrail::analyze(
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
        self.run_task("Edit the named source", extra)
    }
    fn run_task(&self, task: &str, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .current_dir(&self.0)
            .args(["context", "--graph", "graph.json", "--task", task])
            .args(extra)
            .output()
            .unwrap()
    }
    fn graph(&self, paths: &[&str], edges: &[(&str, &str)]) {
        let original: Value =
            serde_json::from_slice(&fs::read(self.0.join("graph.json")).unwrap()).unwrap();
        let doc = boundrail::analyze(
            serde_json::from_value(original["source"].clone()).unwrap(),
            serde_json::from_value(original["config"].clone()).unwrap(),
            paths.iter().map(|path| (*path).into()).collect(),
            edges
                .iter()
                .map(|(from, to)| Edge {
                    from: (*from).into(),
                    to: (*to).into(),
                    ..Default::default()
                })
                .collect(),
            vec![],
            None,
        )
        .unwrap();
        fs::write(self.0.join("graph.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
    }
    fn write(&self, path: &str, content: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fn exports(&self, paths: &[&str]) {
        let mut graph: Value =
            serde_json::from_slice(&fs::read(self.0.join("graph.json")).unwrap()).unwrap();
        graph["symbols"] = json!({
            "complete": true,
            "exports": paths.iter().map(|path| json!({
                "file": path, "name": "start", "target": {"file": path, "name": "start"}
            })).collect::<Vec<_>>(),
            "imports": [], "limitations": []
        });
        fs::write(
            self.0.join("graph.json"),
            serde_json::to_vec(&graph).unwrap(),
        )
        .unwrap();
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
    boundrail::compat::v1(&mut graph).unwrap();
    assert_eq!(graph["schema"], "archgraph/v1");
    fs::write(
        fixture.0.join("graph.json"),
        serde_json::to_vec(&graph).unwrap(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&success(fixture.run(&["--files", "a.ts"]))).unwrap();
    assert_eq!(value["schema"], "archgraph/v1.2");
    let output = fixture.run(&["--files", "a.ts", "--legacy-v1"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("context JSON requires the current schema")
    );
}

#[test]
fn source_matches_join_graph_context_and_corroboration_gets_body_priority() {
    let fixture = Fixture::new();
    let paths = [
        "seed/start.ts",
        "connected/z.ts",
        "isolated/a.ts",
        "connected/b.ts",
    ];
    for path in paths {
        fixture.write(path, "export const value = 1;\n");
    }
    fixture.write("connected/z.ts", "export const value = 'NEEDLE';\n");
    fixture.write("isolated/a.ts", "export const value = 'needle';\n");
    fixture.graph(
        &paths,
        &[
            ("seed/start.ts", "connected/z.ts"),
            ("seed/start.ts", "connected/b.ts"),
        ],
    );
    let report: Value = serde_json::from_str(&success(
        fixture.run_task("needle", &["--files", "seed/start.ts", "--budget", "2k"]),
    ))
    .unwrap();
    let entries = report["entries"].as_array().unwrap();
    let sources: Vec<_> = entries.iter().filter(|e| e["mode"] != "rules").collect();
    assert_eq!(sources[0]["path"], "seed/start.ts");
    assert_eq!(sources[1]["path"], "connected/z.ts");
    assert!(
        sources
            .iter()
            .any(|e| e["path"] == "isolated/a.ts" && e["mode"] == "body")
    );
    assert!(sources.iter().any(|e| e["path"] == "connected/b.ts"));
    assert!(report["used_tokens"].as_u64().unwrap() <= 2_000);
}

#[test]
fn folder_fallback_is_immediate_and_search_respects_saved_graph_eligibility() {
    let fixture = Fixture::new();
    let paths = [
        "feature/start.ts",
        "feature/helper.ts",
        "feature/nested/deep.ts",
        "feature-other/near.ts",
        "tests/check.ts",
        "generated/table.ts",
        "vendor/pkg.ts",
        "barrel/index.ts",
    ];
    for path in paths {
        fixture.write(path, "export const value = 1;\n");
    }
    for path in ["tests/check.ts", "generated/table.ts", "vendor/pkg.ts"] {
        fixture.write(path, "export const value = 'needle';\n");
    }
    fixture.write(
        "barrel/index.ts",
        "export { needle } from './definition';\n",
    );
    fixture.write("unlisted.ts", "export const value = 'needle';\n");
    fixture.graph(&paths, &[]);
    let mut graph: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("graph.json")).unwrap()).unwrap();
    graph["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|file| file["path"] == "barrel/index.ts")
        .unwrap()["barrel"] = json!(true);
    fs::write(
        fixture.0.join("graph.json"),
        serde_json::to_vec(&graph).unwrap(),
    )
    .unwrap();
    let report: Value = serde_json::from_str(&success(
        fixture.run_task("needle", &["--files", "feature/start.ts", "--budget", "2k"]),
    ))
    .unwrap();
    let sources: Vec<_> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["mode"] != "rules")
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["feature/start.ts", "feature/helper.ts"]);
}

#[cfg(unix)]
#[test]
fn source_search_reports_outside_symlinks_without_admitting_them() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    outside.write("secret.ts", "export const secret = 'needle';\n");
    std::os::unix::fs::symlink(outside.0.join("secret.ts"), fixture.0.join("linked.ts")).unwrap();
    fixture.graph(&["a.ts", "linked.ts"], &[]);
    let report: Value = serde_json::from_str(&success(fixture.run_task("needle", &[]))).unwrap();
    assert!(
        !report["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "linked.ts")
    );
    assert!(report["omitted"].as_array().unwrap().iter().any(|entry| {
        entry["path"] == "linked.ts" && entry["reason"].as_str().unwrap().contains("escapes root")
    }));
}

#[test]
fn source_search_reports_invalid_utf8_without_panicking() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("binary.ts"), [0xff, 0xfe]).unwrap();
    fixture.graph(&["a.ts", "binary.ts"], &[("a.ts", "binary.ts")]);
    let report: Value =
        serde_json::from_str(&success(fixture.run_task("needle", &["--files", "a.ts"]))).unwrap();
    assert!(
        !report["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "binary.ts")
    );
    assert!(report["omitted"].as_array().unwrap().iter().any(|entry| {
        entry["path"] == "binary.ts" && entry["reason"].as_str().unwrap().contains("UTF-8")
    }));
}

#[test]
fn source_search_matches_words_and_identifier_tokens_without_substrings() {
    let fixture = Fixture::new();
    let paths = [
        "words/a.ts",
        "camel/b.ts",
        "underscore/c.ts",
        "substring/d.ts",
    ];
    fixture.write(paths[0], "export const action = 'PAY';\n");
    fixture.write(paths[1], "export const payInvoice = 1;\n");
    fixture.write(paths[2], "export const invoice_pay = 1;\n");
    fixture.write(paths[3], "export const payload = 'repayment';\n");
    fixture.graph(&paths, &[]);
    let report: Value = serde_json::from_str(&success(fixture.run_task("pay", &[]))).unwrap();
    let mut selected: Vec<_> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["mode"] != "rules")
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    selected.sort_unstable();
    assert_eq!(selected, ["camel/b.ts", "underscore/c.ts", "words/a.ts"]);
}

#[test]
fn equally_matching_graph_neighbors_prefer_compact_source_bodies() {
    let fixture = Fixture::new();
    let paths = ["seed/start.ts", "neighbors/a.ts", "neighbors/z.ts"];
    fixture.write(paths[0], "export const start = 1;\n");
    let matching = "export const action = 'needle reconcile';\n";
    fixture.write(
        paths[1],
        &format!("{matching}/* {} */\n", "padding ".repeat(1500)),
    );
    fixture.write(paths[2], matching);
    fixture.graph(&paths, &[(paths[0], paths[1]), (paths[0], paths[2])]);
    let report: Value = serde_json::from_str(&success(
        fixture.run_task("needle reconcile", &["--files", paths[0], "--budget", "8k"]),
    ))
    .unwrap();
    let sources: Vec<_> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["mode"] != "rules")
        .collect();
    assert_eq!(sources.len(), 3);
    assert_eq!(sources[0]["path"], paths[0]);
    assert_eq!(sources[1]["path"], paths[2]);
    assert_eq!(sources[1]["mode"], "body");
    assert_eq!(sources[2]["path"], paths[1]);
}

#[test]
fn large_explicit_seeds_leave_budget_for_relevant_neighbor_bodies() {
    let fixture = Fixture::new();
    let paths = ["seed/a.ts", "seed/b.ts", "neighbor/c.ts"];
    for path in &paths[..2] {
        fixture.write(
            path,
            &format!(
                "export const start = 1;\n/* {} */\n",
                "padding ".repeat(750)
            ),
        );
    }
    fixture.write(
        paths[2],
        &format!(
            "export const action = 'needle';\n/* {} */\n",
            "padding ".repeat(190)
        ),
    );
    fixture.graph(&paths, &[(paths[0], paths[2]), (paths[1], paths[2])]);
    fixture.exports(&paths);
    let report: Value = serde_json::from_str(&success(fixture.run_task(
        "needle",
        &["--files", paths[0], "--files", paths[1], "--budget", "2k"],
    )))
    .unwrap();
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries[0]["path"], "layout.yaml");
    let sources: Vec<_> = entries
        .iter()
        .filter(|entry| entry["mode"] != "rules")
        .collect();
    assert_eq!(sources.len(), 3);
    for (entry, path) in sources[..2].iter().zip(paths) {
        assert_eq!(entry["path"], path);
        assert_eq!(entry["mode"], "exports");
    }
    assert_eq!(sources[2]["path"], "neighbor/c.ts");
    assert_eq!(sources[2]["mode"], "body");
    let used = report["used_tokens"].as_u64().unwrap();
    assert!(used <= 2000);
    assert_eq!(
        used,
        report["overhead_tokens"].as_u64().unwrap()
            + entries
                .iter()
                .map(|entry| entry["tokens"].as_u64().unwrap())
                .sum::<u64>()
    );
}

#[test]
fn seed_body_is_preserved_when_it_fits_with_related_body() {
    let fixture = Fixture::new();
    let paths = ["seed/a.ts", "neighbor/b.ts"];
    fixture.write(
        paths[0],
        &format!(
            "export const start = 1;\n/* {} */\n",
            "padding ".repeat(750)
        ),
    );
    fixture.write(paths[1], "export const action = 'needle';\n");
    fixture.graph(&paths, &[(paths[0], paths[1])]);
    fixture.exports(&[paths[0]]);
    let report: Value = serde_json::from_str(&success(
        fixture.run_task("needle", &["--files", paths[0], "--budget", "2k"]),
    ))
    .unwrap();
    let sources: Vec<_> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["mode"] != "rules")
        .collect();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["path"], paths[0]);
    assert_eq!(sources[0]["mode"], "body");
    assert_eq!(sources[1]["path"], paths[1]);
    assert_eq!(sources[1]["mode"], "body");
    assert!(report["used_tokens"].as_u64().unwrap() <= 2000);
}

#[test]
fn seed_body_is_preserved_when_related_body_cannot_fit_anyway() {
    let fixture = Fixture::new();
    let paths = ["seed/a.ts", "neighbor/b.ts"];
    fixture.write(
        paths[0],
        &format!(
            "export const start = 1;\n/* {} */\n",
            "padding ".repeat(750)
        ),
    );
    fixture.write(
        paths[1],
        &format!(
            "export const action = 'needle';\n/* {} */\n",
            "padding ".repeat(3000)
        ),
    );
    fixture.graph(&paths, &[(paths[0], paths[1])]);
    fixture.exports(&[paths[0]]);
    let report: Value = serde_json::from_str(&success(
        fixture.run_task("needle", &["--files", paths[0], "--budget", "2k"]),
    ))
    .unwrap();
    let sources: Vec<_> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["mode"] != "rules")
        .collect();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["path"], paths[0]);
    assert_eq!(sources[0]["mode"], "body");
    assert!(
        report["omitted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == paths[1])
    );
}
