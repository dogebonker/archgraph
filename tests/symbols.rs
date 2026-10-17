mod common;

use archgraph::{
    model::{Language, ScanInput},
    symbols::{self, Options},
};
use std::{fs, path::Path};

fn fixture(name: &str) -> (String, archgraph::model::Document) {
    let (root, input) = common::synthetic(
        &format!("{name}-{}", std::process::id()),
        2,
        &[("app.ts".into(), "pkg.ts".into())],
    );
    fs::write(Path::new(&root).join("app.py"), "from pkg import Order\n").unwrap();
    fs::write(Path::new(&root).join("pkg.py"), "class Order: pass\n").unwrap();
    let (_, json, _) = common::scan(&[
        &root,
        "--language",
        "typescript",
        "--collector-input",
        &input,
    ]);
    let input: ScanInput = serde_json::from_str(&json).unwrap();
    let mut doc = archgraph::restore_scan(input).unwrap();
    doc.source.language = Language::Python;
    for file in &mut doc.files {
        file.path = file.path.replace(".ts", ".py");
    }
    (root, doc)
}

#[test]
fn symbol_collection_rejects_parent_paths_before_launching_tools() {
    let (root, mut doc) = fixture("symbols-parent-escape");
    doc.files[0].path = "../outside.py".into();
    let error = symbols::collect(
        Path::new(&root),
        &doc,
        Options {
            depcruise: None,
            python: "nonexistent-python",
            go: "nonexistent-go",
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("must stay inside scan root"));
}

#[cfg(unix)]
#[test]
fn symbol_collection_rejects_symlinks_outside_scan_root() {
    let (root, doc) = fixture("symbols-symlink-escape");
    let source = Path::new(&root).join("app.py");
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", source).unwrap();
    let error = symbols::collect(
        Path::new(&root),
        &doc,
        Options {
            depcruise: None,
            python: "nonexistent-python",
            go: "nonexistent-go",
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("must stay inside scan root"));
}

#[test]
#[ignore = "requires Python 3"]
fn python_collector_isolates_repository_standard_library_shadows() {
    let (root, doc) = fixture("symbols-python-isolated");
    fs::write(
        Path::new(&root).join("ast.py"),
        "raise RuntimeError('repository code executed')\n",
    )
    .unwrap();
    let result = symbols::collect(
        Path::new(&root),
        &doc,
        Options {
            depcruise: None,
            python: "python3",
            go: "go",
        },
    )
    .unwrap();
    assert!(result.complete, "{:?}", result.limitations);
    assert!(result.imports.iter().any(|item| item.name == "Order"));
}
