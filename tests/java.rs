mod common;

use archgraph::collect::java;
use common::{fixture, scan, write_tmp};
use serde_json::{Value, json};

fn edges(doc: &Value) -> Vec<(&str, &str)> {
    doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            assert_eq!(e["kind"], "runtime");
            (e["from"].as_str().unwrap(), e["to"].as_str().unwrap())
        })
        .collect()
}

#[test]
fn java_recording_uses_shared_analysis_and_test_excludes() {
    let root = fixture("java-resolve");
    let input = root.join("java.json");
    let rules = write_tmp("java-layout.yaml", "version: 1\n");
    let (doc, stdout, _) = scan(&[
        root.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
        "--rules",
        &rules,
    ]);
    assert_eq!(doc["source"]["language"], "java");
    assert_eq!(doc["source"]["collector"]["name"], "java-javac");
    assert_eq!(doc["files"].as_array().unwrap().len(), 3);
    assert_eq!(
        edges(&doc),
        [
            (
                "src/main/java/demo/model/Factory.java",
                "src/main/java/demo/model/Order.java"
            ),
            (
                "src/main/java/demo/service/Checkout.java",
                "src/main/java/demo/model/Factory.java"
            ),
            (
                "src/main/java/demo/service/Checkout.java",
                "src/main/java/demo/model/Order.java"
            ),
        ]
    );
    let graph = write_tmp("java-recorded.json", &stdout);
    assert!(common::archgraph(&["check", &graph]).status.success());
    assert!(
        common::archgraph(&["render", &graph, "-o", "/dev/null"])
            .status
            .success()
    );
    let (doc, _, _) = scan(&[
        root.to_str().unwrap(),
        "--language",
        "java",
        "--collector-input",
        input.to_str().unwrap(),
        "--include-tests",
    ]);
    assert_eq!(doc["files"].as_array().unwrap().len(), 4);
    assert_eq!(edges(&doc).len(), 4);
}

#[test]
fn java_recordings_reject_paths_outside_the_root() {
    for path in [
        "../outside.java",
        "/tmp/outside.java",
        "C:/outside.java",
        "a\\b.java",
    ] {
        let dump = json!({"java_version":"17", "files":[path], "edges":[], "unresolved":0});
        assert!(
            java::parse(&serde_json::to_vec(&dump).unwrap()).is_err(),
            "{path}"
        );
    }
}

#[test]
fn java_live_resolves_same_package_qualified_static_and_inherited_references() {
    if !std::process::Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("JDK unavailable; Java live collector check skipped");
        return;
    }
    let root = fixture("java-resolve");
    let (live, _, _) = scan(&[root.to_str().unwrap(), "--include-tests"]);
    let input = root.join("java.json");
    let (recorded, _, _) = scan(&[
        root.to_str().unwrap(),
        "--collector-input",
        input.to_str().unwrap(),
        "--include-tests",
    ]);
    assert_eq!(edges(&live), edges(&recorded));
    assert_eq!(live["source"]["collector"]["unresolved"], 0);
    assert!(!root.join("src/main/java/demo/model/Order.class").exists());
}

#[test]
fn java_live_reports_missing_libraries_and_never_executes_source() {
    if !std::process::Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("JDK unavailable; Java live collector check skipped");
        return;
    }
    let file = write_tmp(
        "java-no-execution/Probe.java",
        r#"
        import missing.library.Type;
        class Probe {
            static { if (true) throw new RuntimeException("must never execute"); }
            Type value;
        }
    "#,
    );
    let root = std::path::Path::new(&file).parent().unwrap();
    let (doc, _, stderr) = scan(&[root.to_str().unwrap(), "--language", "java"]);
    assert!(doc["source"]["collector"]["unresolved"].as_u64().unwrap() > 0);
    assert!(stderr.contains("unresolved"), "{stderr}");
    assert_eq!(doc["files"].as_array().unwrap().len(), 1);
    assert!(!root.join("Probe.class").exists());
}

#[test]
fn java_package_declarations_and_qualifiers_are_not_file_dependencies() {
    if !std::process::Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("JDK unavailable; Java live collector check skipped");
        return;
    }
    let file = write_tmp(
        "java-package-reference/demo/package-info.java",
        "package demo;",
    );
    write_tmp(
        "java-package-reference/demo/A.java",
        "package demo; public class A {}",
    );
    write_tmp(
        "java-package-reference/use/Use.java",
        "package use; class Use { demo.A value; }",
    );
    let root = std::path::Path::new(&file)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let (doc, _, _) = scan(&[root.to_str().unwrap(), "--language", "java"]);
    assert_eq!(edges(&doc), [("use/Use.java", "demo/A.java")]);
    assert_eq!(doc["source"]["collector"]["unresolved"], 0);
}
