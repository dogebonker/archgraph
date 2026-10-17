use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use archgraph::hotspots::{self, Options};
use archgraph::model::{Config, Document, Edge, EdgeKind, Git, Source};
use serde_json::json;

struct Repo(PathBuf);
impl Repo {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "hotspots-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "-q"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    fn write(&self, path: &str, text: &str) {
        let file = self.0.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    fn commit(&self, message: &str, date: &str, author: &str) -> String {
        self.git(&["add", "--all"]);
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args([
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                message,
            ])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", format!("{author}@example.com"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        self.git(&["rev-parse", "HEAD"])
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn graph(paths: &[&str], edges: &[(&str, &str)], commit: &str) -> Document {
    let mut source: Source = serde_json::from_value(json!({
        "language":"typescript", "collector":{"name":"test","version":null,"cruised":paths.len()},
        "generated_at":"test", "git":null
    }))
    .unwrap();
    source.git = Some(Git {
        commit: commit.into(),
        dirty: false,
    });
    let config: Config = serde_json::from_value(json!({
        "depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":null
    }))
    .unwrap();
    archgraph::analyze(
        source,
        config,
        paths.iter().map(|s| (*s).into()).collect(),
        edges
            .iter()
            .map(|(from, to)| Edge {
                from: (*from).into(),
                to: (*to).into(),
                kind: EdgeKind::Static,
                verdict: None,
                rule: None,
                reach: None,
            })
            .collect(),
        vec![],
        None,
    )
    .unwrap()
}

#[test]
fn wiring_even_and_move_resolved_pairs_are_not_extra_cut_candidates() {
    let repo = Repo::new("cut-flags");
    let paths = ["a/x.ts", "b/y.ts", "b/z.ts", "b/w.ts"];
    for path in paths {
        repo.write(path, "one\n");
    }
    let commit = repo.commit("baseline", "2026-01-01T12:00:00Z", "author");
    for flag in 0..5 {
        let mut graph = graph(
            &paths,
            &[
                ("a/x.ts", "b/y.ts"),
                ("b/y.ts", "a/x.ts"),
                ("b/z.ts", "a/x.ts"),
                ("b/w.ts", "a/x.ts"),
            ],
            &commit,
        );
        graph.cycles[0].break_set.clear();
        let pair = &mut graph.cycles[0].two_way.as_mut().unwrap()[0];
        pair.even = flag == 0;
        pair.registry = flag == 1;
        pair.entry = flag == 2;
        pair.resolved_by_move = flag == 3;
        let report = hotspots::collect(&repo.0, &graph, &Options::default()).unwrap();
        assert_eq!(report.files.iter().any(|f| f.cut_candidate), flag == 4);
    }
}

#[test]
fn factors_are_explainable_scoped_and_anchored_to_the_scanned_commit() {
    let repo = Repo::new("factors");
    for file in ["a/hot.ts", "b/b.ts", "c/c.ts", "d/cold.ts"] {
        repo.write(file, "one\n");
    }
    repo.write("vendor/noise", "one\n");
    repo.commit("baseline", "2018-01-01T12:00:00Z", "old");
    repo.write("a/hot.ts", "one\ntwo\n");
    repo.write("vendor/noise", "one\ntwo\n");
    repo.commit("fix: a regression", "2020-01-01T12:00:00Z", "alice");
    repo.write("a/hot.ts", "one\ntwo\nthree\n");
    repo.write("b/b.ts", "one\ntwo\n");
    let commit = repo.commit("bug: shared change", "2020-02-01T12:00:00Z", "bob");
    repo.write("d/cold.ts", "newer\n");
    repo.commit("fix: future", "2022-01-01T12:00:00Z", "future");
    let graph = graph(
        &["a/hot.ts", "b/b.ts", "c/c.ts", "d/cold.ts"],
        &[
            ("a/hot.ts", "b/b.ts"),
            ("b/b.ts", "a/hot.ts"),
            ("c/c.ts", "a/hot.ts"),
        ],
        &commit,
    );
    let report = hotspots::collect(
        &repo.0,
        &graph,
        &Options {
            authors: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(report.as_of, commit);
    assert_eq!(report.commits, 2);
    assert_eq!(report.files.len(), 4);
    let hot = &report.files[0];
    assert_eq!(hot.path, "a/hot.ts");
    assert_eq!(
        (
            hot.commits,
            hot.authors,
            hot.bugfix_commits,
            hot.lines_added,
            hot.lines_deleted
        ),
        (2, 2, 2, 2, 0)
    );
    assert_eq!(
        (
            hot.churn,
            hot.coupling,
            hot.cycle,
            hot.author_factor,
            hot.score,
            hot.cycle_size
        ),
        (1.0, 1.0, 1.0, 1.0, 4.0, 2)
    );
    assert!(!hot.finding_indices.is_empty());
    assert!(report.files.iter().any(|f| f.cut_candidate));
    let adjacent = report.files.iter().find(|f| f.path == "c/c.ts").unwrap();
    assert_eq!(
        (adjacent.cycle, adjacent.commits, adjacent.score),
        (0.5, 0, 0.0)
    );
    assert_eq!(report.validation.bugfix_commits, 2);
    assert_eq!(report.validation.top_bugfix_commits, 2);
    assert_eq!(report.validation.share, Some(1.0));
    assert_eq!(report.validation.lift, Some(4.0));
    assert!(hotspots::finding_priorities(&report, graph.findings.len()).contains(&4.0));
}

#[test]
fn follows_renames_with_tabs_newlines_and_subdirectory_scans() {
    let repo = Repo::new("rename");
    let old = "app/a/old\tname\n.ts";
    let new = "app/a/new\tname\n.ts";
    repo.write(old, "one\n");
    repo.commit("fix: initial", "2020-01-01T12:00:00Z", "alice");
    repo.git(&["mv", old, new]);
    let commit = repo.commit("rename", "2020-02-01T12:00:00Z", "bob");
    let path = "a/new\tname\n.ts";
    let graph = graph(&[path], &[], &commit);
    let report = hotspots::collect(&repo.0.join("app"), &graph, &Options::default()).unwrap();
    let file = &report.files[0];
    assert_eq!(file.path, path);
    assert_eq!(
        (
            file.commits,
            file.authors,
            file.lines_added,
            file.bugfix_commits
        ),
        (2, 2, 1, 1)
    );
    assert_eq!(file.score, 0.0);
}

#[test]
fn mechanical_filter_counts_files_outside_the_scanned_root() {
    let repo = Repo::new("mechanical");
    repo.write("app/a/a.ts", "one\n");
    repo.write("outside/one", "one\n");
    repo.write("outside/two", "one\n");
    repo.commit("fix: sweep", "2020-01-01T12:00:00Z", "alice");
    repo.write("app/a/a.ts", "one\ntwo\n");
    let commit = repo.commit("fix: scoped", "2020-02-01T12:00:00Z", "bob");
    let graph = graph(&["a/a.ts"], &[], &commit);
    let report = hotspots::collect(
        &repo.0.join("app"),
        &graph,
        &Options {
            max_files: 2,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((report.commits, report.skipped), (1, 1));
    assert_eq!(
        (
            report.files[0].commits,
            report.files[0].authors,
            report.files[0].lines_added
        ),
        (1, 1, 1)
    );
}

#[test]
fn shallow_history_is_explicitly_unavailable() {
    let repo = Repo::new("shallow");
    repo.write("a/a.ts", "one\n");
    let commit = repo.commit("fix", "2020-01-01T12:00:00Z", "alice");
    fs::write(repo.0.join(".git/shallow"), format!("{commit}\n")).unwrap();
    let graph = graph(&["a/a.ts"], &[], &commit);
    let report = hotspots::collect(&repo.0, &graph, &Options::default()).unwrap();
    assert!(report.shallow);
    assert_eq!(report.files[0].commits, 0);
    assert!(
        report
            .limitations
            .iter()
            .any(|s| s.contains("Shallow clone"))
    );
    assert_eq!(report.validation.share, None);
}

#[test]
fn options_are_validated_and_absolute_start_dates_work() {
    let repo = Repo::new("options");
    repo.write("a/a.ts", "one\n");
    let commit = repo.commit("prefix is not a bugfix", "2020-01-01T12:00:00Z", "alice");
    let graph = graph(&["a/a.ts"], &[], &commit);
    assert!(
        hotspots::collect(
            &repo.0,
            &graph,
            &Options {
                max_files: 0,
                ..Options::default()
            }
        )
        .is_err()
    );
    assert!(
        hotspots::collect(
            &repo.0,
            &graph,
            &Options {
                since: "yesterday".into(),
                ..Options::default()
            }
        )
        .is_err()
    );
    let report = hotspots::collect(
        &repo.0,
        &graph,
        &Options {
            since: "2020-01-02".into(),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(report.commits, 0);
}

#[test]
fn fix_concentration_baseline_accounts_for_commits_touching_many_files() {
    for all_files in [true, false] {
        let repo = Repo::new(if all_files { "fix-sweep" } else { "fix-local" });
        let paths: Vec<_> = (0..10).map(|i| format!("m{i}/f.ts")).collect();
        for path in &paths {
            repo.write(path, "one\n");
        }
        repo.commit("baseline", "2020-01-01T12:00:00Z", "alice");
        for path in paths.iter().take(if all_files { 10 } else { 1 }) {
            repo.write(path, "one\ntwo\n");
        }
        let commit = repo.commit("fix regression", "2020-02-01T12:00:00Z", "alice");
        let refs: Vec<_> = paths.iter().map(String::as_str).collect();
        let graph = graph(&refs, &[(&paths[0], &paths[1])], &commit);
        let report = hotspots::collect(&repo.0, &graph, &Options::default()).unwrap();
        assert_eq!(report.validation.share, Some(1.0));
        assert_eq!(report.validation.lift, Some(10.0));
        let expected = if all_files { 1.0 } else { 0.1 };
        assert!((report.validation.expected_random_share.unwrap() - expected).abs() < 1e-12);
        let random_lift = if all_files { 1.0 } else { 10.0 };
        assert!((report.validation.random_lift.unwrap() - random_lift).abs() < 1e-12);
    }
}

#[test]
fn validation_reads_older_cached_reports_without_random_baselines() {
    let validation: hotspots::Validation = serde_json::from_value(json!({
        "files":10,"top_files":1,"bugfix_commits":2,"top_bugfix_commits":1,
        "share":0.5,"lift":5.0
    }))
    .unwrap();
    assert_eq!(validation.expected_random_share, None);
    assert_eq!(validation.random_lift, None);
}
