mod common;

use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo {
    root: PathBuf,
    language: &'static str,
    files: Vec<&'static str>,
}
impl Repo {
    fn new(language: &'static str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "recipes-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let sources: Vec<_> = match language {
            "typescript" => vec![
                (
                    "app/a.ts",
                    "// café\r\nimport '../domain/b'; // retain comment\r\nexport const a = 1;\r\n",
                ),
                ("domain/b.ts", "import '../app/a';\nexport const b = 2;\n"),
            ],
            "python" => vec![
                (
                    "pkg/app/a.py",
                    "# café\r\nfrom ..domain.b import b as unused_b # retain comment\r\na = 1\r\n",
                ),
                (
                    "pkg/domain/b.py",
                    "from ..app.a import a as unused_a\nb = 2\n",
                ),
            ],
            _ => unreachable!(),
        };
        let repo = Self {
            root,
            language,
            files: sources.iter().map(|(path, _)| *path).collect(),
        };
        for (path, source) in sources {
            let path = repo.root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "Fixture"]);
        repo.git(&["config", "user.email", "fixture@example.invalid"]);
        repo.git(&["add", "."]);
        repo.git(&["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"]);
        repo
    }
    fn git(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .current_dir(&self.root)
            .arg("--no-daemon")
            .args(args)
            .env("SOURCE_DATE_EPOCH", "946684800")
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let output = self.cli(args);
        assert!(
            output.status.success(),
            "{args:?}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn scan(&self) -> Value {
        let mut args = vec![
            "scan",
            ".",
            "--engine",
            "native",
            "--language",
            self.language,
            "--depth",
            "1",
            "--symbols",
        ];
        if self.language == "python" {
            args.extend(["--package", "pkg"]);
        }
        let doc = self.json(&args);
        common::validate(&doc);
        doc
    }
    fn sources(&self) -> Vec<Vec<u8>> {
        self.files
            .iter()
            .map(|path| fs::read(self.root.join(path)).unwrap())
            .collect()
    }
    fn target(&self, command: &str, id: &str, extra: &[&str]) -> Value {
        let mut args = vec![
            command,
            id,
            "--root",
            ".",
            "--graph",
            ".archgraph/test-graph.json",
        ];
        args.extend_from_slice(extra);
        self.json(&args)
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn cycle_id(doc: &Value) -> String {
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "cycle")
        .expect("fixture cycle finding")["finding_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn full(value: &Value) -> &Value {
    if value["schema"] == "archgraph/brief/v1" {
        &value["result"]
    } else {
        value
    }
}
fn round_trip(language: &'static str) {
    let repo = Repo::new(language);
    let before = repo.scan();
    let id = cycle_id(&before);
    assert_eq!(
        id,
        cycle_id(&repo.scan()),
        "identical input retains finding IDs"
    );
    assert!(!before["cycles"].as_array().unwrap().is_empty());
    fs::create_dir(repo.root.join(".archgraph")).unwrap();
    fs::write(
        repo.root.join(".archgraph/test-graph.json"),
        serde_json::to_vec_pretty(&before).unwrap(),
    )
    .unwrap();

    let proposal = repo.target("propose", &id, &[]);
    common::validate(&proposal);
    assert_eq!(proposal["finding_id"], id);
    assert_eq!(proposal["kind"], "recipe");
    assert!(
        proposal["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["kind"] == "cut_import")
    );
    assert!(proposal["verify"]["predicate"].is_string());
    assert!(proposal["verify"]["structured"].is_object());
    assert_eq!(proposal["verify"]["structured"]["baseline_id"], id);
    let brief = repo.target("propose", &id, &["--brief"]);
    assert_eq!(brief["schema"], "archgraph/brief/v1");
    assert_eq!(full(&brief)["finding_id"], id);

    let sources = repo.sources();
    let index = fs::read(repo.root.join(".git/index")).unwrap();
    let dry = repo.target("apply", &id, &["--dry-run"]);
    common::validate(&dry);
    assert_eq!(dry["kind"], "apply-plan");
    assert_eq!(dry["can_apply"], true, "{dry}");
    assert!(dry["pending"].as_array().unwrap().is_empty());
    assert!(!dry["changes"].as_array().unwrap().is_empty());
    assert_eq!(
        repo.sources(),
        sources,
        "dry run leaves all source bytes intact"
    );
    assert_eq!(
        fs::read(repo.root.join(".git/index")).unwrap(),
        index,
        "dry run preserves index bytes"
    );
    assert!(
        !repo
            .root
            .join(format!(".archgraph/applied/{id}.json"))
            .exists()
    );

    let applied = repo.target("apply", &id, &["--yes"]);
    common::validate(&applied);
    assert_eq!(applied["kind"], "apply");
    assert_eq!(applied["applied"], true, "{applied}");
    assert_eq!(applied["verify"]["pass"], true, "{applied}");
    assert_ne!(repo.sources(), sources);
    let after = repo.scan();
    assert!(
        after["cycles"].as_array().unwrap().is_empty(),
        "a new native scan sees the applied cut"
    );
    assert!(
        !after["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["finding_id"] == id)
    );

    let verified = repo.target("verify", &id, &[]);
    common::validate(&verified);
    assert_eq!(verified["kind"], "verification");
    assert_eq!(verified["verify"]["pass"], true, "{verified}");
    let brief = repo.target("verify", &id, &["--brief"]);
    assert_eq!(brief["schema"], "archgraph/brief/v1");
    assert_eq!(full(&brief)["verify"]["pass"], true, "{brief}");
}

#[test]
fn typescript_cli_recipe_dry_run_apply_and_verify() {
    round_trip("typescript");
}

#[test]
fn python_cli_recipe_dry_run_apply_and_verify() {
    round_trip("python");
}
