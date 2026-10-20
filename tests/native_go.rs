use std::fs;
use std::path::{Path, PathBuf};

use archgraph::collect::excludes::{Options, Policy};
use archgraph::collect::native::go;
use archgraph::model::{EdgeKind, Language};

struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let fixture = Self(std::env::temp_dir().join(format!(
            "archgraph-native-go-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        for (name, contents) in files {
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        fixture
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn metadata_go(fixture: &Fixture, packages: &[serde_json::Value]) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = fixture.path().join("fake-go");
    fs::write(&path, "#!/bin/sh\nset -eu\n[ \"$*\" = 'list -e -deps -json ./...' ]\n[ \"$GOTOOLCHAIN\" = local ]\n[ \"$GOPROXY\" = off ]\n[ \"$GOSUMDB\" = off ]\n[ \"$GOWORK\" = off ]\n[ \"$CGO_ENABLED\" = 0 ]\n[ \"$GOFLAGS\" = '-mod=readonly' ]\ncat \"$0.json\"\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        path.with_extension("json"),
        packages
            .iter()
            .map(|p| serde_json::to_string(p).unwrap())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
#[cfg(unix)]
fn native_go_queries_resolve_named_blank_and_same_package_references_without_compilation() {
    let fixture = Fixture::new(&[
        ("go.mod", "module example.test/app\n\ngo 1.21\n"),
        (
            "main.go",
            "package main\nimport renamed \"example.test/app/model\"\nimport _ \"example.test/app/hooks\"\nimport \"fmt\"\nfunc main() { fmt.Println(renamed.Thing{}, helper()) }\nfunc init(){ panic(\"never execute\") }\n",
        ),
        (
            "helper.go",
            "package main\nfunc helper() string { main(); return \"safe\" }\n",
        ),
        ("main_test.go", "package main\nfunc TestExcluded() {}\n"),
        ("model/model.go", "package unusual\ntype Thing struct {}\n"),
        (
            "hooks/doc.go",
            "package hooks\nfunc init() { panic(\"never execute\") }\n",
        ),
    ]);
    let packages = [
        serde_json::json!({"Dir":fixture.path(),"ImportPath":"example.test/app","Name":"main","GoFiles":["main.go","helper.go"],"TestGoFiles":["main_test.go"]}),
        serde_json::json!({"Dir":fixture.path().join("model"),"ImportPath":"example.test/app/model","Name":"unusual","GoFiles":["model.go"]}),
        serde_json::json!({"Dir":fixture.path().join("hooks"),"ImportPath":"example.test/app/hooks","Name":"hooks","GoFiles":["doc.go"]}),
        serde_json::json!({"Dir":"/a/known/go/stdlib/fmt","ImportPath":"fmt","Name":"fmt"}),
    ];
    let executable = metadata_go(&fixture, &packages);
    let policy = Policy::new(fixture.path(), Language::Go, Options::default()).unwrap();
    let output = go::collect(fixture.path(), &executable, &policy).unwrap();
    let parents = std::env::current_dir()
        .unwrap()
        .components()
        .filter(|part| matches!(part, std::path::Component::Normal(_)))
        .count();
    let relative_executable = format!(
        "{}{}",
        "../".repeat(parents),
        executable.trim_start_matches('/')
    );
    assert_eq!(
        go::collect(fixture.path(), &relative_executable, &policy)
            .unwrap()
            .raw
            .files,
        output.raw.files
    );
    assert_eq!(output.raw.collector.name, "tree-sitter-go");
    assert_eq!(output.source_root, ".");
    assert!(!output.symbols.complete);
    assert!(
        output
            .symbols
            .limitations
            .iter()
            .any(|limit| limit.contains("header-only generated"))
    );
    assert!(output.raw.files.contains(&"main_test.go".into()));
    assert!(
        !output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "main_test.go" || e.to == "main_test.go")
    );
    for target in ["helper.go", "model/model.go", "hooks/doc.go"] {
        assert!(
            output
                .raw
                .edges
                .iter()
                .any(|e| e.from == "main.go" && e.to == target),
            "missing edge to {target}"
        );
    }
    assert!(output.raw.edges.iter().all(|e| e.kind == EdgeKind::Static));
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "helper.go" && e.to == "main.go")
    );
    assert_eq!(
        output.raw.collector.bound_files,
        ["helper.go", "hooks/doc.go", "main.go", "model/model.go"]
    );
    let imports = output.raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "fmt" && i.external.as_deref() == Some("fmt"))
    );
    assert!(
        imports
            .iter()
            .any(|i| i.specifier == "example.test/app/model" && i.targets == ["model/model.go"])
    );
    assert!(output.symbols.imports.iter().any(|i| i.name == "Thing"));
}

#[test]
#[cfg(unix)]
fn native_go_missing_metadata_is_unresolved_and_alias_shadowing_uses_conservative_entry() {
    let fixture = Fixture::new(&[
        ("go.mod", "module example.test/app\n\ngo 1.21\n"),
        (
            "a.go",
            "package app\nimport alias \"example.test/app/other\"\nimport \"missing.invalid/pkg\"\nfunc f(){ alias := struct{ Thing int }{}; _ = alias.Thing }\n",
        ),
        ("other/doc.go", "package other\n"),
        ("other/value.go", "package other\ntype Thing struct{}\n"),
    ]);
    let executable = metadata_go(
        &fixture,
        &[
            serde_json::json!({"Dir":fixture.path(),"ImportPath":"example.test/app","Name":"app","GoFiles":["a.go"]}),
            serde_json::json!({"Dir":fixture.path().join("other"),"ImportPath":"example.test/app/other","Name":"other","GoFiles":["doc.go","value.go"]}),
            serde_json::json!({"ImportPath":"missing.invalid/pkg","Error":{"Err":"cannot resolve"}}),
        ],
    );
    let policy = Policy::new(fixture.path(), Language::Go, Options::default()).unwrap();
    let output = go::collect(fixture.path(), &executable, &policy).unwrap();
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.to == "other/doc.go"
                && e.confidence == Some(archgraph::model::Confidence::Low))
    );
    assert!(!output.raw.edges.iter().any(|e| e.to == "other/value.go"));
    assert!(
        output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|i| i.specifier == "missing.invalid/pkg" && i.unresolved.is_some())
    );
}

#[test]
#[ignore = "requires an installed Go toolchain; never compiles or runs repository code"]
fn native_go_live_list_never_runs_poisoned_initializers() {
    let fixture = Fixture::new(&[
        ("go.mod", "module example.test/native\n\ngo 1.21\n"),
        (
            "a.go",
            "package native\nimport \"fmt\"\nfunc init(){panic(\"must not execute\")}\nfunc F(){fmt.Println(helper())}\n",
        ),
        (
            "b.go",
            "package native\nfunc helper() string{return \"ok\"}\n",
        ),
    ]);
    let policy = Policy::new(fixture.path(), Language::Go, Options::default()).unwrap();
    let output = go::collect(fixture.path(), "go", &policy).unwrap();
    assert!(
        output
            .raw
            .edges
            .iter()
            .any(|e| e.from == "a.go" && e.to == "b.go")
    );
    assert!(
        output
            .raw
            .imports
            .unwrap()
            .iter()
            .any(|i| i.external.as_deref() == Some("fmt"))
    );
}

#[test]
fn native_go_names_a_missing_go_executable() {
    let fixture = Fixture::new(&[("go.mod", "module example.test/app\n")]);
    let policy = Policy::new(fixture.path(), Language::Go, Options::default()).unwrap();
    let missing = fixture.path().join("missing/go");
    let Err(error) = go::collect(fixture.path(), missing.to_str().unwrap(), &policy) else {
        panic!("collected with a missing go");
    };
    assert!(
        format!("{error:#}").contains(&format!("--go {}: no such executable", missing.display())),
        "{error:#}"
    );
}

#[test]
#[cfg(unix)]
fn native_go_binds_every_file_and_leaves_names_unknown() {
    let fixture = Fixture::new(&[
        ("go.mod", "module example.test/app\n\ngo 1.21\n"),
        ("a.go", "package app\nfunc A() int { return helper() }\n"),
        ("b.go", "package app\nfunc helper() int { return 1 }\n"),
        (
            "layout.yaml",
            "version: 1\nmoves:\n  - { from: \"b.go\", to: \"lib/\" }\n",
        ),
    ]);
    let executable = metadata_go(
        &fixture,
        &[
            serde_json::json!({"Dir":fixture.path(),"ImportPath":"example.test/app","Name":"app","GoFiles":["a.go","b.go"]}),
        ],
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "scan",
            fixture.path().to_str().unwrap(),
            "--engine",
            "native",
        ])
        .args(["--go", &executable, "--rules"])
        .arg(fixture.path().join("layout.yaml"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        doc["source"]["collector"]["bound_files"],
        serde_json::json!(["a.go", "b.go"])
    );
    assert_eq!(doc["source"]["collector"].get("bound"), None);
    assert_eq!(doc["target"]["moves"][0]["names"], serde_json::Value::Null);
    assert_eq!(
        doc["target"]["notes"],
        serde_json::json!(["names to export unknown for tree-sitter-go"])
    );
}
