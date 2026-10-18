use archgraph::{
    collect::{
        excludes::{Options, Policy},
        native::rust,
    },
    model::Language,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static SEQ: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "archgraph-native-rust-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        Self(root)
    }
    fn scan(&self) -> archgraph::collect::native::Output {
        let policy = Policy::new(&self.0, Language::Rust, Options::default()).unwrap();
        rust::collect(&self.0, &policy).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn module_groups_aliases_and_qualified_paths_use_tree_sitter() {
    let f = Fixture::new(&[
        (
            "Cargo.toml",
            "[package]\nname='sample'\nversion='0.1.0'\nedition='2024'",
        ),
        ("src/lib.rs", "mod domain; mod app;"),
        ("src/domain.rs", "pub struct Shape; pub struct Other;"),
        (
            "src/app.rs",
            "use crate::domain::{Shape as PublicShape, Other}; pub fn run() { let _ = crate::domain::Shape; }",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/app.rs" && e.to == "src/domain.rs")
    );
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.from == "src/app.rs" && r.targets == ["src/domain.rs"])
    );
    assert!(!out.symbols.complete);
}
#[test]
fn path_attributes_and_inline_super_paths_resolve() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "#[path = \"alternate.rs\"] mod domain; mod app;",
        ),
        ("src/alternate.rs", "pub struct Shape;"),
        (
            "src/app.rs",
            "mod nested { use super::super::domain::Shape; }",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/app.rs" && e.to == "src/alternate.rs")
    );
}
#[test]
fn public_use_chain_forwards_to_definition() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod domain; mod facade; mod app;"),
        ("src/domain.rs", "pub struct Shape;"),
        (
            "src/facade.rs",
            "pub use crate::domain::Shape as PublicShape;",
        ),
        (
            "src/app.rs",
            "use crate::facade::PublicShape; pub fn run() {}",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/app.rs" && e.to == "src/domain.rs")
    );
    assert!(out.symbols.exports.iter().any(|e| e.file == "src/facade.rs"
        && e.name == "PublicShape"
        && e.target.file == "src/domain.rs"));
}
#[test]
fn dependency_rename_is_metadata_and_build_script_never_runs() {
    let f = Fixture::new(&[
        (
            "Cargo.toml",
            "[package]\nname='sample'\nversion='0.1.0'\nbuild='build.rs'\n[dependencies]\nwire={package='serde',version='1'}",
        ),
        (
            "src/lib.rs",
            "use wire::Serialize; use missing_crate::Thing;",
        ),
        (
            "build.rs",
            "fn main(){std::fs::write(\"EXECUTED\",\"bad\").unwrap();}",
        ),
    ]);
    let out = f.scan();
    let imports = out.raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|r| r.external.as_deref() == Some("serde"))
    );
    assert!(
        imports
            .iter()
            .any(|r| r.specifier.contains("missing_crate") && r.unresolved.is_some())
    );
    assert!(!f.0.join("EXECUTED").exists());
}
#[test]
fn missing_modules_and_macros_do_not_claim_complete_resolution() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod generated; include!(concat!(env!(\"OUT_DIR\"),\"/generated.rs\"));",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.unresolved.is_some())
    );
    assert!(
        out.symbols
            .limitations
            .iter()
            .any(|s| s.contains("macro expansion"))
    );
}
#[test]
fn custom_library_and_binary_entrypoints_resolve_outline_modules() {
    for manifest in [
        "[lib]\npath='entry.rs'",
        "[[bin]]\nname='custom'\npath='entry.rs'",
    ] {
        let f = Fixture::new(&[
            (
                "Cargo.toml",
                &format!("[package]\nname='sample'\nversion='0.1.0'\n{manifest}"),
            ),
            ("entry.rs", "mod domain; use crate::domain::Shape;"),
            ("domain.rs", "pub struct Shape;"),
        ]);
        let out = f.scan();
        assert!(
            out.raw
                .edges
                .iter()
                .any(|e| e.from == "entry.rs" && e.to == "domain.rs")
        );
    }
}
#[test]
fn star_forwarding_and_workspace_dependency_inheritance() {
    let f = Fixture::new(&[
        (
            "Cargo.toml",
            "[workspace]\nmembers=['core','app']\n[workspace.dependencies]\nwire={package='serde',version='1'}",
        ),
        (
            "core/Cargo.toml",
            "[package]\nname='shared-core'\nversion='0.1.0'",
        ),
        ("core/src/lib.rs", "pub struct Shape;"),
        (
            "app/Cargo.toml",
            "[package]\nname='app'\nversion='0.1.0'\n[dependencies]\nshared-core={path='../core'}\nwire={workspace=true}",
        ),
        (
            "app/src/lib.rs",
            "mod facade; mod consumer; use wire::Serialize;",
        ),
        ("app/src/facade.rs", "pub use shared_core::*;"),
        (
            "app/src/consumer.rs",
            "use crate::facade::Shape; pub fn run() {}",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "app/src/consumer.rs" && e.to == "core/src/lib.rs")
    );
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.external.as_deref() == Some("serde"))
    );
}
#[test]
fn multiline_grouped_import_evidence_contains_no_control_characters() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod domain; use crate::domain::{\n Shape,\n Other,\n};",
        ),
        ("src/domain.rs", "pub struct Shape; pub struct Other;"),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .all(|r| !r.specifier.chars().any(char::is_control))
    );
}
#[test]
fn qualified_associated_calls_require_a_proven_source_declaration() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod domain; mod app;"),
        (
            "src/domain.rs",
            "pub struct Shape; impl Shape { pub fn new()->Self {Self} }",
        ),
        (
            "src/app.rs",
            "pub fn run() { let _=crate::domain::Shape::new(); let _=crate::missing::Thing::new(); }",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/app.rs" && e.to == "src/domain.rs")
    );
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.specifier.contains("missing") && r.unresolved.is_some())
    );
}
#[test]
fn packaged_copies_do_not_shadow_workspace_crates() {
    let manifest = "[package]\nname='b'\nversion='0.1.0'";
    let f = Fixture::new(&[
        (
            "a/Cargo.toml",
            "[package]\nname='a'\nversion='0.1.0'\n[dependencies]\nb={path='../b'}",
        ),
        ("a/src/lib.rs", "use b::foo;"),
        ("b/Cargo.toml", manifest),
        ("b/src/lib.rs", "pub fn foo() {}"),
        ("target/package/b-0.1.0/Cargo.toml", manifest),
        ("target/package/b-0.1.0/src/lib.rs", "pub fn foo() {}"),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "a/src/lib.rs" && e.to == "b/src/lib.rs")
    );
}
#[test]
fn relative_paths_prefer_child_modules_and_path_attributes_skip_comments() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod a; mod inner;\n#[path = \"alternate.rs\"]\n/// Domain docs.\n#[cfg(all())]\nmod domain;",
        ),
        ("src/inner.rs", "pub struct Thing;"),
        ("src/a/mod.rs", "mod inner; use inner::Thing;"),
        ("src/a/inner.rs", "pub struct Thing;"),
        ("src/alternate.rs", "pub struct Shape;"),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|r| r.specifier == "inner::Thing" && r.targets == ["src/a/inner.rs"])
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/lib.rs" && e.to == "src/alternate.rs")
    );
}

#[test]
fn containment_and_test_only_imports_do_not_create_runtime_cycles() {
    use archgraph::model::EdgeKind;
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod domain; mod consumer; #[cfg(test)] mod custom;",
        ),
        (
            "src/domain.rs",
            "pub struct Shape; #[cfg(test)] mod checks { use crate::consumer::run; }",
        ),
        (
            "src/consumer.rs",
            "use crate::domain::Shape; pub fn run() {}",
        ),
        ("src/custom.rs", "use crate::consumer::run;"),
        ("src/tests.rs", "use crate::consumer::run;"),
    ]);
    let out = f.scan();
    assert_eq!(out.source_root, "src");
    assert!(
        out.raw
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Static)
            .all(|e| e.from != "src/lib.rs"
                && e.from != "src/domain.rs"
                && e.from != "src/custom.rs"
                && e.from != "src/tests.rs")
    );
    assert!(
        !out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|r| r.from == "src/domain.rs")
    );
    assert!(!out.raw.files.contains(&"src/custom.rs".into()));
}

#[test]
fn wildcard_parent_import_and_exported_macro_use_actual_definitions() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod domain; mod macros; mod consumer; pub use domain::Shape;",
        ),
        ("src/domain.rs", "pub struct Shape;"),
        (
            "src/macros.rs",
            "#[macro_export] macro_rules! greet { () => {} }",
        ),
        (
            "src/consumer.rs",
            "use super::*; pub fn run(_: Shape) { crate::greet!(); }",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.symbols
            .exports
            .iter()
            .any(|e| e.file == "src/macros.rs" && e.name == "greet")
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/consumer.rs" && e.to == "src/domain.rs")
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/consumer.rs" && e.to == "src/macros.rs")
    );
    assert!(!out.raw.edges.iter().any(|e| e.from == "src/consumer.rs"
        && e.to == "src/lib.rs"
        && e.kind == archgraph::model::EdgeKind::Static));
}

#[test]
fn included_tests_keep_metadata_and_inline_test_accounting() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod domain; #[cfg(test)] mod custom;"),
        (
            "src/domain.rs",
            "pub struct Shape; #[cfg(test)] mod checks { use crate::custom::run; }",
        ),
        ("src/custom.rs", "pub fn run() {}"),
        ("src/helpers_tests.rs", "use crate::domain::Shape;"),
    ]);
    for options in [
        Options {
            include_tests: true,
            ..Default::default()
        },
        Options {
            includes: vec![
                "src/domain.rs".into(),
                "src/custom.rs".into(),
                "src/helpers_tests.rs".into(),
            ],
            ..Default::default()
        },
    ] {
        let policy = Policy::new(&f.0, Language::Rust, options).unwrap();
        let out = rust::collect(&f.0, &policy).unwrap();
        assert!(out.raw.metadata["src/custom.rs"].test);
        assert!(out.raw.metadata["src/helpers_tests.rs"].test);
        assert!(
            out.raw
                .imports
                .as_ref()
                .unwrap()
                .iter()
                .any(|r| r.from == "src/domain.rs" && r.targets == ["src/custom.rs"])
        );
        assert!(!out.raw.edges.iter().any(|e| e.from == "src/domain.rs"));
        assert!(
            out.symbols
                .limitations
                .iter()
                .any(|s| s.contains("inline tests cannot be independently visualized"))
        );
    }
}

#[test]
fn production_back_edges_survive_and_ambiguous_globs_are_structural() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod a; mod b;"),
        (
            "src/a.rs",
            "use crate::b::B; pub struct A; #[cfg(not(test))] fn production(_: B) {}",
        ),
        ("src/b.rs", "use super::*; use crate::a::A; pub struct B;"),
    ]);
    let out = f.scan();
    for (from, to) in [("src/a.rs", "src/b.rs"), ("src/b.rs", "src/a.rs")] {
        assert!(
            out.raw.edges.iter().any(|e| e.from == from
                && e.to == to
                && e.kind == archgraph::model::EdgeKind::Static)
        );
    }
    assert!(
        out.raw
            .edges
            .iter()
            .filter(|e| e.names == ["*"])
            .all(|e| e.kind == archgraph::model::EdgeKind::Reexport
                && e.confidence == Some(archgraph::model::Confidence::Low))
    );
}

#[test]
fn containment_is_not_a_reexport_cycle_but_public_forwarding_cycles_remain() {
    fn scan(f: &Fixture) -> serde_json::Value {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .args([
                "--no-daemon",
                "scan",
                f.0.to_str().unwrap(),
                "--engine",
                "native",
                "--language",
                "rust",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn has_reexport_cycle(doc: &serde_json::Value) -> bool {
        doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .chain(
                doc["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| &d["finding"]),
            )
            .any(|f| f["kind"] == "reexport-cycle")
    }
    let containment = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod child;"),
        ("src/child.rs", "use super::*;"),
    ]);
    assert!(!has_reexport_cycle(&scan(&containment)));
    let forwarding = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod a; mod b;"),
        ("src/a.rs", "pub use crate::b::Shape;"),
        ("src/b.rs", "pub use crate::a::Shape;"),
    ]);
    assert!(has_reexport_cycle(&scan(&forwarding)));
}

#[test]
fn included_cfg_test_files_are_classed_out_of_cycles_even_with_all_kinds() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "#[cfg(test)] mod checks; pub fn run() {}"),
        ("src/checks.rs", "use crate::run;"),
    ]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "--no-daemon",
            "scan",
            f.0.to_str().unwrap(),
            "--engine",
            "native",
            "--language",
            "rust",
            "--include-tests",
            "--all-kinds",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        doc["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "src/checks.rs" && f["class"] == "tests")
    );
    assert!(doc["cycles"].as_array().unwrap().is_empty());
}

#[test]
fn bare_local_module_calls_are_dependencies_and_can_close_production_cycles() {
    use archgraph::model::EdgeKind;
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod charging; mod unused; pub fn parent() { charging::charge(); UnknownType::new(); absent::call(); }",
        ),
        ("src/charging.rs", "pub fn charge() { crate::parent(); }"),
        ("src/unused.rs", "pub fn unused() {}"),
    ]);
    let out = f.scan();
    assert!(out.raw.edges.iter().any(|e| e.from == "src/lib.rs"
        && e.to == "src/charging.rs"
        && e.kind == EdgeKind::Static
        && e.specifier.as_deref() == Some("charging::charge")));
    assert!(out.raw.edges.iter().any(|e| e.from == "src/charging.rs"
        && e.to == "src/lib.rs"
        && e.kind == EdgeKind::Static));
    assert!(
        !out.raw
            .edges
            .iter()
            .any(|e| e.to == "src/unused.rs" && e.kind == EdgeKind::Static)
    );
    assert!(
        !out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|i| i.specifier.contains("UnknownType") || i.specifier.contains("absent"))
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "--no-daemon",
            "scan",
            f.0.to_str().unwrap(),
            "--engine",
            "native",
            "--language",
            "rust",
            "--granularity",
            "file",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doc["cycles"].as_array().unwrap().len(), 1);
    let members = doc["cycles"][0]["members"].as_array().unwrap();
    assert!(members.iter().any(|m| m == "src/lib.rs"));
    assert!(members.iter().any(|m| m == "src/charging.rs"));
    assert!(!members.iter().any(|m| m == "src/unused.rs"));
}

#[test]
fn only_actual_macro_export_attributes_prove_crate_macro_resolution() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod macros; mod consumer;"),
        (
            "src/macros.rs",
            "#[doc = \"macro_export\"] macro_rules! fake { () => {} } #[macro_export(local_inner_macros)] macro_rules! real { () => {} }",
        ),
        (
            "src/consumer.rs",
            "pub fn run() { crate::fake!(); crate::unknown!(); crate::real!(); }",
        ),
    ]);
    let out = f.scan();
    assert!(
        !out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/consumer.rs" && e.to == "src/lib.rs")
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/consumer.rs" && e.to == "src/macros.rs")
    );
    assert!(!out.symbols.exports.iter().any(|e| e.name == "fake"));
    for name in ["crate::fake", "crate::unknown"] {
        assert!(
            out.raw
                .imports
                .as_ref()
                .unwrap()
                .iter()
                .any(|r| r.specifier == name
                    && r.unresolved == Some(archgraph::model::ImportReason::Native)
                    && r.targets.is_empty())
        );
    }
}

#[test]
fn a_test_alias_does_not_erase_a_production_module() {
    for declarations in [
        "mod shared; #[cfg(test)] #[path=\"shared.rs\"] mod checks; mod consumer;",
        "#[cfg(test)] #[path=\"shared.rs\"] mod checks; mod shared; mod consumer;",
    ] {
        let f = Fixture::new(&[
            ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
            ("src/lib.rs", declarations),
            ("src/shared.rs", "pub struct Shape; mod nested;"),
            ("src/shared/nested.rs", "pub struct Nested;"),
            (
                "src/consumer.rs",
                "use crate::shared::Shape; use crate::shared::nested::Nested; pub fn run() {}",
            ),
        ]);
        let out = f.scan();
        assert!(out.raw.files.iter().any(|f| f == "src/shared.rs"));
        assert!(!out.raw.metadata["src/shared.rs"].test);
        assert!(out.raw.edges.iter().any(|e| e.from == "src/consumer.rs"
            && e.to == "src/shared.rs"
            && e.kind == archgraph::model::EdgeKind::Static));
        assert!(out.raw.edges.iter().any(|e| e.from == "src/consumer.rs"
            && e.to == "src/shared/nested.rs"
            && e.kind == archgraph::model::EdgeKind::Static));
    }
}

#[test]
fn functions_and_exported_macros_with_the_same_name_resolve_separately() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        ("src/lib.rs", "mod macros; mod consumer; pub fn same() {}"),
        (
            "src/macros.rs",
            "#[macro_export] macro_rules! same { () => {} }",
        ),
        (
            "src/consumer.rs",
            "pub fn run() {
crate::same();
crate::same!();
}",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|r| r.specifier == "crate::same" && r.targets == ["src/lib.rs"])
    );
    assert!(
        out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|r| r.specifier == "crate::same" && r.targets == ["src/macros.rs"])
    );
}

#[test]
fn parent_globs_keep_symbols_forwarded_by_nested_stars() {
    let f = Fixture::new(&[
        ("Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'"),
        (
            "src/lib.rs",
            "mod direct; mod shapes; mod consumer; pub use direct::Own; pub use shapes::*;",
        ),
        ("src/direct.rs", "pub struct Own;"),
        (
            "src/shapes.rs",
            "pub struct Shape; pub fn back() { crate::consumer::run(Shape, crate::Own); }",
        ),
        (
            "src/consumer.rs",
            "use super::*; pub fn run(_: Shape, _: Own) {}",
        ),
    ]);
    let out = f.scan();
    assert!(out.raw.edges.iter().any(|e| e.from == "src/consumer.rs"
        && e.to == "src/shapes.rs"
        && e.kind == archgraph::model::EdgeKind::Static));
    assert!(out.raw.edges.iter().any(|e| e.from == "src/consumer.rs"
        && e.to == "src/direct.rs"
        && e.kind == archgraph::model::EdgeKind::Static));
    assert!(out.raw.edges.iter().any(|e| e.from == "src/shapes.rs"
        && e.to == "src/consumer.rs"
        && e.kind == archgraph::model::EdgeKind::Static));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .args([
            "--no-daemon",
            "scan",
            f.0.to_str().unwrap(),
            "--engine",
            "native",
            "--language",
            "rust",
            "--granularity",
            "file",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(doc["cycles"].as_array().unwrap().iter().any(|cycle| {
        let members = cycle["members"].as_array().unwrap();
        members.iter().any(|m| m == "src/consumer.rs")
            && members.iter().any(|m| m == "src/shapes.rs")
    }));
}
