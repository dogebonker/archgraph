mod common;

use common::{boundrail, run_json, scan_recorded, validate, write_tmp};
use serde_json::Value;

fn legacy(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, _| {
                !key.starts_with("weighted_")
                    && !matches!(
                        key.as_str(),
                        "weight" | "is_reexport" | "type_only_cycles" | "all_kinds"
                    )
            });
            if let Some(Value::String(kind)) = object.get_mut("kind") {
                match kind.as_str() {
                    "runtime" => *kind = "static".into(),
                    "lazy" => *kind = "dynamic".into(),
                    _ => {}
                }
            }
            for child in object.values_mut() {
                legacy(child);
            }
        }
        Value::Array(array) => array.iter_mut().for_each(legacy),
        _ => {}
    }
}

#[test]
fn v1_and_v1_1_documents_produce_the_same_reader_results() {
    let (modern, _, _) = scan_recorded("ts-cycles", &[]);
    let mut old = modern.clone();
    legacy(&mut old);
    old["schema"] = "boundrail/v1".into();
    validate(&old);
    let modern_path = write_tmp("migration/current.json", &modern.to_string());
    let legacy_path = write_tmp("migration/legacy.json", &old.to_string());

    let current_check = boundrail(&["check", &modern_path]);
    let legacy_check = boundrail(&["check", &legacy_path]);
    assert_eq!(legacy_check.status.code(), current_check.status.code());
    assert_eq!(legacy_check.stdout, current_check.stdout);

    let current_sim = run_json(&[
        "simulate",
        &modern_path,
        "--move",
        "src/c/z.ts",
        "src/a/z.ts",
    ]);
    let legacy_sim = run_json(&[
        "simulate",
        &legacy_path,
        "--move",
        "src/c/z.ts",
        "src/a/z.ts",
    ]);
    assert_eq!(legacy_sim, current_sim);
    let (difference, status) = run_json(&["diff", &legacy_path, &modern_path]);
    assert_eq!(status, Some(0));
    for key in [
        "new_cycles",
        "resolved_cycles",
        "new_forbidden",
        "resolved_forbidden",
        "added_modules",
        "removed_modules",
    ] {
        assert!(difference[key].as_array().unwrap().is_empty(), "{key}");
    }
    let mut rendered = Vec::new();
    for (graph, output) in [
        (&modern_path, "migration/current.html"),
        (&legacy_path, "migration/legacy.html"),
    ] {
        let output = write_tmp(output, "");
        let result = boundrail(&["render", graph, "-o", &output]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        rendered.push(std::fs::read_to_string(output).unwrap());
    }
    assert_eq!(rendered[0], rendered[1]);
}
