//! Cache-provenance and startup races, using real isolated Git worktrees.
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    root: PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new(nested: bool) -> Self {
        let base = std::env::temp_dir().join(format!(
            "ag-daemon-reg-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = base.join("repo");
        let root = if nested {
            repo.join("packages/app")
        } else {
            repo.clone()
        };
        fs::create_dir_all(root.join("src/a")).unwrap();
        fs::create_dir_all(root.join("src/b")).unwrap();
        fs::write(
            root.join("package.json"),
            "{\"name\":\"daemon-regression\",\"type\":\"module\"}\n",
        )
        .unwrap();
        fs::write(root.join("layout.yaml"), "version: 1\n").unwrap();
        fs::write(
            root.join("src/a/a.ts"),
            "import '../b/b'; export const a = 1;\n",
        )
        .unwrap();
        fs::write(root.join("src/b/b.ts"), "export const b = 1;\n").unwrap();
        let f = Self {
            base,
            repo,
            root,
            child: None,
        };
        f.git(&["init", "-q"]);
        f.git(&["add", "."]);
        f.commit("2001-01-01T00:00:00+0000");
        f
    }
    fn git(&self, args: &[&str]) -> Output {
        let output = Command::new("git")
            .current_dir(&self.repo)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn commit(&self, date: &str) {
        let output = Command::new("git")
            .current_dir(&self.repo)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .current_dir(&self.root)
            .env_remove("SOURCE_DATE_EPOCH")
            .args(args)
            .output()
            .unwrap()
    }
    fn scan(&self, standalone: bool, output: Option<&str>) -> Output {
        let mut args = vec![
            "scan",
            ".",
            "--engine",
            "native",
            "--language",
            "typescript",
            "--rules",
            "layout.yaml",
        ];
        if standalone {
            args.push("--no-daemon");
        }
        if let Some(output) = output {
            args.extend(["-o", output]);
        }
        self.command(&args)
    }
    fn spawn(&mut self) {
        fs::create_dir_all(self.root.join(".archgraph")).unwrap();
        let log = fs::File::create(self.root.join(".archgraph/regression.log")).unwrap();
        self.child = Some(
            Command::new(env!("CARGO_BIN_EXE_archgraph"))
                .current_dir(&self.root)
                .env_remove("SOURCE_DATE_EPOCH")
                .args([
                    "serve",
                    ".",
                    "--engine",
                    "native",
                    "--language",
                    "typescript",
                    "--rules",
                    "layout.yaml",
                    "--port",
                ])
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
    }
    fn log(&self) -> String {
        fs::read_to_string(self.root.join(".archgraph/regression.log")).unwrap_or_default()
    }
    fn ready(&mut self) {
        let start = Instant::now();
        while !self.root.join(".archgraph/daemon.json").exists() {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "serve failed: {}",
                self.log()
            );
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "startup timeout: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn parity(&self) -> Value {
        let warm = self.scan(false, None);
        let cold = self.scan(true, None);
        assert!(
            warm.status.success(),
            "warm: {}",
            String::from_utf8_lossy(&warm.stderr)
        );
        assert!(
            cold.status.success(),
            "cold: {}",
            String::from_utf8_lossy(&cold.stderr)
        );
        assert_eq!(warm.stdout, cold.stdout, "warm and cold differ");
        serde_json::from_slice(&warm.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn warm_output_exclusion_preserves_clean_git_and_exact_file_bytes() {
    let mut f = Fixture::new(false);
    fs::write(f.root.join("result.json"), "existing untracked output\n").unwrap();
    f.spawn();
    f.ready();
    let warm = f.scan(false, Some("result.json"));
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );
    let bytes = fs::read(f.root.join("result.json")).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["source"]["git"]["dirty"], false);
    let cold = f.scan(true, Some("result.json"));
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    assert_eq!(bytes, fs::read(f.root.join("result.json")).unwrap());
}

#[test]
fn nested_git_root_empty_commit_refreshes_source_revision_and_timestamp() {
    let mut f = Fixture::new(true);
    f.spawn();
    f.ready();
    let before = f.parity();
    f.commit("2001-02-03T04:05:06+0000");
    let after = f.parity();
    assert_ne!(
        before["source"]["git"]["commit"],
        after["source"]["git"]["commit"]
    );
    assert_eq!(after["source"]["generated_at"], "2001-02-03T04:05:06Z");
}

#[test]
fn linked_worktree_empty_commit_refreshes_source_revision_and_timestamp() {
    let mut f = Fixture::new(true);
    let linked = f.base.join("linked");
    f.git(&[
        "worktree",
        "add",
        "--detach",
        "--quiet",
        linked.to_str().unwrap(),
        "HEAD",
    ]);
    f.repo = linked.clone();
    f.root = linked.join("packages/app");
    f.spawn();
    f.ready();
    let before = f.parity();
    f.commit("2002-03-04T05:06:07+0000");
    let after = f.parity();
    assert_ne!(
        before["source"]["git"]["commit"],
        after["source"]["git"]["commit"]
    );
    assert_eq!(after["source"]["generated_at"], "2002-03-04T05:06:07Z");
}

#[test]
fn malformed_nested_manifest_repeatedly_fails_until_source_is_fixed() {
    let mut f = Fixture::new(false);
    fs::write(f.root.join("src/a/package.json"), "{}\n").unwrap();
    f.git(&["add", "."]);
    f.commit("2001-01-02T00:00:00+0000");
    f.spawn();
    f.ready();
    fs::write(f.root.join("src/a/package.json"), [0xff, 0xfe]).unwrap();
    for _ in 0..3 {
        let output = f.scan(false, None);
        assert_eq!(
            output.status.code(),
            Some(2),
            "accepted malformed source: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
    }
    fs::write(f.root.join("src/a/package.json"), "{}\n").unwrap();
    f.parity();
}

#[test]
fn edit_after_collection_warning_during_startup_is_in_first_ready_snapshot() {
    let mut f = Fixture::new(false);
    fs::create_dir_all(f.root.join("src/padding")).unwrap();
    for index in 0..3500 {
        fs::write(
            f.root.join(format!("src/padding/file{index}.ts")),
            format!("export const item{index} = {index};\n"),
        )
        .unwrap();
    }
    // A definitely unresolved reference ensures the first collection emits a warning.
    fs::write(
        f.root.join("src/a/a.ts"),
        "import '../b/b'; import 'missing-regression-package'; export const a = 1;\n",
    )
    .unwrap();
    f.git(&["add", "."]);
    f.commit("2001-01-02T00:00:00+0000");
    f.spawn();
    let start = Instant::now();
    while !f.log().contains("warning:") {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "missing collection warning: {}",
            f.log()
        );
        assert!(
            f.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "serve exited: {}",
            f.log()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !f.root.join(".archgraph/daemon.json").exists(),
        "fixture did not expose the startup window"
    );
    fs::write(
        f.root.join("src/b/b.ts"),
        "import '../a/a'; export const b = 1;\n",
    )
    .unwrap();
    f.ready();
    let graph = f.parity();
    assert!(
        graph["cycles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cycle| cycle["members"].as_array().unwrap().len() == 2)
    );
}

#[test]
fn nested_sibling_edits_preserve_dirty_parity_without_touching_git_index() {
    let mut f = Fixture::new(true);
    let index = f.repo.join(".git/index");
    let original_bytes = fs::read(&index).unwrap();
    let original_mtime = fs::metadata(&index).unwrap().modified().unwrap();
    let unchanged_index = || {
        assert_eq!(
            fs::read(&index).unwrap(),
            original_bytes,
            "daemon or command rewrote the real Git index"
        );
        assert_eq!(
            fs::metadata(&index).unwrap().modified().unwrap(),
            original_mtime,
            "daemon or command refreshed the real Git index timestamp"
        );
    };
    f.spawn();
    f.ready();
    unchanged_index();
    assert_eq!(f.parity()["source"]["git"]["dirty"], false);
    unchanged_index();
    let sibling = f.repo.join("packages/sibling/untracked.ts");
    fs::create_dir_all(sibling.parent().unwrap()).unwrap();
    fs::write(&sibling, "export const unrelated = 1;\n").unwrap();
    unchanged_index();
    assert_eq!(f.parity()["source"]["git"]["dirty"], true);
    unchanged_index();
    fs::remove_file(sibling).unwrap();
    unchanged_index();
    assert_eq!(f.parity()["source"]["git"]["dirty"], false);
    unchanged_index();
}
