mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{archgraph, validate};
use serde_json::{Value, json};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=archgraph-test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn repo(name: &str) -> (PathBuf, String, String) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("history-{name}-{}", std::process::id()));
    fs::create_dir_all(root.join("project/src/a")).unwrap();
    fs::create_dir_all(root.join("project/src/b")).unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("project/src/a/a.ts"), "export const a = 1;").unwrap();
    let recording = |cycle: bool| {
        json!({
        "modules": [
            {"source":"src/a/a.ts", "dependencies": if cycle {vec![json!({"resolved":"src/b/b.ts", "dependencyTypes":["local","import"]})]} else {vec![]}},
            {"source":"src/b/b.ts", "dependencies":[{"resolved":"src/a/a.ts", "dependencyTypes":["local","import"]}]}
        ], "summary":{"totalCruised":2,"environment":{"version":"18.4.0"}}
    }).to_string()
    };
    fs::write(
        root.join("project/src/b/b.ts"),
        "import { a } from '../a/a';",
    )
    .unwrap();
    fs::write(root.join("project/depcruise.json"), recording(false)).unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "before"]);
    let first = git(&root, &["rev-parse", "HEAD"]);
    fs::write(root.join("project/depcruise.json"), recording(true)).unwrap();
    fs::write(
        root.join("project/src/a/a.ts"),
        "import '../b/b'; export const a = 1;",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
        &["commit", "-qm", "cycle </script><script>alert(1)</script>"],
    );
    let second = git(&root, &["rev-parse", "HEAD"]);
    (root, first, second)
}

#[cfg(unix)]
fn collector(name: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let file = common::write_tmp(
        &format!("history-collector-{name}.sh"),
        "#!/bin/sh\ncat depcruise.json\n",
    );
    fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    file
}

#[test]
#[cfg(unix)]
fn history_cli_orders_snapshots_diffs_and_preserves_dirty_subdirectory_checkout() {
    let (root, first, second) = repo("timeline");
    fs::write(root.join("project/src/a/a.ts"), "dirty working copy").unwrap();
    fs::write(root.join("untracked.txt"), "keep").unwrap();
    let status = git(&root, &["status", "--porcelain"]);
    let worktrees = git(&root, &["worktree", "list", "--porcelain"]);
    let project = root.join("project");
    let output = root.join("reports");
    let out = archgraph(&[
        "history",
        "--sample",
        "commits",
        project.to_str().unwrap(),
        "--limit",
        "2",
        "--depcruise",
        &collector("timeline"),
        "--out-dir",
        output.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value =
        serde_json::from_slice(&fs::read(output.join("history.json")).unwrap()).unwrap();
    validate(&doc);
    assert_eq!(doc["kind"], "history");
    let snapshots = doc["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0]["commit"], first);
    assert_eq!(snapshots[1]["commit"], second);
    assert!(snapshots[0]["diff"].is_null());
    assert_eq!(snapshots[0]["graph"]["summary"]["cycles"], 0);
    assert_eq!(snapshots[1]["graph"]["summary"]["cycles"], 1);
    assert_eq!(
        snapshots[1]["diff"]["new_cycles"].as_array().unwrap().len(),
        1
    );
    for snapshot in snapshots {
        validate(&snapshot["graph"]);
        assert_eq!(snapshot["graph"]["source"]["git"]["dirty"], false);
        assert_eq!(
            snapshot["graph"]["source"]["git"]["commit"],
            snapshot["commit"]
        );
    }
    let page = fs::read_to_string(output.join("history.html")).unwrap();
    let budget = serde_json::to_string(include_str!("../assets/graph.html"))
        .unwrap()
        .replace('<', "\\u003c")
        .len()
        + serde_json::to_string(&doc)
            .unwrap()
            .replace('<', "\\u003c")
            .len()
        + include_str!("../src/history.html").len()
        + 1_000;
    assert!(
        page.len() < budget,
        "history duplicates the renderer or graph data: {} bytes, budget {budget}",
        page.len()
    );
    assert!(page.contains("Architecture history"));
    assert!(page.contains("sandbox=\"allow-scripts\""));
    assert!(!page.contains("cycle </script>"));
    assert!(page.contains("\\u003c/script>"));
    // Ignore only our requested report output when comparing status.
    assert_eq!(
        git(
            &root,
            &["status", "--porcelain", "--", "project", "untracked.txt"]
        ),
        status
    );
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), second);
    assert_eq!(git(&root, &["worktree", "list", "--porcelain"]), worktrees);
    assert_eq!(
        fs::read_to_string(project.join("src/a/a.ts")).unwrap(),
        "dirty working copy"
    );
}

#[test]
#[cfg(unix)]
fn history_supports_revision_ranges_limits_and_fixed_rules() {
    let (root, first, second) = repo("range");
    let rules = common::write_tmp(
        "history-layout.yaml",
        "version: 1\nforbid:\n  - { id: a-no-b, from: 'src/a/**', to: 'src/b/**' }\n",
    );
    let output = root.join("reports");
    let out = archgraph(&[
        "history",
        "--sample",
        "commits",
        root.join("project").to_str().unwrap(),
        "--rev",
        &format!("{first}..{second}"),
        "--limit",
        "1",
        "--rules",
        &rules,
        "--depcruise",
        &collector("range"),
        "--out-dir",
        output.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value =
        serde_json::from_slice(&fs::read(output.join("history.json")).unwrap()).unwrap();
    validate(&doc);
    assert_eq!(doc["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(doc["snapshots"][0]["commit"], second);
    assert_eq!(doc["snapshots"][0]["graph"]["summary"]["forbidden"], 1);
}

#[test]
#[cfg(unix)]
fn history_reuses_the_original_checkout_collector() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _, _) = repo("installed-collector");
    let bin = root.join("project/node_modules/.bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let program = bin.join("depcruise");
    fs::write(&program, "#!/bin/sh\ncat depcruise.json\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let out = archgraph(&[
        "history",
        "--sample",
        "commits",
        root.join("project").to_str().unwrap(),
        "--limit",
        "1",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("project/.archgraph/history.html").is_file());
}

#[test]
fn history_rejects_invalid_revisions_empty_ranges_and_reused_recordings() {
    let (root, first, _) = repo("errors");
    for rev in ["not-a-commit", "--all", &format!("{first}..{first}")] {
        let out = archgraph(&[
            "history",
            "--sample",
            "commits",
            root.to_str().unwrap(),
            "--rev",
            rev,
        ]);
        assert_eq!(out.status.code(), Some(2));
        assert!(!root.join(".archgraph/history.json").exists());
    }
    let out = archgraph(&[
        "history",
        "--sample",
        "commits",
        root.to_str().unwrap(),
        "--collector-input",
        "anything.json",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("live collectors"));
}

#[test]
fn java_history_scans_real_commits() {
    if !Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("JDK unavailable; Java history check skipped");
        return;
    }
    let file = common::write_tmp(
        &format!("history-java-{}/model/Order.java", std::process::id()),
        "package model; public class Order {}\n",
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    git(root, &["init", "-q"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "model"]);
    let first = git(root, &["rev-parse", "HEAD"]);
    fs::create_dir_all(root.join("service")).unwrap();
    fs::write(
        root.join("service/Checkout.java"),
        "package service; public class Checkout { model.Order order; }\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "service uses model"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("node_modules/.bin");
        fs::create_dir_all(&bin).unwrap();
        let program = bin.join("depcruise");
        fs::write(
            &program,
            "#!/bin/sh\ntouch \"$0.executed\"\necho 'unrelated repository code'\n",
        )
        .unwrap();
        fs::set_permissions(program, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = archgraph(&[
        "history",
        "--sample",
        "commits",
        root.to_str().unwrap(),
        "--limit",
        "2",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !root.join("node_modules/.bin/depcruise.executed").exists(),
        "Java history must not execute unrelated TypeScript tooling from the repository"
    );
    let doc: Value =
        serde_json::from_slice(&fs::read(root.join(".archgraph/history.json")).unwrap()).unwrap();
    validate(&doc);
    assert_eq!(doc["snapshots"][0]["commit"], first);
    assert_eq!(
        doc["snapshots"][0]["graph"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        doc["snapshots"][1]["graph"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        doc["snapshots"][1]["diff"]["added_modules"],
        json!(["service"])
    );
}

fn dated_commit(root: &Path, date: &str, label: &str) -> String {
    fs::write(root.join("marker"), label).unwrap();
    git(root, &["add", "."]);
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            label,
        ])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(root, &["rev-parse", "HEAD"])
}

fn drift_repo(name: &str) -> (PathBuf, Vec<String>) {
    let root =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("drift-{name}-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    fs::create_dir_all(root.join("src/a")).unwrap();
    fs::create_dir_all(root.join("src/b")).unwrap();
    fs::write(root.join("src/a/a.ts"), "export const a = 1;\n").unwrap();
    fs::write(root.join("src/b/b.ts"), "export const b = 1;\n").unwrap();
    let first = dated_commit(&root, "2020-01-01T12:00:00Z", "first January");
    let second = dated_commit(&root, "2020-01-20T12:00:00Z", "later January");
    git(
        &root,
        &["-c", "tag.gpgsign=false", "tag", "-am", "release one", "v1"],
    );
    let third = dated_commit(&root, "2020-02-03T12:00:00Z", "first February");
    git(&root, &["tag", "v2"]);
    let fourth = dated_commit(&root, "2020-03-01T12:00:00Z", "first March");
    (root, vec![first, second, third, fourth])
}

fn drift_graph(root: &Path) -> anyhow::Result<archgraph::model::Document> {
    use archgraph::model::{Config, Edge, EdgeKind, Source};
    let cycle = fs::read_to_string(root.join("marker"))?.contains("March");
    let source: Source = serde_json::from_value(
        json!({"language":"typescript","collector":{"name":"fixture","version":null,"cruised":2},"generated_at":"test","git":null}),
    )?;
    let config: Config = serde_json::from_value(
        json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":null}),
    )?;
    let mut edges = vec![Edge {
        from: "a/a.ts".into(),
        to: "b/b.ts".into(),
        kind: EdgeKind::Static,
        verdict: None,
        rule: None,
        reach: None,
    }];
    if cycle {
        edges.push(Edge {
            from: "b/b.ts".into(),
            to: "a/a.ts".into(),
            kind: EdgeKind::Static,
            verdict: None,
            rule: None,
            reach: None,
        });
    }
    archgraph::analyze(
        source,
        config,
        vec!["a/a.ts".into(), "b/b.ts".into()],
        edges,
        vec![],
        None,
    )
}

#[test]
fn drift_sampling_selects_first_monthly_commits_intervals_and_releases() {
    use archgraph::history::{HistoryOptions, Sampling, collect_with_options};
    let (root, commits) = drift_repo("samples");
    let monthly =
        collect_with_options(&root, "HEAD", &HistoryOptions::default(), drift_graph).unwrap();
    assert_eq!(
        monthly
            .snapshots
            .iter()
            .map(|s| &s.commit)
            .collect::<Vec<_>>(),
        vec![&commits[0], &commits[2], &commits[3]]
    );
    assert_eq!(monthly.release_markers.len(), 2);
    assert_eq!(monthly.release_markers[0].tags, vec!["v1"]);
    assert_eq!(monthly.release_markers[1].tags, vec!["v2"]);
    let summary = &monthly.snapshots[2].summary;
    assert_eq!(
        (
            summary.files,
            summary.modules,
            summary.edges,
            summary.cycles,
            summary.largest_cycle
        ),
        (2, 2, 2, 1, 2)
    );
    assert_eq!(
        (
            summary.modules_in_cycles_percent,
            summary.cycles_per_100_modules
        ),
        (100.0, 50.0)
    );
    assert_eq!(summary.forbidden, None);
    let every = collect_with_options(
        &root,
        "HEAD",
        &HistoryOptions {
            sampling: Sampling::Commits,
            every: 2,
            ..HistoryOptions::default()
        },
        drift_graph,
    )
    .unwrap();
    assert_eq!(
        every
            .snapshots
            .iter()
            .map(|s| &s.commit)
            .collect::<Vec<_>>(),
        vec![&commits[0], &commits[2]]
    );
    let tags = collect_with_options(
        &root,
        "HEAD",
        &HistoryOptions {
            sampling: Sampling::Tags,
            ..HistoryOptions::default()
        },
        drift_graph,
    )
    .unwrap();
    assert_eq!(
        tags.snapshots.iter().map(|s| &s.commit).collect::<Vec<_>>(),
        vec![&commits[1], &commits[2]]
    );
}

#[test]
fn drift_cache_append_keeps_old_points_and_scans_only_missing_contexts() {
    use archgraph::history::{HistoryOptions, collect_with_options};
    let (root, commits) = drift_repo("cache");
    let mut scans = 0;
    let mut scan = |path: &Path| {
        scans += 1;
        drift_graph(path)
    };
    let mut options = HistoryOptions {
        limit: 2,
        cache_dir: Some(root.join(".archgraph/history")),
        cache_key: "fixture-options-v1".into(),
        append: true,
        ..HistoryOptions::default()
    };
    let before = collect_with_options(&root, &commits[2], &options, &mut scan).unwrap();
    assert_eq!(before.snapshots.len(), 2);
    let after = collect_with_options(&root, "HEAD", &options, &mut scan).unwrap();
    assert_eq!(after.snapshots.len(), 3);
    assert_eq!(after.snapshots[0].commit, commits[0]);
    let repeated = collect_with_options(&root, "HEAD", &options, &mut scan).unwrap();
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&repeated).unwrap()
    );
    options.cache_key = "fixture-options-v2".into();
    assert_eq!(
        collect_with_options(&root, "HEAD", &options, &mut scan)
            .unwrap()
            .snapshots
            .len(),
        2
    );
    assert_eq!(
        scans, 5,
        "three initial commits plus two scans in the changed context"
    );
}

#[test]
fn drift_skips_failed_scans_but_rejects_an_entirely_failed_timeline() {
    use archgraph::history::{HistoryOptions, collect_with_options};
    let (root, commits) = drift_repo("failures");
    let options = HistoryOptions::default();
    let result = collect_with_options(&root, "HEAD", &options, |path| {
        if fs::read_to_string(path.join("marker"))?.contains("January") {
            anyhow::bail!("collector unavailable for old lockfile");
        }
        drift_graph(path)
    })
    .unwrap();
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].commit, commits[0]);
    assert_eq!(result.snapshots.len(), 2);
    assert!(result.snapshots[0].diff.is_none());
    assert!(result.snapshots[1].diff.is_some());
    assert!(
        collect_with_options(&root, "HEAD", &options, |_| anyhow::bail!("no collector"))
            .err()
            .unwrap()
            .to_string()
            .contains("all selected history snapshots failed")
    );
    let page = archgraph::history::render(&result).unwrap();
    assert!(page.contains("Architecture drift"));
    assert!(page.contains("Skipped snapshots"));
    assert!(!page.contains("__HISTORY_DATA__"));
}

#[test]
#[cfg(unix)]
fn drift_refuses_symlink_cache_directories() {
    use archgraph::history::{HistoryOptions, collect_with_options};
    let (root, _) = drift_repo("symlink");
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let cache = root.join("cache");
    std::os::unix::fs::symlink(&outside, &cache).unwrap();
    let result = collect_with_options(
        &root,
        "HEAD",
        &HistoryOptions {
            cache_dir: Some(cache),
            cache_key: "fixture".into(),
            ..HistoryOptions::default()
        },
        drift_graph,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("symlink history cache")
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
#[cfg(unix)]
fn history_cli_defaults_to_monthly_appends_from_cache_and_compares_release_refs() {
    use std::os::unix::fs::PermissionsExt;
    let (root, commits) = drift_repo("cli-cache");
    git(&root, &["tag", "v3"]);
    let program = common::write_tmp(
        &format!("history-drift-collector-{}.sh", std::process::id()),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo fixture-1; exit 0; fi\nprintf '%s\\n' scan >> \"$0.calls\"\ncase \"$(cat marker)\" in\n  *March*) cat \"$0.after\";;\n  *) cat \"$0.before\";;\nesac\n",
    );
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let recording = |cycle: bool| {
        json!({
            "modules":[
                {"source":"src/a/a.ts","dependencies":if cycle {vec![json!({"resolved":"src/b/b.ts","dependencyTypes":["local","import"]})]} else {vec![]}},
                {"source":"src/b/b.ts","dependencies":[{"resolved":"src/a/a.ts","dependencyTypes":["local","import"]}]}
            ],"summary":{"totalCruised":2,"environment":{"version":"18.4.0"}}
        })
    };
    fs::write(format!("{program}.before"), recording(false).to_string()).unwrap();
    fs::write(format!("{program}.after"), recording(true).to_string()).unwrap();
    let output = root.join("reports");
    let read_history = || -> Value {
        serde_json::from_slice(&fs::read(output.join("history.json")).unwrap()).unwrap()
    };
    let run = |rev: &str| {
        let out = archgraph(&[
            "history",
            root.to_str().unwrap(),
            "--rev",
            rev,
            "--limit",
            "2",
            "--append",
            "--language",
            "typescript",
            "--depcruise",
            &program,
            "--out-dir",
            output.to_str().unwrap(),
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let doc = read_history();
        validate(&doc);
        doc
    };
    let first = run(&commits[2]);
    assert_eq!(first["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(
        first["snapshots"][0]["commit"], commits[0],
        "default sampling uses the month's first commit"
    );
    let second = run("HEAD");
    assert_eq!(
        second["snapshots"].as_array().unwrap().len(),
        3,
        "append retains older cached points beyond the selection limit"
    );
    assert_eq!(second["snapshots"][2]["commit"], commits[3]);
    assert_eq!(second["snapshots"][2]["summary"]["cycles"], 1);
    let repeated = run("HEAD");
    assert_eq!(second, repeated);
    assert_eq!(
        fs::read_to_string(format!("{program}.calls"))
            .unwrap()
            .lines()
            .count(),
        3,
        "only missing commits invoke the collector"
    );
    let out = archgraph(&[
        "history",
        root.to_str().unwrap(),
        "--diff",
        "v2",
        "v3",
        "--language",
        "typescript",
        "--depcruise",
        &program,
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let markdown = String::from_utf8(out.stdout).unwrap();
    assert!(markdown.contains("1 new cycle"), "{markdown}");
    assert!(markdown.contains("0 resolved cycles"), "{markdown}");
    assert_eq!(
        read_history(),
        second,
        "release diff does not replace the cached timeline"
    );
}

#[test]
#[cfg(unix)]
fn history_cache_tracks_the_collector_package_not_only_the_interpreter() {
    use std::os::unix::fs::PermissionsExt;
    let file = common::write_tmp(
        &format!("grimp-cache-{}/repo/pkg/a.py", std::process::id()),
        "x = 1\n",
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
    fs::write(root.join("pkg/b.py"), "y = 1\n").unwrap();
    git(root, &["init", "-q"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "package"]);
    let program = root.parent().unwrap().join("python");
    fs::write(&program, "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Python 3.12.8'; else cat \"$0.json\"; fi\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let data = program.with_extension("json");
    let recording = |version: &str, imports: Value| json!({"grimp_version":version,"modules":["pkg","pkg.a","pkg.b"],"imports":imports});
    fs::write(&data, recording("3.17", json!([])).to_string()).unwrap();
    let output = root.parent().unwrap().join("reports");
    let run = || {
        let out = archgraph(&[
            "history",
            root.to_str().unwrap(),
            "--sample",
            "commits",
            "--limit",
            "1",
            "--append",
            "--language",
            "python",
            "--python",
            program.to_str().unwrap(),
            "--out-dir",
            output.to_str().unwrap(),
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&fs::read(output.join("history.json")).unwrap()).unwrap()
    };
    let first = run();
    assert_eq!(
        first["snapshots"][0]["graph"]["source"]["collector"]["version"],
        "3.17"
    );
    fs::write(
        &data,
        recording(
            "3.18",
            json!([
                {"importer":"pkg.a","imported":"pkg.b"},
                {"importer":"pkg.b","imported":"pkg.a"}
            ]),
        )
        .to_string(),
    )
    .unwrap();
    let second = run();
    assert_eq!(
        second["snapshots"][0]["graph"]["source"]["collector"]["version"],
        "3.18"
    );
    assert_eq!(
        second["snapshots"][0]["graph"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn history_retains_persisted_finding_context_without_scanning_cached_points() {
    use archgraph::history::{HistoryOptions, collect_with_options, persist_context};
    use archgraph::model::{FindingKind, Provenance};
    let (root, commits) = drift_repo("cached-context");
    let options = HistoryOptions {
        cache_dir: Some(root.join(".archgraph/history")),
        cache_key: "context-fixture".into(),
        append: true,
        ..HistoryOptions::default()
    };
    let mut scans = 0;
    let mut history = collect_with_options(&root, "HEAD", &options, |path| {
        scans += 1;
        drift_graph(path)
    })
    .unwrap();
    assert_eq!(scans, 3);
    let graph = &mut history.snapshots.last_mut().unwrap().graph;
    let finding = graph
        .findings
        .iter_mut()
        .find(|f| f.kind == FindingKind::Cycle)
        .unwrap();
    finding.age_days = Some(7);
    finding.provenance = Some(Provenance {
        status: "verified".into(),
        method: "diff".into(),
        from: Some(commits[2].clone()),
        to: commits[3].clone(),
        previous_commit: Some(commits[2].clone()),
        closing_edge: None,
        limitations: vec!["fixture verification".into()],
        scans: 1,
    });
    let expected = serde_json::to_value(&finding.provenance).unwrap();
    persist_context(&root, &options, graph).unwrap();
    let cached = collect_with_options(&root, "HEAD", &options, |_| {
        panic!("cached snapshots must not run their collectors")
    })
    .unwrap();
    let restored = cached
        .snapshots
        .last()
        .unwrap()
        .graph
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Cycle)
        .unwrap();
    assert_eq!(restored.age_days, Some(7));
    assert_eq!(
        serde_json::to_value(&restored.provenance).unwrap(),
        expected
    );
    assert!(persist_context(&root, &HistoryOptions::default(), graph).is_err());
    graph.source.git.as_mut().unwrap().commit = "../outside".into();
    assert!(persist_context(&root, &options, graph).is_err());
}
