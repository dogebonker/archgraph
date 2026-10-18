mod common;

use serde_json::{Value, json};

#[path = "../src/cli/explain.rs"]
mod explain;

#[test]
fn rust_report_and_agent_explanations_disclose_grouping_and_source_semantics() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("source-evidence-{}", std::process::id()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='evidence'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub mod a;\npub mod b;\n").unwrap();
    std::fs::write(root.join("src/a.rs"), "pub struct A;\n").unwrap();
    std::fs::write(
        root.join("src/b.rs"),
        "use crate::a::A;\npub fn take(_: A) {}\n",
    )
    .unwrap();
    let output = root.join("report");
    let args = [
        "report",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--language",
        "rust",
        "--no-provenance",
        "--out-dir",
        output.to_str().unwrap(),
    ];
    let full = common::archgraph(&args);
    assert!(
        full.status.success(),
        "{}",
        String::from_utf8_lossy(&full.stderr)
    );
    let text = String::from_utf8(full.stdout).unwrap();
    assert!(
        text.contains("grouping: folder depth 2 under src"),
        "{text}"
    );
    assert!(
        text.contains("source/type coupling, not runtime initialization"),
        "{text}"
    );
    let brief = common::archgraph(&[&args[..], &["--brief"]].concat());
    assert!(brief.status.success());
    let brief: Value = serde_json::from_slice(&brief.stdout).unwrap();
    assert_eq!(brief["result"]["grouping"], "folder depth 2 under src");
    assert_eq!(
        brief["result"]["source_semantics"],
        "rust-source-references"
    );
    let value =
        serde_json::from_slice(&std::fs::read(output.join("archgraph.json")).unwrap()).unwrap();
    let doc = archgraph::restore_scan(value).unwrap();
    let cli = explain::explain(&doc, "src/a.rs").unwrap();
    let mcp = archgraph::mcp_tools::call(&doc, &root, "explain", json!({"path":"src/a.rs"}), None)
        .unwrap();
    for value in [&cli, &mcp] {
        assert_eq!(value["grouping"], "folder depth 2 under src");
        assert_eq!(value["source_semantics"], "rust-source-references");
    }
    let mut config = doc.config.clone();
    config.granularity = archgraph::model::Granularity::File;
    assert_eq!(
        archgraph::modules::grouping(&config),
        "file granularity under src"
    );
    config.granularity = archgraph::model::Granularity::Folder;
    config.package_modules = true;
    assert_eq!(
        archgraph::modules::grouping(&config),
        "package directories under src"
    );
    std::fs::remove_dir_all(root).unwrap();
}
