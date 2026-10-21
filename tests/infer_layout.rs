use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

struct Scratch(PathBuf);
static NEXT: AtomicUsize = AtomicUsize::new(0);

impl Scratch {
    fn new() -> Self {
        let stamp = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("infer-layout-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }

    fn graph(&self, schema: &str, paths: &[String], edges: Vec<Value>) -> PathBuf {
        let files: Vec<_> = paths
            .iter()
            .map(|path| {
                let module = path.split_once('/').map_or(path.as_str(), |(dir, _)| dir);
                json!({"path": path, "module": module, "barrel": false})
            })
            .collect();
        let graph = json!({
            "schema": schema, "kind": "scan",
            "source": {"language": "typescript", "collector": {"name": "fixture", "version": null, "cruised": paths.len()}, "git": null, "generated_at": "2026-01-01T00:00:00Z"},
            "config": {"depth": 1, "source_root": ".", "excludes": [], "include_tests": false, "rules": null},
            "files": files, "edges": edges
        });
        let path = self.0.join("graph.json");
        fs::write(&path, serde_json::to_vec(&graph).unwrap()).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn edge(from: &str, to: &str) -> Value {
    json!({"from": from, "to": to, "kind": "runtime"})
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn printed_draft_is_valid_and_checks_both_schemas_without_writing() {
    for schema in ["archgraph/v1", "archgraph/v1.1"] {
        let scratch = Scratch::new();
        let paths = [
            "entry/main.ts",
            "service/api.ts",
            "domain/model.ts",
            "isolated.ts",
        ]
        .map(str::to_owned);
        let graph = scratch.graph(
            schema,
            &paths,
            vec![edge(&paths[0], &paths[1]), edge(&paths[1], &paths[2])],
        );
        let output = scratch.run(&["infer-layout", graph.to_str().unwrap()]);
        success(&output);
        assert_eq!(
            fs::read_dir(&scratch.0).unwrap().count(),
            1,
            "default mode must not write a draft or cache"
        );
        let yaml = String::from_utf8(output.stdout).unwrap();
        assert!(yaml.to_lowercase().contains("draft"));
        let draft = scratch.0.join("draft.yaml");
        fs::write(&draft, yaml).unwrap();
        let rules = archgraph::rules::load(&draft).unwrap();
        assert!(!rules.doc.layers.is_empty());
        let checked_path = scratch.0.join("checked.json");
        let checked = scratch.run(&[
            "check",
            graph.to_str().unwrap(),
            "--rules",
            draft.to_str().unwrap(),
            "--output",
            checked_path.to_str().unwrap(),
        ]);
        success(&checked);
        let doc: Value = serde_json::from_slice(&fs::read(checked_path).unwrap()).unwrap();
        assert!(
            doc["edges"]
                .as_array()
                .unwrap()
                .iter()
                .all(|edge| edge["verdict"] != "forbidden")
        );
        assert_eq!(doc["files"].as_array().unwrap().len(), 4);
        let compiled = archgraph::rules::compile(&serde_json::from_value(json!({
            "depth": 1, "source_root": ".", "excludes": [], "include_tests": false, "rules": rules
        })).unwrap()).unwrap().unwrap();
        for path in paths {
            assert!(compiled.module_of(&path).is_some(), "draft omitted {path}");
        }
        let file_mode = scratch.run(&[
            "infer-layout",
            graph.to_str().unwrap(),
            "--granularity",
            "file",
        ]);
        success(&file_mode);
        let _: archgraph::rules::Rules = serde_yaml_ng::from_slice(&file_mode.stdout).unwrap();
    }
}

#[test]
fn apply_creates_default_or_explicit_path_and_never_overwrites() {
    let scratch = Scratch::new();
    let graph = scratch.graph("archgraph/v1.1", &["app/main.ts".into()], vec![]);
    let graph = graph.to_str().unwrap();
    let output = scratch.run(&["infer-layout", graph, "--apply"]);
    success(&output);
    assert!(output.stdout.is_empty());
    let path = scratch.0.join("layout.yaml");
    let original = fs::read(&path).unwrap();
    archgraph::rules::load(&path).unwrap();
    let output = scratch.run(&["infer-layout", graph, "--apply"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), original);
    let explicit = scratch.0.join("another.yaml");
    let output = scratch.run(&["infer-layout", graph, "--apply", explicit.to_str().unwrap()]);
    success(&output);
    assert!(output.stdout.is_empty());
    archgraph::rules::load(&explicit).unwrap();
    #[cfg(unix)]
    {
        let link = scratch.0.join("linked.yaml");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let output = scratch.run(&["infer-layout", graph, "--apply", link.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(2));
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn invalid_input_and_thresholds_cannot_create_a_layout() {
    let scratch = Scratch::new();
    let graph = scratch.graph("archgraph/v1.1", &["app/main.ts".into()], vec![]);
    let graph = graph.to_str().unwrap();
    for arguments in [
        vec!["--min-potential-edges", "0"],
        vec!["--max-density", "NaN"],
        vec!["--max-density", "1.1"],
        vec!["--min-slices", "1"],
        vec!["--min-share", "0"],
        vec!["--min-share", "1.5"],
    ] {
        let output = scratch.run(&[&["infer-layout", graph, "--apply"][..], &arguments].concat());
        assert_eq!(output.status.code(), Some(2));
        assert!(!scratch.0.join("layout.yaml").exists());
    }
    fs::write(graph, "{\"schema\":\"archgraph/v999\"}").unwrap();
    let output = scratch.run(&["infer-layout", graph, "--apply"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!scratch.0.join("layout.yaml").exists());
}

#[test]
fn sparse_proposal_reports_and_checks_written_import_violations() {
    let scratch = Scratch::new();
    let mut paths: Vec<_> = (0..25).map(|i| format!("adapters-in/{i}.ts")).collect();
    paths.extend((0..19).map(|i| format!("adapters-out/{i}.ts")));
    paths.extend(["adapters-out/index.ts".into(), "domain/value.ts".into()]);
    let mut edges = vec![
        edge("adapters-in/0.ts", "domain/value.ts"),
        edge("adapters-in/1.ts", "domain/value.ts"),
    ];
    for (i, edge) in edges.iter_mut().enumerate() {
        edge["original_to"] = json!("adapters-out/index.ts");
        edge["line"] = json!(11 + i);
    }
    let graph = scratch.graph("archgraph/v1.1", &paths, edges);
    let output = scratch.run(&["infer-layout", graph.to_str().unwrap()]);
    success(&output);
    let yaml = String::from_utf8(output.stdout).unwrap();
    assert!(yaml.contains("500"), "{yaml}");
    for witness in [
        "\"adapters-in/0.ts\":11",
        "\"adapters-in/1.ts\":12",
        "adapters-out/index.ts",
    ] {
        assert!(yaml.contains(witness), "missing {witness}: {yaml}");
    }
    let draft = scratch.0.join("draft.yaml");
    fs::write(&draft, yaml).unwrap();
    let rules = archgraph::rules::load(&draft).unwrap();
    assert!(!rules.doc.forbid.is_empty());
    let checked_path = scratch.0.join("checked.json");
    let checked = scratch.run(&[
        "check",
        graph.to_str().unwrap(),
        "--rules",
        draft.to_str().unwrap(),
        "--output",
        checked_path.to_str().unwrap(),
    ]);
    assert_eq!(
        checked.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let doc: Value = serde_json::from_slice(&fs::read(checked_path).unwrap()).unwrap();
    let forbidden: Vec<_> = doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["verdict"] == "forbidden")
        .collect();
    assert_eq!(forbidden.len(), 2);
    assert!(
        forbidden
            .iter()
            .all(|edge| edge["original_to"] == "adapters-out/index.ts")
    );
}

#[test]
fn drafted_slice_claim_checks_as_holding_in_most_slices() {
    let scratch = Scratch::new();
    let mut paths = Vec::new();
    let mut edges = Vec::new();
    for name in ["a", "b", "c", "d", "e"] {
        for role in ["handler", "service", "repository"] {
            paths.push(format!("internal/{name}/{role}.go"));
        }
        edges.push(edge(
            &format!("internal/{name}/handler.go"),
            &format!("internal/{name}/service.go"),
        ));
    }
    edges.push(edge("internal/e/service.go", "internal/e/handler.go"));
    let graph = scratch.graph("archgraph/v1.1", &paths, edges);
    let output = scratch.run(&["infer-layout", graph.to_str().unwrap()]);
    success(&output);
    let draft = scratch.0.join("draft.yaml");
    fs::write(&draft, output.stdout).unwrap();
    let checked = scratch.run(&[
        "check",
        graph.to_str().unwrap(),
        "--rules",
        draft.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&checked.stdout);
    assert_eq!(checked.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains(
            "claim infer-claim-1: holds in 4 of 5 slices; broken in e reason=\"service does not import handler (drafted from the graph; review intent)\"\n"
        ),
        "{stdout}"
    );
}

#[test]
fn type_back_edge_makes_cycle_unresolved_even_without_all_kinds_flag() {
    let scratch = Scratch::new();
    let mut back = edge("domain/model.ts", "app/main.ts");
    back["kind"] = json!("type");
    let graph = scratch.graph(
        "archgraph/v1.1",
        &["app/main.ts".into(), "domain/model.ts".into()],
        vec![edge("app/main.ts", "domain/model.ts"), back],
    );
    let output = scratch.run(&["infer-layout", graph.to_str().unwrap()]);
    success(&output);
    let yaml = String::from_utf8(output.stdout).unwrap();
    assert!(yaml.contains("# unresolved:"), "{yaml}");
    let rules: archgraph::rules::Rules = serde_yaml_ng::from_str(&yaml).unwrap();
    assert!(rules.layers.is_empty());
    assert!(rules.forbid.is_empty());
    assert_eq!(rules.modules.len(), 2);
    assert!(yaml.contains("\"app\""));
    assert!(yaml.contains("\"domain\""));
}
