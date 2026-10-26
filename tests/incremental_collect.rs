use std::fs;
use std::path::{Path, PathBuf};

use boundrail::collect::{excludes, incremental, native};
use boundrail::model::Language;

struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let mut fixture = Self(std::env::temp_dir().join(format!(
            "boundrail-incremental-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        for (name, source) in files {
            fixture.write(name, source);
        }
        fixture.0 = fixture.0.canonicalize().unwrap();
        fixture
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn write(&self, name: &str, source: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    fn policy(&self, language: Language) -> excludes::Policy {
        excludes::Policy::new(self.path(), language, excludes::Options::default()).unwrap()
    }
    fn session(&self, language: Language) -> incremental::Session {
        incremental::Session::new(incremental::Options {
            root: self.0.clone(),
            language,
            source_dir: ".".into(),
            package: None,
            python_paths: Vec::new(),
            go: "go".into(),
            scan_all: false,
        })
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn comparable(output: native::Output) -> serde_json::Value {
    serde_json::json!({
        "files": output.raw.files, "edges":output.raw.edges, "imports":output.raw.imports,
        "metadata":output.raw.metadata.into_iter().map(|(file, meta)| (file, (meta.barrel, meta.test))).collect::<std::collections::BTreeMap<_,_>>(),"collector":output.raw.collector,
        "symbols":output.symbols,"source_root":output.source_root
    })
}

#[test]
fn typescript_reuses_syntax_and_updates_importers_after_barrel_change() {
    let fixture = Fixture::new(&[
        (
            "a.ts",
            "import { thing } from './index'; export const result = thing;",
        ),
        ("index.ts", "export { thing } from './b';"),
        ("b.ts", "export const thing = 1;"),
        ("c.ts", "export const thing = 2;"),
    ]);
    let policy = fixture.policy(Language::Typescript);
    let mut session = fixture.session(Language::Typescript);
    let first = session.collect(&policy).unwrap();
    assert_eq!(first.parsed_files, 4);
    assert_eq!(first.cached_files, 0);
    fixture.write("index.ts", "export { thing } from './c';");
    let update = session.update(&["index.ts".into()], &policy).unwrap();
    assert_eq!(update.parsed_files, 1);
    assert_eq!(update.cached_files, 3);
    assert!(update.fallback_reason.is_none());
    assert!(
        update
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.from == "a.ts" && edge.to == "c.ts")
    );
    assert_eq!(
        comparable(update.output),
        comparable(native::typescript::collect(fixture.path(), ".", &policy, false).unwrap())
    );
    let unchanged = session.collect(&policy).unwrap();
    assert_eq!(unchanged.parsed_files, 0);
    assert_eq!(unchanged.cached_files, 4);
}

#[test]
fn typescript_inventory_and_config_changes_reset_resolution_metadata() {
    let fixture = Fixture::new(&[
        (
            "a.ts",
            "import { thing } from '@thing'; export const result = thing;",
        ),
        ("b.ts", "export const thing = 1;"),
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@thing":["b.ts"]}}}"#,
        ),
    ]);
    let policy = fixture.policy(Language::Typescript);
    let mut session = fixture.session(Language::Typescript);
    session.collect(&policy).unwrap();
    fixture.write("c.ts", "export const thing = 2;");
    let added = session.update(&["c.ts".into()], &policy).unwrap();
    assert_eq!(added.parsed_files, 3);
    assert!(added.fallback_reason.is_some());
    fixture.write(
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@thing":["c.ts"]}}}"#,
    );
    let changed = session.update(&["tsconfig.json".into()], &policy).unwrap();
    assert_eq!(changed.parsed_files, 3);
    assert!(changed.fallback_reason.is_some());
    assert!(
        changed
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.from == "a.ts" && edge.to == "c.ts")
    );
    assert_eq!(
        comparable(changed.output),
        comparable(native::typescript::collect(fixture.path(), ".", &policy, false).unwrap())
    );
    fs::remove_file(fixture.path().join("c.ts")).unwrap();
    let removed = session.update(&["c.ts".into()], &policy).unwrap();
    assert!(removed.fallback_reason.is_some());
    assert_eq!(
        comparable(removed.output),
        comparable(native::typescript::collect(fixture.path(), ".", &policy, false).unwrap())
    );
}

#[test]
fn python_reuses_syntax_and_recomputes_declaration_and_barrel_consumers() {
    let fixture = Fixture::new(&[
        ("pkg/__init__.py", "from .b import Thing\n"),
        ("pkg/a.py", "from pkg import Thing\n"),
        ("pkg/b.py", "class Thing: pass\n"),
        ("pkg/c.py", "class Thing: pass\n"),
    ]);
    let policy = fixture.policy(Language::Python);
    let mut session = fixture.session(Language::Python);
    let first = session.collect(&policy).unwrap();
    assert_eq!(first.parsed_files, 4);
    fixture.write("pkg/__init__.py", "from .c import Thing\n");
    let update = session
        .update(&[fixture.path().join("pkg/__init__.py")], &policy)
        .unwrap();
    assert_eq!(update.parsed_files, 1);
    assert_eq!(update.cached_files, 3);
    assert!(update.fallback_reason.is_none());
    assert!(
        update
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.from == "pkg/a.py" && edge.to == "pkg/c.py")
    );
    assert_eq!(
        comparable(update.output),
        comparable(native::python::collect(fixture.path(), ".", None, &[], &policy).unwrap())
    );
    let reset = session.update(&[PathBuf::from(".")], &policy).unwrap();
    assert_eq!(reset.parsed_files, 4);
    assert!(reset.fallback_reason.is_some());
}

#[test]
fn rejects_dirty_paths_outside_root() {
    let fixture = Fixture::new(&[("a.ts", "export const a = 1;")]);
    let policy = fixture.policy(Language::Typescript);
    let mut session = fixture.session(Language::Typescript);
    session.collect(&policy).unwrap();
    assert!(session.update(&["../escape.ts".into()], &policy).is_err());
    assert!(
        session
            .update(&[std::env::temp_dir().join("escape.ts")], &policy)
            .is_err()
    );
}

#[test]
#[cfg(unix)]
fn go_reuses_metadata_but_refreshes_for_a_new_import() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new(&[
        ("go.mod", "module example.test/app\n\ngo 1.21\n"),
        ("main.go", "package main\nfunc main() { helper() }\n"),
        ("helper.go", "package main\nfunc helper() {}\n"),
    ]);
    let executable = fixture.path().join("fake-go");
    fs::write(
        &executable,
        "#!/bin/sh\nset -eu\nprintf 'x' >> \"$0.calls\"\ncat \"$0.json\"\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let packages = serde_json::json!({"Dir":fixture.path(),"ImportPath":"example.test/app","Name":"main","GoFiles":["main.go","helper.go"]});
    fs::write(executable.with_extension("json"), packages.to_string()).unwrap();
    let policy = fixture.policy(Language::Go);
    let mut session = incremental::Session::new(incremental::Options {
        root: fixture.0.clone(),
        language: Language::Go,
        source_dir: ".".into(),
        package: None,
        python_paths: Vec::new(),
        go: executable.to_string_lossy().into_owned(),
        scan_all: false,
    })
    .unwrap();
    assert_eq!(session.collect(&policy).unwrap().parsed_files, 2);
    fixture.write("helper.go", "package main\nfunc helper() { main() }\n");
    let update = session.update(&["helper.go".into()], &policy).unwrap();
    assert_eq!(update.parsed_files, 1);
    assert_eq!(update.cached_files, 1);
    assert_eq!(
        fs::read_to_string(fixture.path().join("fake-go.calls")).unwrap(),
        "x"
    );
    assert_eq!(
        comparable(update.output),
        comparable(
            native::go::collect(fixture.path(), executable.to_str().unwrap(), &policy).unwrap()
        )
    );
    fixture.write(
        "main.go",
        "package main\nimport \"fmt\"\nfunc main() { fmt.Println(1) }\n",
    );
    let packages = format!(
        "{packages}\n{}",
        serde_json::json!({"Dir":"/known/stdlib/fmt","ImportPath":"fmt","Name":"fmt"})
    );
    fs::write(executable.with_extension("json"), packages).unwrap();
    let update = session.update(&["main.go".into()], &policy).unwrap();
    assert_eq!(update.parsed_files, 1);
    let records = update.output.raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|import| import.external.as_deref() == Some("fmt")),
        "{records:?}"
    );
    // Initial + explicit standalone comparison + new import metadata refresh.
    assert_eq!(
        fs::read_to_string(fixture.path().join("fake-go.calls")).unwrap(),
        "xxx"
    );
}

#[test]
#[cfg(unix)]
fn typescript_replacing_a_source_with_a_symlink_resets_cached_targets() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(&[
        (
            "a.ts",
            "import { value } from './b'; export const result = value;",
        ),
        ("b.ts", "export const value = 1;"),
        ("c.ts", "export const value = 2;"),
    ]);
    let policy = fixture.policy(Language::Typescript);
    let mut session = fixture.session(Language::Typescript);
    session.collect(&policy).unwrap();
    fs::remove_file(fixture.path().join("b.ts")).unwrap();
    symlink("c.ts", fixture.path().join("b.ts")).unwrap();
    let changed = session.update(&["b.ts".into()], &policy).unwrap();
    assert!(changed.fallback_reason.is_some());
    assert!(
        changed
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.from == "a.ts" && edge.to == "c.ts")
    );
    assert_eq!(
        comparable(changed.output),
        comparable(native::typescript::collect(fixture.path(), ".", &policy, false).unwrap())
    );
}

#[test]
fn typescript_session_reselects_when_new_policy_changes_options_or_ignore_files() {
    let fixture = Fixture::new(&[
        ("src/a.ts", "export const a = 1;"),
        ("src/b.ts", "export const b = 1;"),
        ("src/a.test.ts", "export const test = 1;"),
    ]);
    let mut session = fixture.session(Language::Typescript);
    let policy = fixture.policy(Language::Typescript);
    let initial = session.collect(&policy).unwrap();
    assert!(
        !initial
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.from.ends_with(".test.ts"))
    );
    let included = excludes::Policy::new(
        fixture.path(),
        Language::Typescript,
        excludes::Options {
            include_tests: true,
            ..Default::default()
        },
    )
    .unwrap();
    let update = session.collect(&included).unwrap();
    assert!(
        update
            .output
            .symbols
            .exports
            .iter()
            .any(|export| export.file == "src/a.test.ts")
    );
    assert_eq!(
        comparable(update.output),
        comparable(native::typescript::collect(fixture.path(), ".", &included, false).unwrap())
    );
    fixture.write("src/.gitignore", "b.ts\n");
    let changed = fixture.policy(Language::Typescript);
    let update = session.collect(&changed).unwrap();
    assert_eq!(
        comparable(update.output),
        comparable(native::typescript::collect(fixture.path(), ".", &changed, false).unwrap())
    );
    fixture.write(".boundrailignore", "src/a.ts\n");
    let changed = fixture.policy(Language::Typescript);
    let update = session.collect(&changed).unwrap();
    assert_eq!(
        comparable(update.output),
        comparable(native::typescript::collect(fixture.path(), ".", &changed, false).unwrap())
    );
}

#[test]
fn typescript_session_exposes_enclosing_custom_config_and_missing_extends() {
    let fixture = Fixture::new(&[
        (".git/marker", ""),
        (
            "package.json",
            r#"{"private":true,"workspaces":["packages/*"]}"#,
        ),
        (
            "config/base.json",
            r#"{"compilerOptions":{"baseUrl":"../packages/app","paths":{"@value":["b.ts"]}}}"#,
        ),
        ("packages/app/package.json", r#"{"name":"app"}"#),
        (
            "packages/app/tsconfig.json",
            r#"{"extends":"../../config/base.json","include":["**/*.ts"]}"#,
        ),
        (
            "packages/app/a.ts",
            "import { value } from '@value'; export const a = value;",
        ),
        ("packages/app/b.ts", "export const value = 1;"),
        ("packages/app/c.ts", "export const value = 2;"),
    ]);
    let root = fixture.path().join("packages/app");
    let policy =
        excludes::Policy::new(&root, Language::Typescript, excludes::Options::default()).unwrap();
    let mut session = incremental::Session::new(incremental::Options {
        root: root.clone(),
        language: Language::Typescript,
        source_dir: ".".into(),
        package: None,
        python_paths: Vec::new(),
        go: "go".into(),
        scan_all: false,
    })
    .unwrap();
    let initial = session.collect(&policy).unwrap();
    assert!(
        initial
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.to == "b.ts")
    );
    let metadata = session.metadata_files();
    assert!(
        metadata.contains(&fixture.path().join("config/base.json")),
        "{metadata:?}"
    );
    assert!(metadata.contains(&fixture.path().join("package.json")));
    assert!(metadata.contains(&fixture.path().join("pnpm-workspace.yaml")));
    fixture.write(
        "config/base.json",
        r#"{"compilerOptions":{"baseUrl":"../packages/app","paths":{"@value":["c.ts"]}}}"#,
    );
    // Enclosing metadata changes use the documented full reset signal after watcher notification.
    let updated = session
        .update(std::slice::from_ref(&root), &policy)
        .unwrap();
    assert!(updated.fallback_reason.is_some());
    assert!(
        updated
            .output
            .raw
            .edges
            .iter()
            .any(|edge| edge.to == "c.ts")
    );
    assert_eq!(
        comparable(updated.output),
        comparable(native::typescript::collect(&root, ".", &policy, false).unwrap())
    );
    fixture.write(
        "packages/app/tsconfig.json",
        r#"{"extends":"../../config/not-yet-created.json"}"#,
    );
    session.update(&["tsconfig.json".into()], &policy).unwrap();
    let metadata = session.metadata_files();
    assert!(
        metadata.contains(&fixture.path().join("config/not-yet-created.json")),
        "{metadata:?}"
    );
    assert!(
        !metadata.contains(&fixture.path().join("config/base.json")),
        "old configuration footprint must reset"
    );
}

#[test]
fn python_session_refreshes_wheel_metadata_and_exposes_environment_inventory() {
    let fixture = Fixture::new(&[
        ("pkg/__init__.py", ""),
        ("pkg/a.py", "import installed\n"),
        (
            ".venv/lib/python3.14/site-packages/distribution-1.dist-info/RECORD",
            "",
        ),
        (
            ".venv/lib/python3.14/site-packages/installed/__init__.py",
            "",
        ),
    ]);
    let policy = fixture.policy(Language::Python);
    let mut session = fixture.session(Language::Python);
    let first = session.collect(&policy).unwrap();
    assert!(
        first
            .output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|record| record.specifier == "installed" && record.unresolved.is_some())
    );
    let record = ".venv/lib/python3.14/site-packages/distribution-1.dist-info/RECORD";
    let metadata = session.metadata_files();
    for path in [
        ".venv",
        ".venv/lib",
        ".venv/lib/python3.14/site-packages",
        record,
        "venv/lib",
    ] {
        assert!(
            metadata.contains(&fixture.path().join(path)),
            "{path}: {metadata:?}"
        );
    }
    fixture.write(record, "installed/__init__.py,,\n");
    let update = session.update(&[record.into()], &policy).unwrap();
    assert!(update.fallback_reason.is_some());
    assert!(
        update
            .output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|record| record.external.as_deref() == Some("installed"))
    );
    fs::remove_file(
        fixture
            .path()
            .join(".venv/lib/python3.14/site-packages/installed/__init__.py"),
    )
    .unwrap();
    let update = session
        .update(
            &[".venv/lib/python3.14/site-packages/installed/__init__.py".into()],
            &policy,
        )
        .unwrap();
    assert!(
        update
            .output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|record| record.specifier == "installed" && record.unresolved.is_some())
    );
}
