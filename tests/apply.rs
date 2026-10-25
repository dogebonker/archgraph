use boundrail::{
    apply,
    model::Document,
    recipes::{Confidence, Expected, Predicate, Recipe, Step, StepKind, Verification},
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "apply-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let repo = Self(root);
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "Fixture"]);
        repo.git(&["config", "user.email", "fixture@example.invalid"]);
        repo
    }
    fn write(&self, file: &str, text: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn git(&self, args: &[&str]) -> Vec<u8> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }
    fn commit(&self) {
        self.git(&["add", "."]);
        self.git(&["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"]);
    }
    fn scan(&self) -> Document {
        self.scan_as("typescript", &[])
    }
    fn scan_as(&self, language: &str, extra: &[&str]) -> Document {
        let output = Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .args([
                "--no-daemon",
                "scan",
                self.0.to_str().unwrap(),
                "--engine",
                "native",
                "--language",
                language,
                "--depth",
                "1",
            ])
            .args(extra)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        boundrail::restore_scan(serde_json::from_slice(&output.stdout).unwrap()).unwrap()
    }
    fn cycle() -> Self {
        let r = Self::new();
        r.write(
            "app/move.ts",
            "import { b } from '../domain/b';\nexport const a = () => b;\n",
        );
        r.write(
            "domain/b.ts",
            "import { a } from '../app/move';\nexport const b = () => a;\n",
        );
        r.write(
            "app/main.ts",
            "import { a } from './move'; // keep spaces\nconsole.log(a);\n",
        );
        r.commit();
        r
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn recipe(doc: &mut Document, steps: Vec<Step>) -> String {
    let f = doc.findings.first_mut().expect("fixture cycle finding");
    let id = f.finding_id.clone();
    f.recipe = Some(Recipe {
        steps,
        expected: Expected::default(),
        verify: Verification {
            predicate: "recipe structural and measured deltas match".into(),
            structured: Predicate::default(),
        },
        confidence: Confidence::High,
        risk_notes: vec![],
    });
    id
}
fn move_step() -> Step {
    Step {
        kind: StepKind::MoveFile,
        file: "app/move.ts".into(),
        line: None,
        from: "app/move.ts".into(),
        to: "domain/move.ts".into(),
        detail: json!({}),
        mechanical: true,
    }
}

#[test]
fn moves_rewrite_both_directions_byte_exact_and_preserve_staged_work() {
    let repo = Repo::cycle();
    repo.write("unrelated.txt", "staged\n");
    repo.git(&["add", "unrelated.txt"]);
    repo.write("unrelated.txt", "unstaged\n");
    let staged = repo.git(&["show", ":unrelated.txt"]);
    let mut before = repo.scan();
    let id = recipe(&mut before, vec![move_step()]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    assert!(!repo.0.join("domain/move.ts").exists());
    let report = apply::execute(&repo.0, &before, &plan, || Ok(repo.scan())).unwrap();
    assert!(report.applied, "{:?}", report);
    assert_eq!(
        fs::read_to_string(repo.0.join("domain/move.ts")).unwrap(),
        "import { b } from './b';\nexport const a = () => b;\n"
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("app/main.ts")).unwrap(),
        "import { a } from '../domain/move'; // keep spaces\nconsole.log(a);\n"
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("domain/b.ts")).unwrap(),
        "import { a } from './move';\nexport const b = () => a;\n"
    );
    assert_eq!(repo.git(&["show", ":unrelated.txt"]), staged);
    assert_eq!(
        fs::read_to_string(repo.0.join("unrelated.txt")).unwrap(),
        "unstaged\n"
    );
    assert!(
        apply::verify_receipt(&repo.0, &id, &repo.scan())
            .unwrap()
            .pass
    );
}

#[test]
fn stale_plan_and_semantic_steps_never_write() {
    let repo = Repo::cycle();
    let mut before = repo.scan();
    let id = recipe(&mut before, vec![move_step()]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    repo.write("app/main.ts", "// concurrent edit\n");
    assert!(apply::execute(&repo.0, &before, &plan, || Ok(repo.scan())).is_err());
    assert!(repo.0.join("app/move.ts").exists());
    assert!(!repo.0.join("domain/move.ts").exists());
    let mut step = move_step();
    step.kind = StepKind::ExtractInterface;
    step.mechanical = false;
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(!plan.can_apply);
    assert!(!plan.pending.is_empty());
}

#[test]
fn verification_mismatch_rolls_back_only_owned_changes_and_index() {
    let repo = Repo::cycle();
    let index = repo.git(&["rev-parse", "--git-path", "index"]);
    let index = String::from_utf8(index).unwrap();
    let index = repo.0.join(index.trim());
    let index_before = fs::read(&index).unwrap();
    let mut before = repo.scan();
    let id = recipe(&mut before, vec![move_step()]);
    let mut plan = apply::plan(&repo.0, &before, &id).unwrap();
    plan.recipe.expected.pairs_removed = Some(999);
    plan.recipe.verify.structured.expected = Some(plan.recipe.expected.clone());
    let report = apply::execute(&repo.0, &before, &plan, || {
        repo.write("concurrent.txt", "keep me");
        Ok(repo.scan())
    })
    .unwrap();
    assert!(!report.applied);
    assert!(!report.verify.pass);
    assert!(report.hint.unwrap().contains("expected 999"));
    assert_eq!(fs::read(&index).unwrap(), index_before);
    assert_eq!(
        fs::read_to_string(repo.0.join("app/main.ts")).unwrap(),
        "import { a } from './move'; // keep spaces\nconsole.log(a);\n"
    );
    assert!(repo.0.join("app/move.ts").exists());
    assert!(!repo.0.join("domain/move.ts").exists());
    assert_eq!(
        fs::read_to_string(repo.0.join("concurrent.txt")).unwrap(),
        "keep me"
    );
}

#[test]
fn destinations_collisions_and_traversal_are_refused() {
    let repo = Repo::cycle();
    let mut before = repo.scan();
    let mut step = move_step();
    step.to = "domain/b.ts".into();
    let id = recipe(&mut before, vec![step]);
    assert!(apply::plan(&repo.0, &before, &id).is_err());
    let mut step = move_step();
    step.to = "../outside.ts".into();
    let id = recipe(&mut before, vec![step]);
    assert!(apply::plan(&repo.0, &before, &id).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_destination_parent_is_refused() {
    let repo = Repo::cycle();
    std::os::unix::fs::symlink(repo.0.join("domain"), repo.0.join("linked")).unwrap();
    let mut before = repo.scan();
    let mut step = move_step();
    step.to = "linked/move.ts".into();
    let id = recipe(&mut before, vec![step]);
    assert!(apply::plan(&repo.0, &before, &id).is_err());
}

#[test]
fn used_bindings_block_cut_and_unused_imports_can_be_removed() {
    let repo = Repo::cycle();
    let mut before = repo.scan();
    let edge = before
        .edges
        .iter()
        .find(|e| e.from == "app/move.ts")
        .unwrap()
        .clone();
    let step = Step {
        kind: StepKind::CutImport,
        file: edge.from.clone(),
        line: edge.line,
        from: edge.from.clone(),
        to: edge.to.clone(),
        detail: json!({"operation":"remove_import","specifier":edge.specifier}),
        mechanical: true,
    };
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(!plan.can_apply);
    assert!(plan.pending.iter().any(|p| p.contains("live or ambiguous")));
    repo.write(
        "app/move.ts",
        "import { b } from '../domain/b';\nexport const a = 1;\n",
    );
    let mut before = repo.scan();
    let edge = before
        .edges
        .iter()
        .find(|e| e.from == "app/move.ts")
        .unwrap()
        .clone();
    let step = Step {
        kind: StepKind::CutImport,
        file: edge.from.clone(),
        line: edge.line,
        from: edge.from,
        to: edge.to,
        detail: json!({"operation":"remove_import","specifier":edge.specifier}),
        mechanical: false,
    };
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    let result = apply::execute(&repo.0, &before, &plan, || Ok(repo.scan())).unwrap();
    assert!(result.applied, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.0.join("app/move.ts")).unwrap(),
        "\nexport const a = 1;\n"
    );
}

#[test]
fn proposal_baseline_checks_explicit_expected_deltas_after_manual_edit() {
    let repo = Repo::cycle();
    let mut before = repo.scan();
    let id = recipe(&mut before, vec![move_step()]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    let mut predicate = plan.recipe.verify.structured.clone();
    predicate.baseline_id = Some(id.clone());
    apply::remember_proposal(&repo.0, &before, &id, &plan.recipe).unwrap();
    repo.git(&["mv", "app/move.ts", "domain/move.ts"]);
    repo.write(
        "domain/move.ts",
        "import { b } from './b';\nexport const a = () => b;\n",
    );
    repo.write(
        "domain/b.ts",
        "import { a } from './move';\nexport const b = () => a;\n",
    );
    repo.write(
        "app/main.ts",
        "import { a } from '../domain/move'; // keep spaces\nconsole.log(a);\n",
    );
    assert!(
        apply::verify_predicate(&repo.0, &id, &repo.scan(), &predicate)
            .unwrap()
            .pass
    );
    predicate.expected.as_mut().unwrap().pairs_removed = Some(999);
    assert!(apply::verify_predicate(&repo.0, &id, &repo.scan(), &predicate).is_err());
}

#[test]
fn verification_failure_preserves_concurrent_edits_to_recipe_files() {
    let repo = Repo::cycle();
    let mut before = repo.scan();
    let id = recipe(&mut before, vec![move_step()]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    let error = apply::execute(&repo.0, &before, &plan, || {
        repo.write(
            "app/main.ts",
            "// user changed this while verification ran\n",
        );
        anyhow::bail!("fixture rescan failure")
    })
    .unwrap_err();
    assert!(format!("{error:#}").contains("rollback conflict"));
    assert_eq!(
        fs::read_to_string(repo.0.join("app/main.ts")).unwrap(),
        "// user changed this while verification ran\n"
    );
    assert!(repo.0.join("app/move.ts").exists());
    assert!(!repo.0.join("domain/move.ts").exists());
}

#[test]
fn python_move_keeps_relative_package_imports_and_bindings() {
    let repo = Repo::new();
    repo.write("pkg/app/a.py", "from ..domain.b import b\na = b\n");
    repo.write("pkg/domain/b.py", "from ..app.a import a\nb = a\n");
    repo.write(
        "pkg/app/main.py",
        "from .a import a  # retained\nprint(a)\n",
    );
    repo.commit();
    let mut before = repo.scan_as("python", &["--package", "pkg"]);
    let step = Step {
        kind: StepKind::MoveFile,
        file: "pkg/app/a.py".into(),
        line: None,
        from: "pkg/app/a.py".into(),
        to: "pkg/domain/a.py".into(),
        detail: json!({}),
        mechanical: true,
    };
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    let report = apply::execute(&repo.0, &before, &plan, || {
        Ok(repo.scan_as("python", &["--package", "pkg"]))
    })
    .unwrap();
    assert!(report.applied, "{report:?}");
    assert_eq!(
        fs::read_to_string(repo.0.join("pkg/domain/a.py")).unwrap(),
        "from .b import b\na = b\n"
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("pkg/app/main.py")).unwrap(),
        "from ..domain.a import a  # retained\nprint(a)\n"
    );
}

#[test]
fn add_reexport_and_import_retarget_preserve_existing_source_bytes() {
    let repo = Repo::cycle();
    repo.write("domain/c.ts", "export const c = 3;\n");
    repo.write(
        "domain/index.ts",
        "export { c } from './c'; // keep facade\n",
    );
    repo.commit();
    let mut before = repo.scan();
    let steps = vec![
        Step {
            kind: StepKind::AddReexport,
            file: "domain/index.ts".into(),
            line: None,
            from: "domain/index.ts".into(),
            to: "domain/b.ts".into(),
            detail: json!({"target":"domain/b.ts","symbols":["b"]}),
            mechanical: true,
        },
        Step {
            kind: StepKind::CutImport,
            file: "app/move.ts".into(),
            line: Some(1),
            from: "app/move.ts".into(),
            to: "domain/index.ts".into(),
            detail: json!({"operation":"rewrite_specifier","target":"domain/b.ts","specifier":"../domain/b","old_target":"domain/b.ts"}),
            mechanical: true,
        },
    ];
    let id = recipe(&mut before, steps);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    let result = apply::execute(&repo.0, &before, &plan, || Ok(repo.scan())).unwrap();
    assert!(result.applied, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.0.join("app/move.ts")).unwrap(),
        "import { b } from '../domain/index';\nexport const a = () => b;\n"
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("domain/index.ts")).unwrap(),
        "export { c } from './c'; // keep facade\nexport { b } from './b';\n"
    );
}

#[test]
fn elixir_file_move_preserves_declared_module_identity_without_import_edits() {
    let repo = Repo::new();
    repo.write(
        "app/a.ex",
        "defmodule Demo.A do\n  alias Demo.B\n  def run, do: B.run()\nend\n",
    );
    repo.write(
        "domain/b.ex",
        "defmodule Demo.B do\n  def run, do: Demo.A.run()\nend\n",
    );
    repo.commit();
    let mut before = repo.scan_as("elixir", &[]);
    let step = Step {
        kind: StepKind::MoveFile,
        file: "app/a.ex".into(),
        line: None,
        from: "app/a.ex".into(),
        to: "domain/a.ex".into(),
        detail: json!({}),
        mechanical: true,
    };
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    assert!(
        plan.changes
            .iter()
            .all(|change| change.before == change.after)
    );
    let result =
        apply::execute(&repo.0, &before, &plan, || Ok(repo.scan_as("elixir", &[]))).unwrap();
    assert!(result.applied, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.0.join("domain/a.ex")).unwrap(),
        "defmodule Demo.A do\n  alias Demo.B\n  def run, do: B.run()\nend\n"
    );
}

#[test]
#[ignore = "requires installed Go toolchain (native go list)"]
fn go_same_package_filename_move_keeps_package_imports_exact() {
    let repo = Repo::new();
    repo.write("go.mod", "module example.invalid/fixture\n\ngo 1.21\n");
    repo.write(
        "app/a.go",
        "package app\nimport _ \"example.invalid/fixture/domain\"\nfunc A() {}\n",
    );
    repo.write(
        "domain/b.go",
        "package domain\nimport _ \"example.invalid/fixture/app\"\nfunc B() {}\n",
    );
    repo.commit();
    let mut before = repo.scan_as("go", &[]);
    let old = fs::read(repo.0.join("app/a.go")).unwrap();
    let step = Step {
        kind: StepKind::MoveFile,
        file: "app/a.go".into(),
        line: None,
        from: "app/a.go".into(),
        to: "app/renamed.go".into(),
        detail: json!({}),
        mechanical: true,
    };
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    let result = apply::execute(&repo.0, &before, &plan, || Ok(repo.scan_as("go", &[]))).unwrap();
    assert!(result.applied, "{result:?}");
    assert_eq!(fs::read(repo.0.join("app/renamed.go")).unwrap(), old);
    let mut before = repo.scan_as("go", &[]);
    let edge = before
        .edges
        .iter()
        .find(|edge| edge.from == "app/renamed.go" && edge.to == "domain/b.go")
        .unwrap()
        .clone();
    let step = Step {
        kind: StepKind::CutImport,
        file: edge.from.clone(),
        line: edge.line,
        from: edge.from,
        to: edge.to,
        detail: json!({"operation":"remove_import","specifier":edge.specifier}),
        mechanical: true,
    };
    let id = recipe(&mut before, vec![step]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    let result = apply::execute(&repo.0, &before, &plan, || Ok(repo.scan_as("go", &[]))).unwrap();
    assert!(result.applied, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.0.join("app/renamed.go")).unwrap(),
        "package app\n\nfunc A() {}\n"
    );
}

#[test]
fn moves_preserve_bare_external_imports_and_resolve_internal_imports() {
    let repo = Repo::cycle();
    repo.write("package.json", r#"{"dependencies":{"react":"18.0.0"}}"#);
    repo.write("app/move.ts","import React from 'react';\nimport { b } from '../domain/b';\nexport const a = () => React.createElement('div', null, b);\n");
    repo.commit();
    let mut before = repo.scan();
    assert!(!before.external_nodes.is_empty());
    let id = recipe(&mut before, vec![move_step()]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    assert!(plan.can_apply, "{:?}", plan.pending);
    let result = apply::execute(&repo.0, &before, &plan, || Ok(repo.scan())).unwrap();
    assert!(result.applied, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.0.join("domain/move.ts")).unwrap(),
        "import React from 'react';\nimport { b } from './b';\nexport const a = () => React.createElement('div', null, b);\n"
    );
}

#[test]
fn postscan_lost_external_import_is_rejected_and_rolled_back() {
    let repo = Repo::cycle();
    repo.write("package.json", r#"{"dependencies":{"react":"18.0.0"}}"#);
    repo.write("app/move.ts","import React from 'react';\nimport { b } from '../domain/b';\nexport const a = () => React.createElement('div', null, b);\n");
    repo.commit();
    let original = fs::read(repo.0.join("app/move.ts")).unwrap();
    let mut before = repo.scan();
    let id = recipe(&mut before, vec![move_step()]);
    let plan = apply::plan(&repo.0, &before, &id).unwrap();
    let error = apply::execute(&repo.0, &before, &plan, || {
        let mut after = repo.scan();
        after.external_edges.clear();
        Ok(after)
    })
    .unwrap_err();
    assert!(format!("{error:#}").contains("lost or changed external import"));
    assert_eq!(fs::read(repo.0.join("app/move.ts")).unwrap(), original);
    assert!(!repo.0.join("domain/move.ts").exists());
}
