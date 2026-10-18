mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use archgraph::model::{Config, Document, Edge, EdgeKind, Source};
use serde_json::{Value, json};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Impact Author",
            "-c",
            "user.email=impact@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

struct Fixture {
    dir: PathBuf,
    root: PathBuf,
    base: String,
    head: String,
    graph: PathBuf,
    base_graph: PathBuf,
}

impl Fixture {
    fn new(owned: bool, deleted: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "impact-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = dir.join("repo");
        for path in ["src", "app", "tests", "far"] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        git(&root, &["init", "-q"]);
        for file in [
            "src/util.ts",
            "app/app.ts",
            "tests/app.spec.ts",
            "far/far.ts",
        ] {
            fs::write(root.join(file), "export const value = 1;\n").unwrap();
        }
        if owned {
            fs::write(root.join("CODEOWNERS"), "* @team\n").unwrap();
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "initial architecture"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        let base_graph = dir.join("base.json");
        fs::write(
            &base_graph,
            serde_json::to_vec(&graph(&base, owned, false)).unwrap(),
        )
        .unwrap();
        if deleted {
            fs::remove_file(root.join("src/util.ts")).unwrap();
        } else {
            fs::write(root.join("src/util.ts"), "export const value = 2;\n").unwrap();
        }
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "change utility"]);
        let head = git(&root, &["rev-parse", "HEAD"]);
        let graph_path = dir.join("head.json");
        fs::write(
            &graph_path,
            serde_json::to_vec(&graph(&head, owned, deleted)).unwrap(),
        )
        .unwrap();
        Self {
            dir,
            root,
            base,
            head,
            graph: graph_path,
            base_graph,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_archgraph"));
        cmd.args(["impact", &format!("{}..{}", self.base, self.head), "--root"])
            .arg(&self.root)
            .arg("--graph")
            .arg(&self.graph)
            .args(["--format", "json"]);
        cmd
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn graph(commit: &str, owned: bool, deleted: bool) -> Document {
    let source: Source = serde_json::from_value(json!({
        "language":"typescript", "collector":{"name":"fixture","version":null,"cruised":4},
        "git":{"commit":commit,"dirty":false},"generated_at":"2026-10-17T00:00:00Z"
    }))
    .unwrap();
    let mut config: Config = serde_json::from_value(json!({
        "depth":1,"source_root":".","excludes":[],"include_tests":true,"rules":null
    }))
    .unwrap();
    if owned {
        config.codeowners = Some(archgraph::codeowners::parse("CODEOWNERS", ".", "* @team\n"));
    }
    let pairs = [
        ("app/app.ts", "src/util.ts"),
        ("tests/app.spec.ts", "app/app.ts"),
        ("far/far.ts", "tests/app.spec.ts"),
    ];
    let files = [
        "src/util.ts",
        "app/app.ts",
        "tests/app.spec.ts",
        "far/far.ts",
    ]
    .into_iter()
    .filter(|path| !deleted || *path != "src/util.ts")
    .map(str::to_owned)
    .collect();
    let edges = pairs
        .into_iter()
        .filter(|(_, to)| !deleted || *to != "src/util.ts")
        .map(|(from, to)| Edge {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Static,
            verdict: None,
            rule: None,
            reach: None,
            ..archgraph::model::Edge::default()
        })
        .collect();
    archgraph::analyze(source, config, files, edges, vec![], None).unwrap()
}

fn parse(out: Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    common::validate(&value);
    value
}

#[test]
fn impact_cli_selects_transitive_tests_exports_and_codeowners_without_changing_checkout() {
    let fixture = Fixture::new(true, false);
    fs::write(fixture.root.join("app/app.ts"), "dirty working copy\n").unwrap();
    let before = git(&fixture.root, &["status", "--porcelain"]);
    let tests = fixture.dir.join("tests.txt");
    let doc = parse(
        fixture
            .command()
            .args(["--max-depth", "2", "--tests-output"])
            .arg(&tests)
            .output()
            .unwrap(),
    );
    assert_eq!(doc["kind"], "impact");
    assert_eq!(doc["changed_files"], json!(["src/util.ts"]));
    assert_eq!(doc["selected_tests"], json!(["tests/app.spec.ts"]));
    assert_eq!(doc["affected_modules"].as_array().unwrap().len(), 3);
    assert_eq!(doc["affected_modules"][2]["depth"], 2);
    assert_eq!(doc["affected_modules"][2]["weight"], 1.0 / 3.0);
    assert_eq!(doc["reviewers"][0]["identity"], "@team");
    assert_eq!(doc["reviewers"][0]["source"], "codeowners");
    assert_eq!(fs::read_to_string(tests).unwrap(), "tests/app.spec.ts\n");
    assert_eq!(git(&fixture.root, &["status", "--porcelain"]), before);
    assert_eq!(git(&fixture.root, &["rev-parse", "HEAD"]), fixture.head);
}

#[test]
fn impact_cli_defaults_to_success_and_depth_failure_is_opt_in() {
    let fixture = Fixture::new(false, false);
    let doc = parse(fixture.command().output().unwrap());
    assert_eq!(doc["affected_modules"].as_array().unwrap().len(), 4);
    assert!(
        doc["reviewers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reviewer| reviewer["source"] == "git"
                && reviewer["identity"] == "Impact Author <impact@example.invalid>"
                && reviewer["commits"].as_u64().unwrap() >= 1)
    );
    let out = fixture
        .command()
        .args(["--fail-on-depth", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["kind"], "impact");
    assert!(
        fixture
            .command()
            .args(["--max-depth", "0", "--fail-on-depth", "0"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "impact",
            &format!("{}..{}", fixture.base, fixture.head),
            "--root",
        ])
        .arg(&fixture.root)
        .arg("--collector-input")
        .arg(&fixture.graph)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--collector-input"));
}

#[test]
fn impact_cli_retains_deleted_file_dependents_from_base_graph() {
    let fixture = Fixture::new(true, true);
    let doc = parse(
        fixture
            .command()
            .arg("--base-graph")
            .arg(&fixture.base_graph)
            .output()
            .unwrap(),
    );
    assert_eq!(doc["changed_files"], json!(["src/util.ts"]));
    assert_eq!(doc["unscanned_files"], json!([]));
    assert_eq!(doc["selected_tests"], json!(["tests/app.spec.ts"]));
    assert!(
        doc["affected_modules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|module| module["id"] == "app")
    );
    let limited = parse(fixture.command().output().unwrap());
    assert_eq!(limited["unscanned_files"], json!(["src/util.ts"]));
    assert!(
        limited["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("supply --base-graph"))
    );
}

#[test]
fn impact_cli_rejects_stale_or_dirty_graph_and_invalid_ranges() {
    let fixture = Fixture::new(false, false);
    let out = fixture
        .command()
        .arg("--base-graph")
        .arg(&fixture.graph)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("clean scan"));
    let mut saved: Value = serde_json::from_slice(&fs::read(&fixture.graph).unwrap()).unwrap();
    saved["source"]["git"]["dirty"] = json!(true);
    fs::write(&fixture.graph, serde_json::to_vec(&saved).unwrap()).unwrap();
    let out = fixture.command().output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("clean scan"));
    for range in ["HEAD", "HEAD...HEAD", "HEAD..", "..HEAD"] {
        let out = common::archgraph(&["impact", range, "--root", fixture.root.to_str().unwrap()]);
        assert!(!out.status.success(), "accepted {range}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("impact expects"));
    }
}

#[test]
fn impact_cli_additional_test_globs_and_markdown_describe_lower_bound() {
    let fixture = Fixture::new(false, false);
    let doc = parse(
        fixture
            .command()
            .args(["--tests", "app/*.ts"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        doc["selected_tests"],
        json!(["app/app.ts", "tests/app.spec.ts"])
    );
    let out = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "impact",
            &format!("{}..{}", fixture.base, fixture.head),
            "--root",
        ])
        .arg(&fixture.root)
        .arg("--graph")
        .arg(&fixture.graph)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("At least these"));
    assert!(text.contains("Architecture impact"));
    assert!(text.contains("Impact Author"));
}

#[test]
#[cfg(unix)]
fn impact_cli_actions_posts_reviewable_summary_with_fake_gh() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new(true, false);
    let bin = fixture.dir.join("bin");
    fs::create_dir(&bin).unwrap();
    let gh = bin.join("gh");
    fs::write(&gh, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IMPACT_GH_ARGS\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = --body-file ]; then cp \"$2\" \"$IMPACT_GH_BODY\"; exit 0; fi\n  shift\ndone\nexit 1\n").unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let args = fixture.dir.join("gh-args.txt");
    let body = fixture.dir.join("comment.md");
    let summary = fixture.dir.join("summary.md");
    let event = fixture.dir.join("event.json");
    fs::write(&event, r#"{"pull_request":{"number":4521}}"#).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let doc = parse(
        fixture
            .command()
            .arg("--github-actions")
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_EVENT_PATH", event)
            .env("GITHUB_STEP_SUMMARY", &summary)
            .env("IMPACT_GH_ARGS", &args)
            .env("IMPACT_GH_BODY", &body)
            .output()
            .unwrap(),
    );
    assert_eq!(doc["selected_tests"], json!(["tests/app.spec.ts"]));
    let arguments = fs::read_to_string(args).unwrap();
    assert!(arguments.starts_with("pr\ncomment\n4521\n--body-file\n"));
    let comment = fs::read_to_string(body).unwrap();
    assert!(comment.contains("At least these"));
    assert!(comment.contains("@team"));
    assert_eq!(fs::read_to_string(summary).unwrap(), comment);
}
