use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "archgraph-init-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for directory in ["domain", "adapters", "presentation"] {
            fs::create_dir_all(root.join("src").join(directory)).unwrap();
        }
        for file in [
            "domain/a",
            "domain/b",
            "adapters/c",
            "adapters/d",
            "presentation/p",
        ] {
            fs::write(
                root.join("src").join(format!("{file}.ts")),
                "export const value = 1;\n",
            )
            .unwrap();
        }
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .arg("--no-daemon")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn init_five_files_then_check_live_and_quote_the_rule_reason() {
    let fixture = Fixture::new();
    let started = std::time::Instant::now();
    let output = fixture.run(&["init", "--preset", "layered", "--scope", "internal"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("clean"));
    let checked = fixture.run(&["check"]);
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
    fs::write(
        fixture.0.join("src/domain/a.ts"),
        "import '../adapters/c';\nexport const a = 1;\n",
    )
    .unwrap();
    let checked = fixture.run(&["check"]);
    assert_eq!(checked.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&checked.stdout)
            .contains("reason=\"Domain logic must not depend on adapter implementations.\""),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
}

#[test]
fn minimal_inference_preserves_folders_without_adding_import_bans() {
    let fixture = Fixture::new();
    for folder in ["alpha", "beta"] {
        fs::create_dir_all(fixture.0.join("src").join(folder)).unwrap();
        for index in 0..12 {
            fs::write(
                fixture
                    .0
                    .join("src")
                    .join(folder)
                    .join(format!("f{index}.ts")),
                "export const value = 1;\n",
            )
            .unwrap();
        }
    }
    assert!(
        fixture
            .run(&["init", "--preset", "minimal", "--scope", "demo"])
            .status
            .success()
    );
    let rules = boundrail::rules::load(&fixture.0.join("layout.yaml")).unwrap();
    assert!(rules.doc.forbid.is_empty());
    assert_eq!(rules.doc.layers[0], "**");
    assert!(
        fixture
            .run(&[
                "scan",
                ".",
                "--engine",
                "native",
                "--rules",
                "layout.yaml",
                "-o",
                "graph.json"
            ])
            .status
            .success()
    );
    let graph: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("graph.json")).unwrap()).unwrap();
    assert!(graph["modules"].as_array().unwrap().len() >= 2);
}

#[test]
fn saved_profile_checks_are_incomplete_and_layer_only_edges_keep_reasons() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .run(&["init", "--scope", "internal"])
            .status
            .success()
    );
    assert!(
        fixture
            .run(&[
                "scan",
                ".",
                "--engine",
                "native",
                "--rules",
                "layout.yaml",
                "-o",
                "graph.json"
            ])
            .status
            .success()
    );
    let saved = fixture.run(&["check", "graph.json", "--brief"]);
    assert_eq!(saved.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert_eq!(result["result"]["early_smells"]["verdict"], "INCOMPLETE");
    fs::create_dir_all(fixture.0.join("src/application")).unwrap();
    fs::write(
        fixture.0.join("src/application/work.ts"),
        "export const work = 1;\n",
    )
    .unwrap();
    fs::write(
        fixture.0.join("src/domain/a.ts"),
        "import '../application/work';\n",
    )
    .unwrap();
    let checked = fixture.run(&["check"]);
    assert_eq!(checked.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&checked.stdout)
            .contains("reason=\"Foundational layers must not depend on outer layers.\"")
    );
    assert!(
        fixture
            .run(&[
                "scan",
                ".",
                "--engine",
                "native",
                "--rules",
                "layout.yaml",
                "-o",
                "violation.json"
            ])
            .status
            .success()
    );
    let graph: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("violation.json")).unwrap()).unwrap();
    assert!(graph["findings"].as_array().unwrap().iter().any(|finding| {
        finding["recipe"]["risk_notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap().contains("Foundational layers"))
    }));
}

#[test]
fn inference_marks_conflicts_and_existing_layout_needs_force() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("src/domain/a.ts"),
        "import '../adapters/c';\n",
    )
    .unwrap();
    assert!(
        fixture
            .run(&["init", "--scope", "internal"])
            .status
            .success()
    );
    let path = fixture.0.join("layout.yaml");
    let original = fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&original).contains(
        "# CONFLICT: inferred import src/domain/a.ts -> src/adapters/c.ts, preset says forbidden"
    ));
    assert_eq!(
        fixture.run(&["init", "--scope", "regulated"]).status.code(),
        Some(2)
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(
        fixture
            .run(&["init", "--scope", "regulated", "--force"])
            .status
            .success()
    );
    assert_ne!(fs::read(&path).unwrap(), original);
}

#[test]
fn noninteractive_scope_is_required_and_brief_init_reports_clean() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&["init"]).status.code(), Some(2));
    assert!(!fixture.0.join("layout.yaml").exists());
    let output = fixture.run(&["init", "--preset", "minimal", "--scope", "demo", "--brief"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["result"]["status"], "clean");
}

#[cfg(unix)]
#[test]
fn forcing_a_private_layout_preserves_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let path = fixture.0.join("layout.yaml");
    fs::write(&path, "version: 1\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        fixture
            .run(&["init", "--scope", "internal", "--force"])
            .status
            .success()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
