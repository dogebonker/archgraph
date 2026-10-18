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
