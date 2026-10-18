mod common;
use common::{archgraph, scan, synthetic_typed, validate};
use serde_json::Value;

#[test]
fn erased_reexports_still_define_the_public_boundary() {
    let (root, input) = synthetic_typed(
        "type-public-boundary",
        5,
        &[
            ("src/a/index.ts", "src/a/model.ts", &["export", "type-only"]),
            ("src/b/via.ts", "src/a/index.ts", &["import", "type-only"]),
            ("src/b/deep.ts", "src/a/model.ts", &["import", "type-only"]),
        ],
    );
    let (doc, _, _) = scan(&[
        &root,
        "--language",
        "typescript",
        "--collector-input",
        &input,
    ]);
    let edge = doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["from"] == "src/a/index.ts")
        .unwrap();
    assert_eq!(edge["kind"], "type");
    assert_eq!(edge["is_reexport"], true);
    assert!(
        doc["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["finding"]["kind"] == "internals-reach")
    );
}

#[test]
fn explicit_legacy_export_validates_against_the_frozen_v1_schema() {
    let root = common::fixture("ts-basic");
    let out = archgraph(&[
        "scan",
        root.to_str().unwrap(),
        "--collector-input",
        root.join("depcruise.json").to_str().unwrap(),
        "--legacy-v1",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "archgraph/v1");
    validate(&doc);
    assert!(
        doc["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "static")
    );
    assert!(
        doc["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "dynamic")
    );
}
