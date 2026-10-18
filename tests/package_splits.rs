mod common;

use archgraph::{
    model::{Config, Document, Edge, ImportRecord, Source},
    package_splits::{self, Options, WorkspaceInventory},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "package-split-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    fn git(&self, root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn checkout(&self) -> (PathBuf, String) {
        let root = self.0.join("repo");
        fs::create_dir_all(root.join("lib/core")).unwrap();
        fs::create_dir_all(root.join("app")).unwrap();
        fs::create_dir_all(root.join("sibling")).unwrap();
        for (path, source) in [
            ("lib/core/a.ts", "import './b';"),
            ("lib/core/b.ts", "export const b=1;"),
            ("app/a.ts", "import '../lib/core/a';"),
            ("sibling/.keep", ""),
            ("package.json", "{\"name\":\"@repo/root\",\"private\":true}"),
        ] {
            fs::write(root.join(path), source).unwrap();
        }
        self.git(&root, &["init", "-q"]);
        self.git(&root, &["add", "."]);
        self.git(&root, &["commit", "-qm", "fixture"]);
        let commit = self.git(&root, &["rev-parse", "HEAD"]);
        (root, commit)
    }
    fn graph(&self, commit: Option<&str>, anchor: bool) -> PathBuf {
        let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"dependency-cruiser","cruised":3},"git":commit.map(|commit| json!({"commit":commit,"dirty":false})),"scan_root":if anchor {json!(".")} else {Value::Null},"generated_at":"2026-10-17T00:00:00Z"})).unwrap();
        let config: Config = serde_json::from_value(
            json!({"depth":1,"source_root":".","excludes":[],"include_tests":false}),
        )
        .unwrap();
        let edges: Vec<_> = [
            ("app/a.ts", "lib/core/a.ts"),
            ("lib/core/a.ts", "lib/core/b.ts"),
        ]
        .into_iter()
        .map(|(from, to)| Edge {
            from: from.into(),
            to: to.into(),
            line: Some(1),
            ..Default::default()
        })
        .collect();
        let mut doc = archgraph::analyze(
            source,
            config,
            vec![
                "app/a.ts".into(),
                "lib/core/a.ts".into(),
                "lib/core/b.ts".into(),
            ],
            edges,
            vec![],
            None,
        )
        .unwrap();
        doc.imports = Some(
            doc.edges
                .iter()
                .map(|edge| ImportRecord {
                    from: edge.from.clone(),
                    specifier: edge.to.clone(),
                    targets: vec![edge.to.clone()],
                    line: edge.line,
                    kind: edge.kind,
                    ..Default::default()
                })
                .collect(),
        );
        let path = self.0.join("graph.json");
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn parsed(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = serde_json::from_slice(&output.stdout).unwrap();
    common::validate(&value);
    value
}
fn core(report: &Value) -> &Value {
    report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == "lib/core")
        .unwrap()
}

#[test]
fn report_enrichment_restores_recomputes_and_renders_without_collectors() {
    let s = Scratch::new();
    let (root, commit) = s.checkout();
    let graph = s.graph(Some(&commit), true);
    let enriched = s.0.join("enriched.json");
    let report = parsed(s.run(&[
        "package-splits",
        graph.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--format",
        "json",
        "--output",
        enriched.to_str().unwrap(),
    ]));
    assert_eq!(report["workspace"]["checked"], true);
    assert_eq!(core(&report)["readiness"], "import-level-ready");
    assert_eq!(core(&report)["score"], 2);
    assert!(core(&report)["simulation"].get("no_new_cycles").is_none());
    let mut saved: Value = serde_json::from_slice(&fs::read(&enriched).unwrap()).unwrap();
    common::validate(&saved);
    saved["package_splits"]["candidates"][0]["score"] = json!(9999);
    fs::write(&enriched, serde_json::to_vec(&saved).unwrap()).unwrap();
    let restored = s.0.join("restored.json");
    let rules = s.0.join("layout.yaml");
    fs::write(&rules, "version: 1\n").unwrap();
    let checked = s.run(&[
        "check",
        enriched.to_str().unwrap(),
        "--rules",
        rules.to_str().unwrap(),
        "--output",
        restored.to_str().unwrap(),
    ]);
    assert!(checked.status.success());
    let restored: Value = serde_json::from_slice(&fs::read(restored).unwrap()).unwrap();
    common::validate(&restored);
    assert_eq!(core(&restored["package_splits"])["score"], 2);
    let output = s.run(&["render", enriched.to_str().unwrap()]);
    assert!(output.status.success());
    let html = String::from_utf8(output.stdout).unwrap();
    assert!(html.contains("package_splits"));
    assert!(html.contains("Package split candidates"));
    let persisted = parsed(s.run(&[
        "package-splits",
        enriched.to_str().unwrap(),
        "--format",
        "json",
    ]));
    assert_eq!(core(&persisted)["readiness"], "import-level-ready");
}

#[test]
fn legacy_scans_work_but_unbound_or_dirty_workspace_never_proves_readiness() {
    let s = Scratch::new();
    let (root, commit) = s.checkout();
    let graph = s.graph(Some(&commit), false);
    let mut old: Value = serde_json::from_slice(&fs::read(&graph).unwrap()).unwrap();
    archgraph::compat::v1(&mut old).unwrap();
    fs::write(&graph, serde_json::to_vec(&old).unwrap()).unwrap();
    let report = parsed(s.run(&[
        "package-splits",
        graph.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--format",
        "json",
    ]));
    assert_eq!(report["workspace"]["checked"], false);
    assert_eq!(core(&report)["readiness"], "needs-verification");
    let graph = s.graph(Some(&commit), true);
    fs::write(
        root.join("package.json"),
        "{\"name\":\"@repo/changed\",\"workspaces\":[\"lib/*\"]}",
    )
    .unwrap();
    let report = parsed(s.run(&[
        "package-splits",
        graph.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--format",
        "json",
    ]));
    assert_eq!(report["workspace"]["checked"], false);
    assert!(
        report["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["readiness"] != "import-level-ready")
    );
}

#[test]
fn sibling_checkout_root_same_commit_is_rejected_before_writing() {
    let s = Scratch::new();
    let (root, commit) = s.checkout();
    let graph = s.graph(Some(&commit), true);
    let out = s.0.join("not-written.json");
    let output = s.run(&[
        "package-splits",
        graph.to_str().unwrap(),
        "--root",
        root.join("sibling").to_str().unwrap(),
        "--output",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!out.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("recorded by the scan"));
}

#[test]
fn invalid_options_and_legacy_standalone_do_not_write() {
    let s = Scratch::new();
    let graph = s.graph(None, false);
    let out = s.0.join("not-written.json");
    for args in [
        vec!["--min-cohesion", "NaN"],
        vec!["--min-cohesion", "1.1"],
        vec!["--limit", "0"],
        vec!["--legacy-v1", "--format", "json"],
    ] {
        let output = s.run(
            &[
                vec![
                    "package-splits",
                    graph.to_str().unwrap(),
                    "--output",
                    out.to_str().unwrap(),
                ],
                args,
            ]
            .concat(),
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(!out.exists());
    }
}

#[test]
fn simulated_file_move_recalculates_saved_package_candidates() {
    let s = Scratch::new();
    let graph = s.graph(None, false);
    let input = serde_json::from_slice(&fs::read(graph).unwrap()).unwrap();
    let mut doc: Document = archgraph::restore_scan(input).unwrap();
    doc.package_splits = Some(
        package_splits::analyze(&doc, Options::default(), WorkspaceInventory::default()).unwrap(),
    );
    let moved = archgraph::simulate::apply(
        &doc,
        None,
        &[archgraph::model::Move {
            file: "lib/core/b.ts".into(),
            to: "outside/b.ts".into(),
        }],
    )
    .unwrap();
    assert!(
        moved
            .package_splits
            .unwrap()
            .candidates
            .iter()
            .all(|candidate| candidate.path != "lib/core")
    );
}

#[test]
fn scans_record_the_git_relative_root_used_for_workspace_binding() {
    let s = Scratch::new();
    let (root, _) = s.checkout();
    let raw = s.0.join("collector.json");
    fs::write(&raw,json!({"modules":[
        {"source":"app/a.ts","dependencies":[{"resolved":"lib/core/a.ts","module":"../lib/core/a","dependencyTypes":["local","import"]}]},
        {"source":"lib/core/a.ts","dependencies":[{"resolved":"lib/core/b.ts","module":"./b","dependencyTypes":["local","import"]}]},
        {"source":"lib/core/b.ts","dependencies":[]}],"summary":{"totalCruised":3,"environment":{"version":"18.4.0"}}}).to_string()).unwrap();
    let graph = parsed(s.run(&[
        "scan",
        root.to_str().unwrap(),
        "--language",
        "typescript",
        "--collector-input",
        raw.to_str().unwrap(),
    ]));
    assert_eq!(graph["source"]["scan_root"], ".");
    assert_eq!(graph["source"]["git"]["dirty"], false);
}

#[test]
fn parent_package_files_recorded_as_external_never_become_free_proof() {
    let s = Scratch::new();
    let (root, _) = s.checkout();
    fs::write(root.join("lib/entry.ts"), "import './core/a';").unwrap();
    fs::write(root.join("shared.ts"), "export const shared=1;").unwrap();
    fs::write(
        root.join("lib/core/a.ts"),
        "import './b'; import '../../shared';",
    )
    .unwrap();
    s.git(&root, &["add", "."]);
    s.git(&root, &["commit", "-qm", "parent dependency"]);
    let commit = s.git(&root, &["rev-parse", "HEAD"]);
    let graph = s.graph(Some(&commit), true);
    let mut value: Value = serde_json::from_slice(&fs::read(&graph).unwrap()).unwrap();
    let rename = |path: &str| match path {
        "app/a.ts" => "entry.ts",
        "lib/core/a.ts" => "core/a.ts",
        "lib/core/b.ts" => "core/b.ts",
        _ => panic!("unexpected path"),
    };
    for file in value["files"].as_array_mut().unwrap() {
        let path = rename(file["path"].as_str().unwrap());
        file["path"] = json!(path);
        file["module"] = json!(if path == "entry.ts" {
            "entry.ts"
        } else {
            "core"
        });
    }
    for edge in value["edges"].as_array_mut().unwrap() {
        edge["from"] = json!(rename(edge["from"].as_str().unwrap()));
        edge["to"] = json!(rename(edge["to"].as_str().unwrap()));
    }
    for import in value["imports"].as_array_mut().unwrap() {
        import["from"] = json!(rename(import["from"].as_str().unwrap()));
        for target in import["targets"].as_array_mut().unwrap() {
            *target = json!(rename(target.as_str().unwrap()));
        }
    }
    value["imports"].as_array_mut().unwrap().push(json!({"from":"core/a.ts","specifier":"../../shared","line":2,"kind":"runtime","targets":[],"external":"@repo/root"}));
    value["source"]["scan_root"] = json!("lib");
    fs::write(&graph, serde_json::to_vec(&value).unwrap()).unwrap();
    let report = parsed(s.run(&[
        "package-splits",
        graph.to_str().unwrap(),
        "--root",
        root.join("lib").to_str().unwrap(),
        "--format",
        "json",
    ]));
    assert_eq!(report["workspace"]["checked"], true);
    assert_eq!(report["workspace"]["enclosing_package"], "@repo/root");
    let candidate = report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == "core")
        .unwrap();
    assert_eq!(candidate["readiness"], "needs-verification");
    assert!(!candidate["uncertainties"].as_array().unwrap().is_empty());
}
