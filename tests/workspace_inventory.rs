use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use boundrail::package_splits::workspaces::discover;

struct Scratch(PathBuf);
static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "workspace-inventory-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let output = Command::new("git").arg("init").arg(&path).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self(path)
    }
    fn write(&self, file: &str, text: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn npm_and_yarn_workspaces_need_real_manifests() {
    for workspaces in [
        r#"["packages/*"]"#,
        r#"{"packages":["packages/*"],"nohoist":["**/react"]}"#,
    ] {
        let root = Scratch::new();
        root.write(
            "package.json",
            &format!(r#"{{"name":"root","workspaces":{workspaces}}}"#),
        );
        root.write("packages/one/package.json", r#"{"name":"one"}"#);
        root.write("packages/not-a-package/source.ts", "");
        root.write("elsewhere/package.json", r#"{"name":"undeclared"}"#);
        root.write("node_modules/ignored/package.json", "malformed");
        let inventory = discover(&root.0).unwrap();
        assert!(!inventory.checked);
        assert_eq!(inventory.projects.len(), 1);
        assert_eq!(inventory.projects[0].root, "elsewhere");
        assert_eq!(inventory.projects[0].name.as_deref(), Some("undeclared"));
        assert_eq!(
            inventory
                .packages
                .iter()
                .map(|p| p.root.as_str())
                .collect::<Vec<_>>(),
            [".", "packages/one"]
        );
    }
}

#[test]
fn pnpm_negations_and_root_only_metadata() {
    let root = Scratch::new();
    root.write("package.json", r#"{"name":"root"}"#);
    root.write(
        "pnpm-workspace.yaml",
        "packages:\n  - packages/*\n  - '!packages/skip'\n",
    );
    root.write("packages/one/package.json", r#"{"name":"one"}"#);
    root.write("packages/skip/package.json", r#"{"name":"skip"}"#);
    let inventory = discover(&root.0).unwrap();
    assert_eq!(inventory.packages.len(), 2);
    assert_eq!(inventory.projects.len(), 1);
    assert_eq!(inventory.projects[0].root, "packages/skip");
    assert!(!inventory.checked);
    root.write("pnpm-workspace.yaml", "catalog: {}\n");
    let inventory = discover(&root.0).unwrap();
    assert!(!inventory.checked);
    assert_eq!(inventory.projects.len(), 2);
    assert_eq!(inventory.packages.len(), 1);
    assert_eq!(inventory.packages[0].root, ".");
}

#[test]
fn ancestor_workspaces_are_relative_to_actual_scanned_root() {
    let root = Scratch::new();
    root.write(
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    root.write("packages/one/package.json", r#"{"name":"one"}"#);
    root.write("packages/two/package.json", r#"{"name":"two"}"#);
    let inventory = discover(&root.0.join("packages/one")).unwrap();
    assert!(inventory.checked);
    assert_eq!(inventory.packages.len(), 1);
    assert_eq!(inventory.packages[0].root, ".");
    assert_eq!(inventory.packages[0].name.as_deref(), Some("one"));
    assert_eq!(inventory.enclosing_package.as_deref(), Some("one"));
    let inventory = discover(&root.0.join("packages")).unwrap();
    assert_eq!(
        inventory
            .packages
            .iter()
            .map(|p| p.root.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
}

#[test]
fn source_subroot_retains_nearest_enclosing_package_name_only() {
    let root = Scratch::new();
    root.write(
        "package.json",
        r#"{"name":"monorepo","workspaces":["packages/*"]}"#,
    );
    root.write("packages/lib/package.json", r#"{"name":"@repo/lib"}"#);
    root.write("packages/lib/src/index.ts", "");
    let inventory = discover(&root.0.join("packages/lib/src")).unwrap();
    assert!(inventory.checked);
    assert!(inventory.packages.is_empty());
    assert_eq!(inventory.enclosing_package.as_deref(), Some("@repo/lib"));
    let json = serde_json::to_string(&inventory).unwrap();
    assert!(!json.contains(root.0.to_str().unwrap()));
}

#[test]
fn nx_layout_is_not_project_or_package_proof() {
    let root = Scratch::new();
    root.write("nx.json", r#"{"workspaceLayout":{"libsDir":"libs"}}"#);
    root.write("libs/plain/file.ts", "");
    root.write("libs/project/project.json", r#"{"name":"project"}"#);
    root.write("libs/package/package.json", r#"{"name":"package","nx":{}}"#);
    root.write("workspace.json", r#"{"projects":{"legacy":"libs/legacy"}}"#);
    root.write("libs/legacy/file.ts", "");
    let inventory = discover(&root.0).unwrap();
    assert!(inventory.checked);
    assert_eq!(inventory.packages.len(), 1);
    assert_eq!(inventory.packages[0].root, "libs/package");
    assert_eq!(
        inventory
            .projects
            .iter()
            .map(|p| p.root.as_str())
            .collect::<Vec<_>>(),
        ["libs/legacy", "libs/package", "libs/project"]
    );
}

#[test]
fn nx_plugins_leave_completeness_unknown_without_execution() {
    let root = Scratch::new();
    root.write("nx.json", r#"{"plugins":["@nx/vite/plugin"]}"#);
    let inventory = discover(&root.0).unwrap();
    assert!(!inventory.checked);
    assert!(inventory.limitations[0].contains("plugin-inferred"));
}

#[test]
fn malformed_metadata_and_escaping_paths_are_not_silent_proof() {
    for (file, text) in [
        ("package.json", "malformed"),
        ("package.json", r#"{"workspaces":[3]}"#),
        ("package.json", r#"{"workspaces":["../outside/*"]}"#),
        ("pnpm-workspace.yaml", "packages: nope"),
        ("workspace.json", r#"{"projects":{"bad":"../outside"}}"#),
        ("nx.json", r#"{"plugins":false}"#),
    ] {
        let root = Scratch::new();
        root.write(file, text);
        assert!(discover(&root.0).is_err(), "{file}: {text}");
    }
}

#[test]
fn broken_nested_manifests_are_skipped_as_limitations() {
    for (file, text) in [
        ("package.json", "{"),
        ("package.json", "[]"),
        ("package.json", r#"{"name":3}"#),
        ("package.json", r#"{"name":"broken","nx":true}"#),
        ("project.json", "{"),
        ("project.json", r#"{"name":false}"#),
    ] {
        let root = Scratch::new();
        root.write("package.json", r#"{"name":"root"}"#);
        root.write("packages/one/package.json", r#"{"name":"one"}"#);
        root.write(&format!("test/fixtures/broken/{file}"), text);
        let inventory = discover(&root.0).unwrap();
        assert!(!inventory.checked, "{file}: {text}");
        assert_eq!(
            inventory.limitations,
            [format!(
                "Unreadable manifest at test/fixtures/broken/{file} was skipped"
            )]
        );
        assert_eq!(
            inventory
                .packages
                .iter()
                .map(|p| p.root.as_str())
                .collect::<Vec<_>>(),
            [".", "packages/one"]
        );
        assert!(inventory.projects.is_empty());
        // The scan root's own manifests still fail.
        assert!(discover(&root.0.join("test/fixtures/broken")).is_err());
    }
}

#[cfg(unix)]
#[test]
fn symlinked_directories_are_never_followed() {
    use std::os::unix::fs::symlink;
    let root = Scratch::new();
    let outside = Scratch::new();
    outside.write("package.json", r#"{"name":"outside"}"#);
    symlink(&outside.0, root.0.join("linked")).unwrap();
    let inventory = discover(&root.0).unwrap();
    assert!(!inventory.checked);
    assert!(inventory.packages.is_empty());
    fs::create_dir(root.0.join("nested")).unwrap();
    symlink(
        outside.0.join("package.json"),
        root.0.join("nested/package.json"),
    )
    .unwrap();
    let inventory = discover(&root.0).unwrap();
    assert!(
        inventory
            .limitations
            .contains(&"Unreadable manifest at nested/package.json was skipped".to_owned())
    );
    symlink(outside.0.join("package.json"), root.0.join("package.json")).unwrap();
    assert!(discover(&root.0).is_err());
}

#[test]
fn depth_limit_is_explicit() {
    let root = Scratch::new();
    root.write(&format!("{}file.ts", "nested/".repeat(34)), "");
    let inventory = discover(&root.0).unwrap();
    assert!(!inventory.checked);
    assert!(inventory.limitations[0].contains("depth 32"));
}

#[test]
fn relative_dot_paths_are_normalized_and_nested_workspaces_are_reported() {
    let root = Scratch::new();
    root.write("package.json", r#"{"workspaces":["./packages/*"]}"#);
    root.write("packages/one/package.json", r#"{"name":"one"}"#);
    root.write(
        "workspace.json",
        r#"{"projects":{"project":"./libs/project"}}"#,
    );
    root.write("libs/project/project.json", r#"{"name":"project"}"#);
    let inventory = discover(&root.0).unwrap();
    assert!(inventory.checked);
    assert_eq!(inventory.packages[1].root, "packages/one");
    assert_eq!(inventory.projects.len(), 1);
    assert_eq!(inventory.projects[0].root, "libs/project");
    root.write(
        "packages/one/package.json",
        r#"{"name":"one","workspaces":["nested/*"]}"#,
    );
    assert!(!discover(&root.0).unwrap().checked);
}
