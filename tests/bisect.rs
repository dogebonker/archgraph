use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use archgraph::bisect::{self, Options, Target};
use archgraph::model::{Config, CycleLevel, Document, Edge, EdgeKind, Git, Source};
use serde_json::json;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Test Author",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
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

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "bisect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("src/a")).unwrap();
        fs::create_dir_all(path.join("src/b")).unwrap();
        git(&path, &["init", "-q"]);
        fs::write(path.join("src/b/b.ts"), "import '../a/a';\n").unwrap();
        Self(path)
    }
    fn commit(&self, cycle: bool, message: &str) -> String {
        fs::write(
            self.0.join("src/a/a.ts"),
            if cycle {
                "import '../b/b';\n"
            } else {
                "export const a = 1;\n"
            },
        )
        .unwrap();
        // Give every state a unique tree even if the import state did not change.
        fs::write(self.0.join("counter"), message).unwrap();
        git(&self.0, &["add", "."]);
        git(&self.0, &["commit", "-qm", message]);
        git(&self.0, &["rev-parse", "HEAD"])
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn graph(commit: &str, cycle: bool, language: &str) -> Document {
    let source: Source = serde_json::from_value(json!({
        "language":language, "collector":{"name":"fixture", "version":null,"cruised":2},
        "git":null,"generated_at":"2026-10-17T00:00:00Z"
    }))
    .unwrap();
    let config: Config = serde_json::from_value(json!({
        "depth":1,"source_root":"src","excludes":[],"include_tests":false,"rules":null
    }))
    .unwrap();
    let mut edges = vec![Edge {
        from: "src/b/b.ts".into(),
        to: "src/a/a.ts".into(),
        kind: EdgeKind::Static,
        verdict: None,
        rule: None,
        reach: None,
        ..archgraph::model::Edge::default()
    }];
    if cycle {
        edges.push(Edge {
            from: "src/a/a.ts".into(),
            to: "src/b/b.ts".into(),
            kind: EdgeKind::Static,
            verdict: None,
            rule: None,
            reach: None,
            ..archgraph::model::Edge::default()
        });
    }
    let mut doc = archgraph::analyze(
        source,
        config,
        vec!["src/a/a.ts".into(), "src/b/b.ts".into()],
        edges,
        vec![],
        None,
    )
    .unwrap();
    doc.source.git = Some(Git {
        commit: commit.into(),
        dirty: false,
    });
    doc
}
fn target() -> Target {
    Target::Cycle {
        level: CycleLevel::Module,
        members: vec!["a".into(), "b".into()],
    }
}

#[test]
fn modified_file_pickaxe_finds_verified_cycle_and_offline_pr() {
    let repo = Repo::new();
    let absent = repo.commit(false, "initial");
    let added = repo.commit(true, "close cycle (#6123)");
    let head = repo.commit(true, "unrelated");
    let result = bisect::investigate(
        &repo.0,
        &graph(&head, true, "typescript"),
        target(),
        &Options::default(),
        |oid| Ok(graph(oid, oid != absent, "typescript")),
    )
    .unwrap();
    assert_eq!(result.status, "verified-transition");
    assert_eq!(result.method, "edge");
    let commit = result.introduced_by.unwrap();
    assert_eq!(commit.commit, added);
    assert_eq!(commit.author, "Test Author");
    assert_eq!(commit.email, "test@example.invalid");
    assert_eq!(commit.pull_request, Some(6123));
    assert_eq!(result.previous_commit, Some(absent));
    assert_eq!(result.closing_edge.unwrap().from, "src/a/a.ts");
    assert!(result.scans <= 3);
    assert_eq!(git(&repo.0, &["rev-parse", "HEAD"]), head);
}

#[test]
fn follows_rename_and_selects_reintroduction_in_nonmonotonic_history() {
    let repo = Repo::new();
    let first = repo.commit(false, "before");
    repo.commit(true, "first introduction");
    let removed = repo.commit(false, "removed");
    let restored = repo.commit(true, "reintroduced");
    git(&repo.0, &["mv", "src/a/a.ts", "src/a/renamed.ts"]);
    git(&repo.0, &["commit", "-qm", "rename importer"]);
    let head = git(&repo.0, &["rev-parse", "HEAD"]);
    // Keep module identity stable across a renamed source file.
    let mut renamed = graph(&head, true, "typescript");
    for file in &mut renamed.files {
        if file.path == "src/a/a.ts" {
            file.path = "src/a/renamed.ts".into();
        }
    }
    for edge in &mut renamed.edges {
        if edge.from == "src/a/a.ts" {
            edge.from = "src/a/renamed.ts".into();
        }
        if edge.to == "src/a/a.ts" {
            edge.to = "src/a/renamed.ts".into();
        }
    }
    let result = bisect::investigate(&repo.0, &renamed, target(), &Options::default(), |oid| {
        Ok(graph(oid, oid != first && oid != removed, "typescript"))
    });
    let result = result.unwrap();
    assert_eq!(result.method, "edge");
    assert_eq!(result.status, "verified-transition");
    assert_eq!(result.introduced_by.unwrap().commit, restored);
}

#[test]
fn binary_fallback_reports_verified_local_boundary_with_nonmonotonic_caveat() {
    let repo = Repo::new();
    let mut commits = vec![];
    for index in 0..16 {
        commits.push(repo.commit(index >= 7, &format!("change {index}")));
    }
    let head = commits.last().unwrap();
    let result = bisect::investigate(
        &repo.0,
        &graph(head, true, "rust"),
        target(),
        &Options::default(),
        |oid| {
            let index = commits.iter().position(|commit| commit == oid).unwrap();
            Ok(graph(oid, index >= 7, "rust"))
        },
    )
    .unwrap();
    assert_eq!(result.method, "scan-bisect");
    assert_eq!(result.introduced_by.unwrap().commit, commits[7]);
    assert_eq!(result.previous_commit, Some(commits[6].clone()));
    assert!(result.scans <= 6);
    assert!(
        result
            .limitations
            .iter()
            .any(|limit| limit.contains("monotonic"))
    );
}

#[test]
fn bounded_and_failed_scans_do_not_invent_an_author() {
    let repo = Repo::new();
    let first = repo.commit(false, "before");
    let added = repo.commit(true, "added");
    let head = repo.commit(true, "later");
    let opts = Options {
        from: Some(added.clone()),
        ..Options::default()
    };
    let bounded = bisect::investigate(
        &repo.0,
        &graph(&head, true, "rust"),
        target(),
        &opts,
        |oid| Ok(graph(oid, true, "rust")),
    )
    .unwrap();
    assert_eq!(bounded.status, "present-at-boundary");
    assert!(bounded.introduced_by.is_none());
    let failure = bisect::investigate(
        &repo.0,
        &graph(&head, true, "rust"),
        target(),
        &Options::default(),
        |oid| {
            if oid == first {
                anyhow::bail!("historic collector unavailable");
            }
            Ok(graph(oid, true, "rust"))
        },
    )
    .unwrap();
    assert_eq!(failure.status, "unknown");
    assert!(failure.introduced_by.is_none());
    assert!(
        failure
            .limitations
            .iter()
            .any(|limit| limit.contains("historic collector unavailable"))
    );
}

#[test]
fn exhausted_scan_budget_and_dirty_only_findings_remain_unattributed() {
    let repo = Repo::new();
    let mut commits = Vec::new();
    for index in 0..8 {
        commits.push(repo.commit(index >= 4, &format!("change {index}")));
    }
    let head = commits.last().unwrap();
    let options = Options {
        max_scans: 2,
        ..Options::default()
    };
    let result = bisect::investigate(
        &repo.0,
        &graph(head, true, "rust"),
        target(),
        &options,
        |oid| {
            Ok(graph(
                oid,
                commits.iter().position(|commit| commit == oid).unwrap() >= 4,
                "rust",
            ))
        },
    )
    .unwrap();
    assert_eq!(result.status, "unknown");
    assert_eq!(result.scans, 2);
    assert!(result.introduced_by.is_none());
    let result = bisect::investigate(
        &repo.0,
        &graph(head, true, "rust"),
        target(),
        &options,
        |oid| Ok(graph(oid, false, "rust")),
    )
    .unwrap();
    assert_eq!(result.status, "not-present");
    assert!(result.introduced_by.is_none());
}

#[test]
fn forbidden_edge_requires_verdict_and_notes_fixed_rules() {
    let repo = Repo::new();
    let absent = repo.commit(false, "before");
    let introduced = repo.commit(true, "forbidden import");
    let forbidden = |oid: &str| {
        let mut doc = graph(oid, oid != absent, "typescript");
        for edge in &mut doc.edges {
            if edge.from == "src/a/a.ts" {
                edge.verdict = Some(archgraph::model::Verdict::Forbidden);
            }
        }
        doc
    };
    let result = bisect::investigate(
        &repo.0,
        &forbidden(&introduced),
        Target::ForbiddenEdge {
            from: "src/a/a.ts".into(),
            to: "src/b/b.ts".into(),
        },
        &Options::default(),
        |oid| Ok(forbidden(oid)),
    )
    .unwrap();
    assert_eq!(result.status, "verified-transition");
    assert_eq!(result.introduced_by.unwrap().commit, introduced);
    assert!(
        result
            .limitations
            .iter()
            .any(|limit| limit.contains("fixed rules"))
    );
}

#[test]
fn renamed_cycle_identity_is_not_mislabeled_as_architecture_birth() {
    let repo = Repo::new();
    let original = repo.commit(true, "existing cycle");
    git(&repo.0, &["mv", "src/a/a.ts", "src/a/renamed.ts"]);
    git(&repo.0, &["commit", "-qm", "rename importer"]);
    let head = git(&repo.0, &["rev-parse", "HEAD"]);
    let renamed = |oid: &str| {
        let mut doc = graph(oid, true, "rust");
        doc.cycles[0].level = CycleLevel::File;
        doc.cycles[0].members = vec!["src/a/a.ts".into(), "src/b/b.ts".into()];
        if oid != original {
            for cycle in &mut doc.cycles {
                if cycle.level == CycleLevel::File {
                    for member in &mut cycle.members {
                        if member == "src/a/a.ts" {
                            *member = "src/a/renamed.ts".into();
                        }
                    }
                }
            }
        }
        doc
    };
    let result = bisect::investigate(
        &repo.0,
        &renamed(&head),
        Target::Cycle {
            level: CycleLevel::File,
            members: vec!["src/a/renamed.ts".into(), "src/b/b.ts".into()],
        },
        &Options::default(),
        |oid| Ok(renamed(oid)),
    )
    .unwrap();
    assert_eq!(result.status, "unknown");
    assert!(result.introduced_by.is_none());
    assert!(
        result
            .limitations
            .iter()
            .any(|limit| limit.contains("identity change"))
    );
}

#[test]
#[cfg(unix)]
fn cli_bisect_and_report_keep_verified_provenance_and_checkout() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    let recording = |cycle| {
        json!({
        "modules":[
            {"source":"src/a/a.ts","dependencies":if cycle {vec![json!({"resolved":"src/b/b.ts","dependencyTypes":["local","import"]})]} else {vec![]}},
            {"source":"src/b/b.ts","dependencies":[{"resolved":"src/a/a.ts","dependencyTypes":["local","import"]}]}
        ],"summary":{"totalCruised":2,"environment":{"version":"18.4.0"}}
    }).to_string()
    };
    fs::write(repo.0.join("depcruise.json"), recording(false)).unwrap();
    repo.commit(false, "before");
    fs::write(repo.0.join("depcruise.json"), recording(true)).unwrap();
    let introducing = repo.commit(true, "cycle (#6123)");
    let collector = repo.0.join("collector.sh");
    fs::write(&collector, "#!/bin/sh\ncat depcruise.json\n").unwrap();
    fs::set_permissions(&collector, fs::Permissions::from_mode(0o755)).unwrap();
    let root = repo.0.to_str().unwrap();
    let collector = collector.to_str().unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let scan = run(&["scan", root, "--depcruise", collector, "--depth", "1"]);
    let graph = repo.0.join("graph.json");
    fs::write(&graph, scan.stdout).unwrap();
    let before = git(&repo.0, &["status", "--porcelain"]);
    let result = run(&[
        "bisect",
        graph.to_str().unwrap(),
        "--root",
        root,
        "--depcruise",
        collector,
    ]);
    let investigation: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(investigation["status"], "verified-transition");
    assert_eq!(investigation["introduced_by"]["commit"], introducing);
    assert_eq!(investigation["introduced_by"]["pull_request"], 6123);
    let markdown = run(&[
        "bisect",
        graph.to_str().unwrap(),
        "--root",
        root,
        "--depcruise",
        collector,
        "--format",
        "markdown",
    ]);
    let markdown = String::from_utf8(markdown.stdout).unwrap();
    assert!(markdown.contains("Verified absent → present"));
    assert!(markdown.contains("Test Author"));
    assert!(markdown.contains("PR #6123"));
    assert!(markdown.contains("Historical CI installation was not checked"));
    let bounded = run(&[
        "bisect",
        graph.to_str().unwrap(),
        "--root",
        root,
        "--depcruise",
        collector,
        "--from",
        &introducing,
    ]);
    let bounded: serde_json::Value = serde_json::from_slice(&bounded.stdout).unwrap();
    assert_eq!(bounded["status"], "present-at-boundary");
    assert!(bounded["introduced_by"].is_null());
    assert_eq!(git(&repo.0, &["status", "--porcelain"]), before);
    assert_eq!(git(&repo.0, &["rev-parse", "HEAD"]), introducing);
    run(&["report", root, "--depcruise", collector, "--depth", "1"]);
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.0.join(".archgraph/archgraph.json")).unwrap())
            .unwrap();
    let finding = document["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "cycle")
        .unwrap();
    assert_eq!(finding["provenance"]["status"], "verified-transition");
    assert_eq!(finding["introduced_by"]["commit"], introducing);
    assert!(finding["age_days"].is_u64());
    for finding in document["findings"].as_array().unwrap() {
        assert!(finding["provenance"].is_object());
        assert_ne!(finding["provenance"]["status"], "unsupported");
        if finding["introduced_by"].is_object() {
            assert!(finding["age_days"].is_u64());
        }
    }
    let rendered = repo.0.join("rendered.html");
    run(&[
        "render",
        repo.0.join(".archgraph/archgraph.json").to_str().unwrap(),
        "--output",
        rendered.to_str().unwrap(),
    ]);
    assert!(fs::read_to_string(rendered).unwrap().contains(&introducing));
}

#[test]
fn generic_misplaced_finding_has_a_verified_introduction() {
    use archgraph::model::{Evidence, Finding, FindingKind};
    let repo = Repo::new();
    let absent = repo.commit(false, "before placement concern");
    let introduced = repo.commit(false, "placement concern appears");
    let head = repo.commit(false, "later");
    let observed = |oid: &str| {
        let mut doc = graph(oid, false, "rust");
        if oid != absent {
            doc.findings.push(Finding {
                kind: FindingKind::MisplacedFile,
                subject: "src/a/a.ts".into(),
                evidence: Evidence::MisplacedFile {
                    own_module_edges: 0,
                    target_module: "b".into(),
                    imports_from_target: 2,
                    via_barrel: false,
                    ownership: None,
                },
                suggestion: None,
                simulation: None,
                age_days: None,
                introduced_by: None,
                provenance: None,
                hotspot_priority: None,
            });
        }
        doc
    };
    let doc = observed(&head);
    let target = Target::finding(&doc, doc.findings.len() - 1).unwrap();
    assert!(
        matches!(target,Target::Finding { ref finding_kind, .. } if finding_kind == "misplaced-file")
    );
    let result = bisect::investigate(&repo.0, &doc, target, &Options::default(), |oid| {
        Ok(observed(oid))
    })
    .unwrap();
    assert_eq!(result.status, "verified-transition");
    assert_eq!(result.method, "scan-bisect");
    assert_eq!(result.introduced_by.unwrap().commit, introduced);
    assert_eq!(result.previous_commit, Some(absent));
    assert!(result.closing_edge.is_none());
}

#[test]
#[cfg(unix)]
fn report_dates_hidden_coupling_with_the_original_cochange_window() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    fs::write(repo.0.join("depcruise.json"),json!({
        "modules":[{"source":"src/a/a.ts","dependencies":[]},{"source":"src/b/b.ts","dependencies":[]}],
        "summary":{"totalCruised":2,"environment":{"version":"18.4.0"}}
    }).to_string()).unwrap();
    let mut commits = Vec::new();
    for index in 0..4 {
        fs::write(
            repo.0.join("src/a/a.ts"),
            format!("export const a = {index};\n"),
        )
        .unwrap();
        fs::write(
            repo.0.join("src/b/b.ts"),
            format!("export const b = {index};\n"),
        )
        .unwrap();
        git(&repo.0, &["add", "."]);
        git(
            &repo.0,
            &["commit", "-qm", &format!("coordinated change {index}")],
        );
        commits.push(git(&repo.0, &["rev-parse", "HEAD"]));
    }
    let collector = repo.0.join("collector.sh");
    fs::write(&collector, "#!/bin/sh\ncat depcruise.json\n").unwrap();
    fs::set_permissions(&collector, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "report",
            repo.0.to_str().unwrap(),
            "--depth",
            "1",
            "--depcruise",
            collector.to_str().unwrap(),
            "--cochange=300",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.0.join(".archgraph/archgraph.json")).unwrap())
            .unwrap();
    let finding = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "hidden-coupling")
        .unwrap();
    assert_eq!(finding["provenance"]["status"], "verified-transition");
    assert_eq!(finding["introduced_by"]["commit"], commits[2]);
    assert_eq!(finding["provenance"]["previous_commit"], commits[1]);
    assert!(finding["age_days"].is_u64());
}

#[test]
fn backdated_commits_never_turn_sparse_probes_into_an_adjacent_transition() {
    let repo = Repo::new();
    let commit_at = |cycle, message, date| {
        repo.commit(cycle, message);
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo.0)
            .args([
                "-c",
                "user.name=Test Author",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--amend",
                "--no-edit",
                "--no-gpg-sign",
                "--date",
                date,
            ])
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        git(&repo.0, &["rev-parse", "HEAD"])
    };
    let absent = commit_at(false, "before", "2026-01-01T00:00:00Z");
    let actual_introduction = commit_at(true, "backdated introduction", "2024-01-01T00:00:00Z");
    let head = commit_at(true, "unchanged architecture", "2026-10-01T00:00:00Z");
    assert_eq!(
        git(&repo.0, &["rev-parse", &format!("{head}^1")]),
        actual_introduction
    );
    for language in ["rust", "typescript"] {
        let result = bisect::investigate(
            &repo.0,
            &graph(&head, true, language),
            target(),
            &Options {
                since: "2025-01-01".into(),
                ..Options::default()
            },
            |oid| Ok(graph(oid, oid != absent, language)),
        )
        .unwrap();
        assert_ne!(
            result.status, "verified-transition",
            "{language}: {result:?}"
        );
        assert!(result.introduced_by.is_none(), "{language}: {result:?}");
        assert!(result.previous_commit.is_none());
    }
}

#[test]
fn a_type_only_import_is_never_named_as_the_runtime_cycle_closer() {
    let repo = Repo::new();
    fs::write(repo.0.join("src/a/0-types.ts"), "export {};\n").unwrap();
    fs::write(repo.0.join("src/b/types.ts"), "export interface Shape {}\n").unwrap();
    let absent = repo.commit(false, "before");
    fs::write(
        repo.0.join("src/a/0-types.ts"),
        "import type { Shape } from '../b/types';\n",
    )
    .unwrap();
    let introduced = repo.commit(true, "add runtime and type-only imports");
    let observed = |oid: &str| {
        let mut doc = graph(oid, oid != absent, "typescript");
        if oid != absent {
            doc.edges.push(Edge {
                from: "src/a/0-types.ts".into(),
                to: "src/b/types.ts".into(),
                kind: EdgeKind::Type,
                verdict: None,
                rule: None,
                reach: None,
                ..archgraph::model::Edge::default()
            });
        }
        archgraph::analyze(
            doc.source,
            doc.config,
            vec![
                "src/a/a.ts".into(),
                "src/b/b.ts".into(),
                "src/a/0-types.ts".into(),
                "src/b/types.ts".into(),
            ],
            doc.edges,
            vec![],
            None,
        )
        .unwrap()
    };
    let result = bisect::investigate(
        &repo.0,
        &observed(&introduced),
        target(),
        &Options::default(),
        |oid| Ok(observed(oid)),
    )
    .unwrap();
    assert_eq!(result.status, "verified-transition");
    assert_eq!(result.closing_edge.unwrap().from, "src/a/a.ts");
}
