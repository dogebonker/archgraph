mod common;

use common::{layout, run_json, scan_recorded, write_tmp};
use serde_json::{Value, json};

/// The scan of `name` and a copy without the edge `from -> to`, as files.
fn before_after(name: &str, extra: &[&str], from: &str, to: &str) -> (String, String) {
    let (doc, stdout, _) = scan_recorded(name, extra);
    let old = write_tmp(&format!("diff/{name}-old.json"), &stdout);
    let mut fixed = doc.clone();
    let edges = fixed["edges"].as_array_mut().unwrap();
    let before = edges.len();
    edges.retain(|e| !(e["from"] == from && e["to"] == to));
    assert_eq!(edges.len(), before - 1);
    let new = write_tmp(&format!("diff/{name}-new.json"), &fixed.to_string());
    (old, new)
}

#[test]
fn diff_shows_a_resolved_cycle() {
    // Dropping the break-set import c/z.ts -> a/x.ts resolves the a -> b -> c module cycle.
    let (old, new) = before_after("ts-cycles", &[], "src/c/z.ts", "src/a/x.ts");
    let (diff, code) = run_json(&["diff", &old, &new]);
    assert_eq!(code, Some(0));
    assert_eq!(diff["schema"], "archgraph/v1");
    assert_eq!(diff["kind"], "diff");
    assert_eq!(diff["old"]["commit"], Value::Null);
    assert!(diff["old"]["generated_at"].is_string() && diff["new"]["generated_at"].is_string());
    assert_eq!(
        diff["resolved_cycles"],
        json!([{ "level": "module", "members": ["a", "b", "c"], "exact": true,
                 "break": [{ "from": "src/c/z.ts", "to": "src/a/x.ts" }] }])
    );
    assert_eq!(diff["new_cycles"], json!([]));
    assert_eq!(diff["added_modules"], json!([]));
    assert_eq!(diff["removed_modules"], json!([]));
    assert_eq!(diff["new_forbidden"], json!([]));
    assert_eq!(diff["resolved_forbidden"], json!([]));
    // a loses its importer and c its only outgoing import; every module is listed.
    assert_eq!(
        diff["modules"]["a"],
        json!({ "cohesion": [0.0, 0.0], "ca": [1, 0], "ce": [1, 1], "instability": [0.5, 1.0] })
    );
    assert_eq!(
        diff["modules"]["c"],
        json!({ "cohesion": [0.0, null], "ca": [1, 1], "ce": [1, 0], "instability": [0.5, 0.0] })
    );
    assert_eq!(
        diff["modules"]["f"],
        json!({ "cohesion": [1.0, 1.0], "ca": [0, 0], "ce": [0, 0], "instability": [null, null] })
    );
    assert_eq!(diff["modules"].as_object().unwrap().len(), 6);
}

#[test]
fn diff_shows_a_resolved_forbidden_edge_and_a_removed_module() {
    // domain/user.ts -> adapters/out/pg.ts is forbidden by `layers`.
    let rules = layout("ts-hexagonal");
    let (old, new) = before_after(
        "ts-hexagonal",
        &["--rules", &rules],
        "src/domain/user.ts",
        "src/adapters/out/pg.ts",
    );
    let (diff, _) = run_json(&["diff", &old, &new]);
    assert_eq!(
        diff["resolved_forbidden"],
        json!([{ "from": "src/domain/user.ts", "to": "src/adapters/out/pg.ts", "kind": "static",
                 "verdict": "forbidden", "rule": "layers" }])
    );
    assert_eq!(diff["new_forbidden"], json!([]));

    // Against a scan of a different tree, ts-cycles' modules all count as added.
    let (_, other, _) = scan_recorded("ts-cycles", &[]);
    let other = write_tmp("diff/other.json", &other);
    let (diff, _) = run_json(&["diff", &old, &other]);
    assert_eq!(diff["added_modules"], json!(["a", "b", "c", "d", "e", "f"]));
    assert_eq!(
        diff["removed_modules"],
        json!([".", "adapters/in", "adapters/out", "application", "domain"])
    );
    assert_eq!(diff["modules"], json!({}));
    assert_eq!(diff["new_cycles"].as_array().unwrap().len(), 3);
    assert_eq!(diff["resolved_cycles"].as_array().unwrap().len(), 1);
}
