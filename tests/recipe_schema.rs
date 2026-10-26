mod common;

use serde_json::{Value, json};

fn schema() -> Value {
    serde_json::from_str(include_str!("../schema/boundrail.v1.2.schema.json")).unwrap()
}

fn recipe() -> Value {
    json!({
        "steps": [{"kind":"cut_import", "file":"app/a.ts", "line":3,
            "from":"app/a.ts", "to":"core/b.ts", "detail":{}, "mechanical":false}],
        "expected":{"pairs_removed":1,"cycle_size_after":0,"new_cycles":0,
            "instability_delta":{"app":-0.5}},
        "verify":{"predicate":"cycles_in(app) == 0", "structured":{"cycles_in":["app"],"equals":0}},
        "confidence":"medium", "risk_notes":["Imported bindings require agent changes."]
    })
}

fn definition(name: &str) -> jsonschema::Validator {
    let mut value = schema();
    value["$ref"] = json!(format!("#/$defs/{name}"));
    value.as_object_mut().unwrap().remove("allOf");
    value.as_object_mut().unwrap().remove("required");
    value.as_object_mut().unwrap().remove("properties");
    jsonschema::validator_for(&value).unwrap()
}

#[test]
fn v12_schema_and_every_recipe_step_are_typed() {
    jsonschema::draft202012::meta::validate(&schema()).unwrap();
    let validator = definition("recipe");
    for kind in [
        "cut_import",
        "move_file",
        "add_reexport",
        "extract_interface",
        "split_module",
    ] {
        let mut value = recipe();
        value["steps"][0]["kind"] = json!(kind);
        assert!(validator.is_valid(&value), "{kind}");
    }
    let mut value = recipe();
    value["steps"][0]["kind"] = json!("run_shell");
    assert!(!validator.is_valid(&value));
    value = recipe();
    value["verify"]["structured"]["equals"] = json!(-1);
    assert!(!validator.is_valid(&value));
    value = recipe();
    value["steps"][0]["line"] = json!(0);
    assert!(!validator.is_valid(&value));
}

#[test]
fn actionable_findings_require_recipes_but_diagnostics_do_not() {
    let validator = definition("finding");
    let mut finding = json!({"kind":"deep-import","subject":"app/a.ts","evidence":{"from":"app/a.ts","to":"core/b.ts","entrypoint":"core/index.ts","symbols":["B"]}});
    assert!(!validator.is_valid(&finding));
    finding["finding_id"] = json!(format!("br-{}", "a".repeat(64)));
    finding["recipe"] = recipe();
    assert!(validator.is_valid(&finding));
    assert!(definition("diagnostic").is_valid(&json!({
        "finding":{"kind":"small-repo","subject":".","evidence":{"files":2,"modules":1}},
        "reason":"Not an executable architecture change."
    })));
}

#[test]
fn legacy_export_restores_advisories_and_removes_recipe_fields() {
    let mut value = json!({"schema":"boundrail/v1.2","kind":"scan",
        "findings":[
            {"kind":"forbidden-edge","subject":"a","evidence":{},"recipe":recipe()},
            {"kind":"cycle","subject":"a","evidence":{},"finding_id":"br-id","recipe":recipe()}],
        "diagnostics":[{"finding":{"kind":"small-repo","subject":".",
            "evidence":{"files":2,"modules":1},"finding_id":"br-advisory"},"reason":"advisory"}]
    });
    boundrail::compat::v1(&mut value).unwrap();
    assert_eq!(value["schema"], "boundrail/v1");
    assert!(value.get("diagnostics").is_none());
    assert_eq!(value["findings"].as_array().unwrap().len(), 2);
    assert_eq!(value["findings"][1]["kind"], "small-repo");
    for finding in value["findings"].as_array().unwrap() {
        assert!(finding.get("recipe").is_none());
        assert!(finding.get("finding_id").is_none());
    }
}

#[test]
fn legacy_export_also_accepts_v11_documents() {
    let mut value = json!({"schema":"boundrail/v1.1","kind":"scan","findings":[],
        "edges":[{"from":"a.ts","to":"b.ts","kind":"runtime"}]});
    boundrail::compat::v1(&mut value).unwrap();
    assert_eq!(value["schema"], "boundrail/v1");
    assert_eq!(value["edges"][0]["kind"], "static");
}

#[test]
fn reader_recognizes_all_published_graph_versions() {
    for version in ["boundrail/v1", "boundrail/v1.1", "boundrail/v1.2"] {
        assert!(boundrail::model::supported_schema(version), "{version}");
    }
    assert!(!boundrail::model::supported_schema("boundrail/v2"));
}

#[test]
fn old_scan_documents_restore_into_current_recipe_schema() {
    let (current, _, _) = common::scan_recorded("ts-basic", &[]);
    for version in ["boundrail/v1", "boundrail/v1.1", "boundrail/v1.2"] {
        let mut value = current.clone();
        if version == "boundrail/v1" {
            boundrail::compat::v1(&mut value).unwrap();
        } else {
            value["schema"] = json!(version);
        }
        let input = serde_json::from_value(value).unwrap();
        let restored = serde_json::to_value(boundrail::restore_scan(input).unwrap()).unwrap();
        assert_eq!(restored["schema"], "boundrail/v1.2");
        common::validate(&restored);
    }
}

#[test]
fn recipe_application_and_verification_documents_have_typed_public_schemas() {
    let validator = jsonschema::validator_for(&schema()).unwrap();
    let id = format!("br-{}", "b".repeat(64));
    let verify = json!({"pass":true,"checks":[{"predicate":"cycles_in","pass":true,"expected":0,"actual":0}]});
    let mut proposal = recipe();
    proposal["schema"] = json!("boundrail/v1.2");
    proposal["kind"] = json!("recipe");
    proposal["finding_id"] = json!(id);
    let mut documents = vec![
        proposal,
        json!({"schema":"boundrail/v1.2","kind":"apply-plan","finding_id":id,"recipe":recipe(),"can_apply":false,"pending":["requires agent work"],"changes":[],"moves":[]}),
        json!({"schema":"boundrail/v1.2","kind":"apply","finding_id":id,"applied":true,"verify":verify,"hint":null}),
        json!({"schema":"boundrail/v1.2","kind":"verification","finding_id":id,"verify":verify}),
    ];
    for value in &documents {
        assert!(validator.is_valid(value), "{value}");
    }
    documents[1]["can_apply"] = json!("yes");
    assert!(!validator.is_valid(&documents[1]));
    documents[2]["verify"]["pass"] = json!("passed");
    assert!(!validator.is_valid(&documents[2]));
    documents[3]["verify"]["checks"][0]
        .as_object_mut()
        .unwrap()
        .remove("actual");
    assert!(!validator.is_valid(&documents[3]));
}
