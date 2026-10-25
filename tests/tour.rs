use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "tour-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn save(&self, doc: &Value) -> PathBuf {
        let path = self.0.join("graph.json");
        fs::write(&path, serde_json::to_vec(doc).unwrap()).unwrap();
        path
    }

    fn tour(&self, doc: &Value, args: &[&str]) -> String {
        let path = self.save(doc);
        let output = Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .current_dir(&self.0)
            .arg("tour")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn graph(paths: &[&str], edges: &[(&str, &str, &str)]) -> Value {
    json!({
        "schema": "archgraph/v1.1", "kind": "scan",
        "source": {"language":"typescript", "collector":{"name":"fixture","cruised":paths.len()}, "git":null, "generated_at":"2026-01-01T00:00:00Z"},
        "config": {"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":null},
        "files": paths.iter().map(|path| json!({"path":path,"module":path.split('/').next().unwrap(),"barrel":false})).collect::<Vec<_>>(),
        "edges": edges.iter().map(|(from,to,kind)|json!({"from":from,"to":to,"kind":kind})).collect::<Vec<_>>()
    })
}

fn stops(markdown: &str) -> Vec<&str> {
    markdown
        .lines()
        .filter(|line| {
            line.trim_start_matches('#')
                .trim_start()
                .split_once(". ")
                .is_some_and(|(number, _)| number.parse::<usize>().is_ok())
        })
        .collect()
}

fn assert_order(output: &str, modules: &[&str]) {
    let actual = stops(output);
    assert_eq!(actual.len(), modules.len(), "{output}");
    for (line, module) in actual.iter().zip(modules) {
        assert!(
            line.contains(module),
            "expected {module} in {line}:\n{output}"
        );
    }
}

#[test]
fn dependencies_precede_importers_and_reverse_keeps_layer_priorities() {
    let s = Scratch::new();
    let doc = graph(
        &[
            "entry/main.ts",
            "app/service.ts",
            "domain/a.ts",
            "zleaf/b.ts",
        ],
        &[
            ("entry/main.ts", "app/service.ts", "runtime"),
            ("app/service.ts", "domain/a.ts", "runtime"),
            ("entry/main.ts", "domain/a.ts", "runtime"),
            ("app/service.ts", "zleaf/b.ts", "runtime"),
        ],
    );
    let output = s.tour(&doc, &[]);
    assert_order(&output, &["domain", "zleaf", "app", "entry"]);
    assert!(output.contains("imported by 2 modules"), "{output}");
    assert!(output.contains("imports nothing"), "{output}");
    let reverse = s.tour(&doc, &["--reverse"]);
    assert_order(&reverse, &["entry", "app", "domain", "zleaf"]);
}

#[test]
fn same_layer_priority_counts_distinct_modules_instead_of_file_edges() {
    let s = Scratch::new();
    let doc = graph(
        &[
            "alpha/a.ts",
            "zeta/a.ts",
            "one/a.ts",
            "one/b.ts",
            "two/a.ts",
        ],
        &[
            ("one/a.ts", "alpha/a.ts", "runtime"),
            ("one/b.ts", "alpha/a.ts", "runtime"),
            ("one/a.ts", "alpha/a.ts", "runtime"),
            ("one/a.ts", "zeta/a.ts", "runtime"),
            ("two/a.ts", "zeta/a.ts", "runtime"),
        ],
    );
    let output = s.tour(&doc, &[]);
    assert_order(&output, &["zeta", "alpha", "one", "two"]);
}

#[test]
fn cycles_are_single_stops_starting_at_the_least_outbound_module() {
    let s = Scratch::new();
    let doc = graph(
        &["a/a.ts", "b/b.ts", "leaf/a.ts", "entry/main.ts"],
        &[
            ("a/a.ts", "b/b.ts", "runtime"),
            ("b/b.ts", "a/a.ts", "runtime"),
            ("a/a.ts", "leaf/a.ts", "runtime"),
            ("entry/main.ts", "a/a.ts", "runtime"),
            ("entry/main.ts", "b/b.ts", "runtime"),
        ],
    );
    let output = s.tour(&doc, &[]);
    let lines = stops(&output);
    assert_eq!(lines.len(), 3, "{output}");
    assert!(lines[0].contains("leaf"), "{output}");
    assert!(lines[1].contains('a') && lines[1].contains('b'), "{output}");
    assert!(lines[2].contains("entry"), "{output}");
    assert!(output.contains("read together"), "{output}");
    assert!(
        output.contains("imported by 1 module, imports 1 module outside this stop"),
        "{output}"
    );
    assert!(
        output.contains("start at ` b `")
            || output.contains("start at `b`")
            || output.contains("start at b"),
        "{output}"
    );
}

#[test]
fn production_runtime_graph_ignores_tests_externals_and_written_barrel_targets() {
    let s = Scratch::new();
    let mut doc = graph(
        &["a/a.ts", "b/b.ts", "facade/index.ts", "tests/a.test.ts"],
        &[
            ("a/a.ts", "b/b.ts", "runtime"),
            ("b/b.ts", "a/a.ts", "type"),
            ("tests/a.test.ts", "a/a.ts", "runtime"),
            ("a/a.ts", "tests/a.test.ts", "runtime"),
        ],
    );
    doc["files"][3]["class"] = json!("tests");
    doc["files"][3]["module"] = json!("tests::tests");
    doc["edges"][0]["original_to"] = json!("facade/index.ts");
    doc["external_nodes"] = json!([{"id":"external:react","external":true,"package":"react"}]);
    doc["external_edges"] = json!([{"from":"a/a.ts","to":"external:react","kind":"runtime"}]);
    let output = s.tour(&doc, &[]);
    assert_order(&output, &["b", "facade", "a"]);
    assert!(
        !stops(&output)
            .iter()
            .any(|line| line.contains("tests") || line.contains("react")),
        "{output}"
    );
    assert!(!output.contains("read together"), "{output}");
    let all = s.tour(&doc, &["--all-kinds"]);
    assert_eq!(stops(&all).len(), 2, "{all}");
    assert!(all.contains("read together"), "{all}");
    doc["config"]["all_kinds"] = json!(true);
    assert_eq!(s.tour(&doc, &[]), all);
}

#[test]
fn agent_uses_saved_export_names_without_opening_sources_and_marks_missing_evidence() {
    let s = Scratch::new();
    let mut doc = graph(&["domain/model.ts"], &[]);
    let ordinary = s.tour(&doc, &[]);
    let missing = s.tour(&doc, &["--agent"]);
    assert!(
        missing.to_lowercase().contains("unavailable")
            || missing.to_lowercase().contains("not collected"),
        "{missing}"
    );
    doc["symbols"] = json!({
        "complete":true,"imports":[],"limitations":[],
        "exports":[{"file":"domain/model.ts","name":"Account","target":{"file":"domain/model.ts","name":"Account"}}]
    });
    let agent = s.tour(&doc, &["--agent"]);
    assert!(agent.contains("Account"), "{agent}");
    assert!(agent.contains("File export index fallback"), "{agent}");
    assert!(!ordinary.contains("Account"));
    assert!(!s.0.join("domain/model.ts").exists());
    doc["symbols"]["complete"] = json!(false);
    let partial = s.tour(&doc, &["--agent"]);
    assert!(partial.contains("Account"), "{partial}");
    assert!(
        partial.to_lowercase().contains("incomplete") || partial.to_lowercase().contains("partial"),
        "{partial}"
    );
    doc["symbols"]["exports"][0]["name"] = json!("<script>alert(1)</script>`");
    let escaped = s.tour(&doc, &["--agent"]);
    assert!(
        escaped.contains("`` <script>alert(1)</script>` ``"),
        "{escaped}"
    );
    doc.as_object_mut().unwrap().remove("symbols");
    doc["surface"] = json!({
        "schema":"archgraph/v1.1","kind":"surface","max_symbols":50,"complete":true,"limitations":[],
        "modules":[{
            "module":"domain","entrypoints":["domain/model.ts"],
            "exported":[{"file":"domain/model.ts","name":"FromSurface","target":{"file":"domain/model.ts","name":"FromSurface"}}],
            "used_externally":[],"could_be_internal":[],"deep_imports":[],"oversized":false
        }]
    });
    let surface = s.tour(&doc, &["--agent"]);
    assert!(surface.contains("FromSurface"), "{surface}");
    let file_mode = s.tour(&doc, &["--agent", "--granularity", "file"]);
    assert_order(&file_mode, &["domain/model.ts"]);
    assert!(file_mode.contains("FromSurface"), "{file_mode}");
}

#[test]
fn default_input_and_legacy_schema_work_without_writes_and_invalid_input_fails() {
    let s = Scratch::new();
    let mut doc = graph(&["domain/model.ts"], &[]);
    doc["schema"] = json!("archgraph/v1");
    let explicit = s.tour(&doc, &[]);
    assert_order(&explicit, &["domain"]);
    let default_dir = s.0.join(".archgraph");
    fs::create_dir(&default_dir).unwrap();
    let default_graph = default_dir.join("archgraph.json");
    fs::copy(s.0.join("graph.json"), &default_graph).unwrap();
    let original = fs::read(&default_graph).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .current_dir(&s.0)
        .arg("tour")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), explicit);
    assert_eq!(fs::read_dir(&s.0).unwrap().count(), 2);
    assert_eq!(fs::read_dir(&default_dir).unwrap().count(), 1);
    assert_eq!(fs::read(&default_graph).unwrap(), original);
    fs::write(&default_graph, r#"{"schema":"archgraph/v999"}"#).unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .current_dir(&s.0)
        .arg("tour")
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(2));
    assert!(failed.stdout.is_empty());
    assert_eq!(fs::read_dir(&default_dir).unwrap().count(), 1);
    assert!(
        s.tour(&graph(&[], &[]), &[])
            .contains("No production modules")
    );
}

#[test]
fn double_digit_agent_stops_keep_markdown_nesting_and_input_order_is_irrelevant() {
    let s = Scratch::new();
    let paths: Vec<_> = (0..12).map(|n| format!("module{n:02}/api.ts")).collect();
    let mut doc = graph(&paths.iter().map(String::as_str).collect::<Vec<_>>(), &[]);
    doc["edges"] = json!(
        paths
            .windows(2)
            .map(|pair| { json!({"from":pair[0],"to":pair[1],"kind":"runtime"}) })
            .collect::<Vec<_>>()
    );
    doc["symbols"] = json!({
        "complete":true,"imports":[],"limitations":[],
        "exports":paths.iter().enumerate().map(|(n,path)| {
            json!({"file":path,"name":format!("Api{n:02}"),"target":{"file":path,"name":format!("Api{n:02}")}})
        }).collect::<Vec<_>>()
    });
    let output = s.tour(&doc, &["--agent"]);
    assert_eq!(stops(&output).len(), 12, "{output}");
    let tenth = output
        .split("10. ")
        .nth(1)
        .unwrap()
        .split("11. ")
        .next()
        .unwrap();
    assert!(tenth.starts_with("` module02 `"), "{tenth}");
    assert!(tenth.contains("\n    Why here:"), "{tenth}");
    assert!(tenth.contains("\n    Surface ` module02 `:"), "{tenth}");
    assert!(tenth.contains("\n    - ` Api02 `"), "{tenth}");
    doc["files"].as_array_mut().unwrap().reverse();
    doc["edges"].as_array_mut().unwrap().reverse();
    assert_eq!(s.tour(&doc, &["--agent"]), output);
}
