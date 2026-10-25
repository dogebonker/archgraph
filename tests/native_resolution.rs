mod common;

use boundrail::collect::{java, rust};
use boundrail::model::ImportReason;
use serde_json::json;

#[test]
fn old_native_recordings_leave_import_coverage_unknown() {
    let java =
        java::parse(br#"{"java_version":"17","files":[],"edges":[],"unresolved":0}"#).unwrap();
    assert!(java.imports.is_none());
    let input = json!({"rust_analyzer":"test","toolchain":"stable","features":"default", "build_scripts":false,"packages":[],"documents":[]});
    assert!(
        rust::parse(&serde_json::to_vec(&input).unwrap())
            .unwrap()
            .imports
            .is_none()
    );
}

#[test]
fn java_compiler_accounts_for_explicit_imports_and_missing_libraries() {
    if !std::process::Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("JDK unavailable; native import ledger smoke test skipped");
        return;
    }
    let file = common::write_tmp(
        "native-import-ledger/demo/Local.java",
        "package demo; public class Local { public static int VALUE; }",
    );
    common::write_tmp(
        "native-import-ledger/use/Use.java",
        "package use;\nimport demo.Local;\nimport static demo.Local.VALUE;\nimport java.util.*;\nimport missing.library.Nope;\nimport missing.library.*;\nimport static demo.Local.*;\nimport demo.*;\nclass Use { Local value; }",
    );
    let root = std::path::Path::new(&file)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let exclude = globset::GlobSetBuilder::new().build().unwrap();
    let graph = java::parse(&java::run(root, "java", &exclude).unwrap()).unwrap();
    let imports = graph.imports.unwrap();
    assert_eq!(imports.len(), 7);
    assert_eq!(imports[0].targets, ["demo/Local.java"]);
    assert_eq!(imports[0].line, Some(2));
    assert_eq!(imports[1].targets, ["demo/Local.java"]);
    assert_eq!(imports[2].external.as_deref(), Some("java.util"));
    assert!(matches!(imports[3].unresolved, Some(ImportReason::Missing)));
    assert!(matches!(imports[4].unresolved, Some(ImportReason::Missing)));
    assert!(imports[4].external.is_none());
    assert!(imports[4].targets.is_empty());
    assert_eq!(imports[5].targets, ["demo/Local.java"]);
    assert!(imports[5].unresolved.is_none());
    assert_eq!(imports[6].targets, ["demo/Local.java"]);
    assert!(imports[6].unresolved.is_none());
    assert!(!root.join("demo/Local.class").exists());
}
