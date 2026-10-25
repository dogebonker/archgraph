use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use boundrail::project_skill::{DRAFT, INSTALLED, PROPOSAL};

const LAYOUT: &str = "version: 1\nmodules:\n  - {id: domain, path: 'src/domain/**'}\n  - {id: adapters, path: 'src/adapters/**'}\n  - {id: shared, path: 'src/shared/**'}\nforbid:\n  - {id: no-adapters, from: 'src/domain/**', to: 'src/adapters/**', reason: 'Keep policy independent.'}\nallow:\n  - {from: 'src/domain/**', to: 'src/shared/**'}\n";

struct Fixture(PathBuf);
impl Fixture {
    fn new(policy: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let fixture = Self(std::env::temp_dir().join(format!(
            "archgraph-project-skill-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        fixture.write(
            "package.json",
            "{\"name\":\"fixture\",\"type\":\"module\"}\n",
        );
        fixture.write(
            "src/domain/a.ts",
            "import {shared} from '../shared/s'; export const a = shared;\n",
        );
        fixture.write("src/shared/s.ts", "export const shared = 1;\n");
        fixture.write("src/adapters/b.ts", "export const b = 2;\n");
        fixture.write("AGENTS.md", "Human instructions stay here.\n");
        if policy {
            fixture.write("layout.yaml", LAYOUT);
        }
        fixture
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.0.join(path)).unwrap()
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .arg("--no-daemon")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> Output {
        let result = self.run(args);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        result
    }
    fn commands(&self) -> Vec<Vec<String>> {
        self.read(DRAFT)
            .split("```sh\n")
            .skip(1)
            .map(|part| {
                part.split("```\n")
                    .next()
                    .unwrap()
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .collect()
    }
    #[cfg(unix)]
    fn shell(&self, command: &str) -> Output {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_boundrail"));
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(binary.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&inherited_path));
        Command::new("sh")
            .args(["-c", command])
            .env("PATH", std::env::join_paths(paths).unwrap())
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
fn draft_is_inactive_deterministic_and_contains_source_backed_workflow() {
    let fixture = Fixture::new(true);
    let source = fixture.read("src/domain/a.ts");
    let agents = fixture.read("AGENTS.md");
    fixture.ok(&["skill", "draft"]);
    let first = fixture.read(DRAFT);
    fixture.ok(&["skill", "draft"]);
    assert_eq!(first, fixture.read(DRAFT));
    assert_eq!(fixture.read("layout.yaml"), LAYOUT);
    assert_eq!(fixture.read("AGENTS.md"), agents);
    assert_eq!(fixture.read("src/domain/a.ts"), source);
    for host in [
        "CLAUDE.md",
        ".cursorrules",
        ".github/copilot-instructions.md",
        INSTALLED,
        PROPOSAL,
    ] {
        assert!(
            !fixture.0.join(host).exists(),
            "unexpected active write {host}"
        );
    }
    assert!(first.starts_with("---\nname: project-architecture\n"));
    assert!(first.contains("src/domain/a.ts"));
    assert!(first.contains("src/shared/s.ts"));
    assert!(first.contains("draft-time source evidence"));
    assert!(first.contains("Unverified constraints"));
    assert!(first.contains("No service boundaries were inferred"));
    let commands = fixture.commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].len(), 2);
    assert_eq!(commands[1].len(), 4);
    assert!(!commands[1][3].contains("--fail-on"));
}

#[test]
fn absent_policy_creates_only_a_proposal_and_requires_review_then_redraft() {
    let fixture = Fixture::new(false);
    fixture.ok(&["skill", "draft"]);
    assert!(!fixture.0.join("layout.yaml").exists());
    let proposed = fixture.read(PROPOSAL);
    assert!(proposed.contains("version: 1"));
    assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
    fixture.write(PROPOSAL, &(proposed + "# Human proposal note\n"));
    let preserved = fixture.read(PROPOSAL);
    let repeated = fixture.ok(&["skill", "draft"]);
    let result: serde_json::Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(result["proposal_preserved"], true);
    assert_eq!(fixture.read(PROPOSAL), preserved);
    fixture.write("layout.yaml", LAYOUT);
    assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
    fixture.ok(&["skill", "draft"]);
    fixture.ok(&["skill", "adopt"]);
    assert_eq!(fixture.read(INSTALLED), fixture.read(DRAFT));
    assert_eq!(fixture.read(PROPOSAL), preserved);
    assert_eq!(fixture.read("AGENTS.md"), "Human instructions stay here.\n");
}

#[test]
#[cfg(unix)]
fn adopted_reviewed_workflow_detects_a_planted_import_without_touching_source() {
    let fixture = Fixture::new(true);
    fixture.ok(&["skill", "draft"]);
    let reviewed = fixture.read(DRAFT) + "\nHuman reviewed note.\n";
    fixture.write(DRAFT, &reviewed);
    fixture.ok(&["skill", "adopt"]);
    assert_eq!(fixture.read(INSTALLED), reviewed);
    let commands = fixture.commands();
    for command in &commands[0] {
        let result = fixture.shell(command);
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let planted =
        "import '../adapters/b'; import {shared} from '../shared/s'; export const a = shared;\n";
    fixture.write("src/domain/a.ts", planted);
    let after = fixture.shell(&commands[1][0]);
    assert!(
        after.status.success(),
        "{}",
        String::from_utf8_lossy(&after.stderr)
    );
    let checked = fixture.shell(&commands[1][1]);
    assert_eq!(
        checked.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stdout).contains("no-adapters"));
    assert_eq!(fixture.read("src/domain/a.ts"), planted);
}

#[test]
fn regeneration_and_adoption_preserve_foreign_frontmatter_and_notes() {
    let fixture = Fixture::new(true);
    fixture.ok(&["skill", "draft"]);
    let draft = fixture.read(DRAFT).replace(
        "description: Use before changing",
        "description: Human wording. Use before changing",
    ) + "\nDraft human notes.\n";
    fixture.write(DRAFT, &draft);
    fixture.ok(&["skill", "adopt"]);
    fixture.write(
        INSTALLED,
        &(fixture.read(INSTALLED) + "\nInstalled notes.\n"),
    );
    fixture.write("src/domain/new.ts", "export const fresh = 1;\n");
    let source = fixture.read("src/domain/new.ts");
    fixture.ok(&["skill", "draft"]);
    assert!(fixture.read(DRAFT).contains("description: Human wording."));
    assert!(fixture.read(DRAFT).ends_with("\nDraft human notes.\n"));
    assert_ne!(fixture.read(DRAFT), draft);
    fixture.ok(&["skill", "adopt"]);
    let installed = fixture.read(INSTALLED);
    assert!(installed.contains("description: Human wording."));
    assert!(installed.ends_with("\nInstalled notes.\n"));
    assert!(installed.contains("src/domain/new.ts"));
    assert_eq!(fixture.read("src/domain/new.ts"), source);
}

#[test]
fn stale_policy_and_manifest_cannot_be_adopted() {
    let fixture = Fixture::new(true);
    fixture.write("services.yaml", "version: 1\nservices: []\nedges: []\n");
    fixture.ok(&["skill", "draft"]);
    let reviewed = fixture.read(DRAFT);
    fixture.write(
        "layout.yaml",
        &(LAYOUT.to_owned() + "# policy bytes changed\n"),
    );
    assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
    assert!(!fixture.0.join(INSTALLED).exists());
    assert_eq!(fixture.read(DRAFT), reviewed);
    fixture.ok(&["skill", "draft"]);
    fixture.write(
        "services.yaml",
        "version: 1\nservices: []\nedges: []\n# changed\n",
    );
    assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
    assert!(!fixture.0.join(INSTALLED).exists());
    let absent = Fixture::new(true);
    absent.ok(&["skill", "draft"]);
    absent.write("services.yaml", "version: 1\nservices: []\nedges: []\n");
    assert_eq!(absent.run(&["skill", "adopt"]).status.code(), Some(2));
}

#[test]
fn conflicting_or_unowned_files_are_preserved() {
    let fixture = Fixture::new(true);
    fixture.ok(&["skill", "draft"]);
    fixture.ok(&["skill", "adopt"]);
    let initial = fixture.read(DRAFT);
    for changed in [
        initial.replace("Read current policy", "Hand-edited owned policy"),
        initial.clone() + "<!-- archgraph:start -->\n<!-- archgraph:end -->\n",
        initial.replace("<!-- archgraph:end -->", ""),
        "---\nname: project-architecture\ndescription: Hand-written\n---\nMy own skill.\n".into(),
        String::new(),
    ] {
        fixture.write(DRAFT, &changed);
        assert_eq!(fixture.run(&["skill", "draft"]).status.code(), Some(2));
        assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
        assert_eq!(fixture.read(DRAFT), changed);
    }
    fixture.write(DRAFT, &initial);
    for changed in [
        fixture
            .read(INSTALLED)
            .replace("Read current policy", "Hand-edited installed policy"),
        "---\nname: project-architecture\ndescription: Hand-written\n---\nUnowned content.\n"
            .into(),
        String::new(),
    ] {
        fixture.write(INSTALLED, &changed);
        assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
        assert_eq!(fixture.read(INSTALLED), changed);
    }
}

#[test]
#[cfg(unix)]
fn service_workflow_gates_only_authored_rules_and_quotes_literal_manifest_paths() {
    let fixture = Fixture::new(true);
    let manifest = "pol icy'<!-- marker -->&$(touch wrong).yaml";
    fixture.write(manifest, "version: 1\nservices: [api, worker]\nedges: []\nforbid:\n  - {id: no-pg, from: worker, to: 'pkg:pg'}\n");
    fixture.write("package.json", "{\"dependencies\":{\"pg\":\"*\"}}\n");
    fixture.write("api/Dockerfile", "FROM node\n");
    fixture.write("worker/Dockerfile", "FROM node\n");
    fixture.write("worker/main.ts", "export const worker = true;\n");
    fixture.ok(&["skill", "draft", "--manifest", manifest]);
    let draft = fixture.read(DRAFT);
    assert_eq!(draft.matches("<!-- archgraph:start -->").count(), 1);
    assert_eq!(draft.matches("<!-- archgraph:end -->").count(), 1);
    fixture.ok(&["skill", "adopt"]);
    let commands = fixture.commands();
    assert!(commands[1][3].contains("--fail-on new-forbidden"));
    for command in &commands[0] {
        let result = fixture.shell(command);
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fixture.write("worker/main.ts", "import {Pool} from 'pg'; new Pool();\n");
    assert!(fixture.shell(&commands[1][2]).status.success());
    let checked = fixture.shell(&commands[1][3]);
    assert_eq!(
        checked.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stdout).contains("no-pg"));
    assert!(!fixture.0.join("wrong").exists());
}

#[test]
#[cfg(unix)]
fn manifest_names_starting_with_a_dash_work_in_generated_commands() {
    let fixture = Fixture::new(true);
    fixture.write("-services.yaml", "version: 1\nservices: []\nedges: []\n");
    fixture.ok(&["skill", "draft", "--manifest=-services.yaml"]);
    for command in &fixture.commands()[0] {
        let output = fixture.shell(command);
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn repository_markers_remain_data_and_frontmatter_requires_a_valid_mapping() {
    let fixture = Fixture::new(true);
    fixture.write(
        "layout.yaml",
        &LAYOUT.replace(
            "Keep policy independent.",
            "literal <!-- archgraph:start --> --> <script> & data",
        ),
    );
    fixture.write("src/shared/<marker>&.ts", "export const marker = 1;\n");
    fixture.write("src/domain/a.ts", "import '../adapters/b';\n");
    fixture.ok(&["skill", "draft"]);
    let draft = fixture.read(DRAFT);
    assert_eq!(draft.matches("<!-- archgraph:start -->").count(), 1);
    assert_eq!(draft.matches("<!-- archgraph:end -->").count(), 1);
    assert!(draft.contains("\\u003c"));
    fixture.ok(&["skill", "adopt"]);
    for header in [
        "---\n42\n---\n",
        "---\nname: different\ndescription: manual\n---\n",
        "---\nname: project-architecture\ndescription: [bad]\n---\n",
    ] {
        let changed = header.to_owned() + draft.split_once("---\n\n").unwrap().1;
        fixture.write(DRAFT, &changed);
        assert_eq!(fixture.run(&["skill", "draft"]).status.code(), Some(2));
        assert_eq!(fixture.read(DRAFT), changed);
    }
}

#[cfg(unix)]
#[test]
fn symlink_components_and_escaping_manifests_are_rejected_without_writes() {
    use std::os::unix::fs::symlink;
    for component in [
        ".archgraph",
        ".archgraph/drafts",
        ".archgraph/drafts/project-architecture",
        DRAFT,
        "layout.yaml",
        "services.yaml",
    ] {
        let fixture = Fixture::new(true);
        let outside = Fixture::new(false);
        let link = fixture.0.join(component);
        if link.exists() {
            fs::remove_file(&link).unwrap();
        }
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let target = if component.ends_with(".yaml") || component.ends_with(".md") {
            outside.write("target", "original outside bytes");
            outside.0.join("target")
        } else {
            outside.0.clone()
        };
        symlink(&target, &link).unwrap();
        assert_eq!(
            fixture.run(&["skill", "draft"]).status.code(),
            Some(2),
            "{component}"
        );
        assert!(!fixture.0.join(INSTALLED).exists());
        if target.is_file() {
            assert_eq!(
                fs::read_to_string(target).unwrap(),
                "original outside bytes"
            );
        }
    }
    let fixture = Fixture::new(true);
    fixture.ok(&["skill", "draft"]);
    let outside = Fixture::new(false);
    symlink(&outside.0, fixture.0.join(".agents")).unwrap();
    assert_eq!(fixture.run(&["skill", "adopt"]).status.code(), Some(2));
    assert!(!outside.0.join("skills").exists());
    for manifest in ["../outside.yaml", "/tmp/outside.yaml", "a/../outside.yaml"] {
        assert_eq!(
            fixture
                .run(&["skill", "draft", "--manifest", manifest])
                .status
                .code(),
            Some(2)
        );
    }
}
