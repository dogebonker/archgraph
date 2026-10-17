mod common;

use std::path::Path;

use archgraph::collect::{detect_language, source_dir};
use archgraph::model::Language;

#[test]
fn detection_keeps_typescript_and_python_in_output_named_source_folders() {
    for (name, file, language) in [
        ("detect-ts-out", "src/out/a.ts", Language::Typescript),
        ("detect-py-target", "src/target/a.py", Language::Python),
    ] {
        let path = common::write_tmp(&format!("{name}/{file}"), "");
        let root = Path::new(&path).ancestors().nth(3).unwrap();
        assert_eq!(detect_language(root, source_dir(root)).unwrap(), language);
    }
}

#[test]
fn detection_finds_rust_workspace_members_and_ignores_language_build_output() {
    let manifest = common::write_tmp("detect-rust-workspace/Cargo.toml", "[workspace]\n");
    let root = Path::new(&manifest).parent().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    common::write_tmp("detect-rust-workspace/crates/app/src/lib.rs", "");
    for n in 0..3 {
        common::write_tmp(
            &format!("detect-rust-workspace/target/generated{n}.java"),
            "",
        );
    }
    assert_eq!(
        detect_language(root, source_dir(root)).unwrap(),
        Language::Rust
    );
}
