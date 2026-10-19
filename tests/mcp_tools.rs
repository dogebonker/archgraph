use std::path::Path;

use archgraph::{
    mcp_tools::{call, finding_id, schemas},
    model::{Config, Document, Edge, Source},
};
use serde_json::{Value, json};

fn document(edges: &[(&str, &str)]) -> Document {
    let source:Source=serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":3},"git":{"commit":"test","dirty":false},"generated_at":"1970-01-01T00:00:00Z"})).unwrap();
    let config: Config = serde_json::from_value(
        json!({"depth":1,"source_root":"src","excludes":[],"include_tests":false,"rules":null}),
    )
    .unwrap();
    archgraph::analyze(
        source,
        config,
        vec![
            "src/a/a.ts".into(),
            "src/b/b.ts".into(),
            "src/c/c.ts".into(),
        ],
        edges
            .iter()
            .map(|(from, to)| Edge {
                from: (*from).into(),
                to: (*to).into(),
                ..Default::default()
            })
            .collect(),
        Vec::new(),
        None,
    )
    .unwrap()
}
fn tool(doc: &Document, name: &str, args: Value) -> Value {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "archgraph-mcp-unit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let result = call(doc, &root, name, args, None);
    std::fs::remove_dir_all(root).unwrap();
    result.unwrap()
}
fn cycle() -> Document {
    document(&[("src/a/a.ts", "src/b/b.ts"), ("src/b/b.ts", "src/a/a.ts")])
}
fn validate(name: &str, value: &Value) {
    let schema = schemas().into_iter().find(|s| s.name == name).unwrap();
    let validator = jsonschema::validator_for(&schema.output).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{name}: {errors:?}");
}
#[test]
fn edit_snapshot_arguments_are_validated_before_collection() {
    use archgraph::mcp_tools::validate;
    let id = "a".repeat(64);
    assert!(validate("check_diff", &json!({"snapshot":id})).is_ok());
    for arguments in [
        json!({"snapshot":"../bad"}),
        json!({"snapshot":"A".repeat(64)}),
        json!({"base":"HEAD","snapshot":id}),
    ] {
        assert!(validate("check_diff", &arguments).is_err());
    }
    assert!(
        call(
            &document(&[]),
            Path::new("."),
            "check_diff",
            json!({"snapshot":id}),
            Some(&document(&[]))
        )
        .unwrap_err()
        .to_string()
        .contains("live MCP")
    );
}
#[test]
fn stable_ids_exclude_timing_priority_and_member_order() {
    let doc = cycle();
    let f = doc
        .findings
        .iter()
        .find(|f| f.kind.as_str() == "cycle")
        .unwrap();
    let mut other = f.clone();
    other.age_days = Some(300);
    other.hotspot_priority = Some(0.98);
    if let archgraph::model::Evidence::Cycle(c) = &mut other.evidence {
        c.members.reverse();
        c.break_set.reverse();
    }
    assert_eq!(finding_id(f), finding_id(&other));
    let out = tool(&doc, "verify", json!({}));
    validate("verify", &out);
    assert_eq!(
        out["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["finding_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        doc.findings
            .iter()
            .map(|f| f.finding_id.as_str())
            .collect::<Vec<_>>()
    );
}
#[test]
fn cycle_recipe_contains_executable_predicate_and_no_invented_cut_delta() {
    let doc = cycle();
    let out = tool(&doc, "verify", json!({}));
    let id = out["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "cycle")
        .unwrap()["finding_id"]
        .clone();
    let recipe = tool(&doc, "propose", json!({"finding_id":id}));
    validate("propose", &recipe);
    let step = &recipe["steps"][0];
    assert_eq!(step["kind"], "cut_import");
    // The graph fixture has no source import ledger: never invent mechanical deltas.
    assert!(recipe["expected"]["pairs_removed"].is_null());
    let predicate = json!({"findings_absent":[id]});
    let failed = tool(&doc, "verify", json!({"expect":predicate}));
    assert_eq!(failed["pass"], false);
    let clean = document(&[]);
    let passed = tool(&clean, "verify", json!({"expect":predicate}));
    assert_eq!(passed["pass"], true);
}
#[test]
fn diff_distinguishes_grown_cycles_and_schemas_validate() {
    let old = cycle();
    let new = document(&[
        ("src/a/a.ts", "src/b/b.ts"),
        ("src/b/b.ts", "src/a/a.ts"),
        ("src/b/b.ts", "src/c/c.ts"),
        ("src/c/c.ts", "src/a/a.ts"),
    ]);
    let out = call(
        &new,
        Path::new("."),
        "check_diff",
        json!({"base":"HEAD"}),
        Some(&old),
    )
    .unwrap();
    assert_eq!(out["grown_cycles"].as_array().unwrap().len(), 1);
    validate("check_diff", &out);
    let output = tool(
        &old,
        "simulate",
        json!({"moves":[{"from":"src/b/b.ts","to":"src/a/b.ts"}]}),
    );
    assert_eq!(output["before"]["cycles"], 1);
    assert_eq!(output["after"]["cycles"], 1);
    assert_eq!(output["new_cycles"][0]["level"], "file");
    assert_eq!(output["pairs_removed"], 1);
    validate("simulate", &output);
    let explanation = tool(&old, "explain", json!({"module":"a"}));
    validate("explain", &explanation);
    let rules = tool(&old, "rules", json!({"paths":["src/a/a.ts"]}));
    validate("rules", &rules);
}
#[test]
fn expectations_fail_closed_for_unknown_subjects_and_invalid_inputs() {
    let doc = cycle();
    for args in [
        json!({"expect":{"cycles_in":["typo"]}}),
        json!({"expect":{}}),
        json!({"expect":{"findings_absent":["nonsense"]}}),
        json!({"expect":{"cycles_in":[]}}),
        json!({"expect":{"forbidden":-1}}),
        json!({"unknown":true}),
    ] {
        assert!(call(&doc, Path::new("."), "verify", args, None).is_err());
    }
    assert!(
        call(
            &doc,
            Path::new("."),
            "explain",
            json!({"path":"a","module":"a"}),
            None
        )
        .is_err()
    );
    assert!(
        call(
            &doc,
            Path::new("."),
            "context",
            json!({"task":"x","budget_tokens":0}),
            None
        )
        .is_err()
    );
    let fail = tool(
        &doc,
        "verify",
        json!({"expect":{"cycles_in":["a"],"equals":0}}),
    );
    assert_eq!(fail["pass"], false);
    let pass = tool(
        &doc,
        "verify",
        json!({"expect":{"cycles_in":["a"],"equals":1,"forbidden":0}}),
    );
    assert_eq!(pass["pass"], true);
}
#[test]
fn input_schemas_reject_unknown_fields_and_match_examples() {
    let examples = [
        ("context", json!({"task":"billing","budget_tokens":4000})),
        ("check_diff", json!({})),
        (
            "propose",
            json!({"finding_id":format!("ag-{}","a".repeat(64))}),
        ),
        (
            "simulate",
            json!({"moves":[{"from":"src/a.ts","to":"src/b.ts"}]}),
        ),
        ("verify", json!({"expect":{"cycles_in":["a"],"equals":0}})),
        ("explain", json!({"path":"src/a.ts"})),
        ("rules", json!({"paths":["src/a.ts"]})),
    ];
    for (name, mut args) in examples {
        let schema = schemas().into_iter().find(|s| s.name == name).unwrap();
        let validator = jsonschema::validator_for(&schema.input).unwrap();
        assert!(validator.is_valid(&args), "{name}");
        args["unexpected"] = json!(true);
        assert!(!validator.is_valid(&args), "{name}");
    }
}

#[test]
fn context_uses_existing_budgeted_selection_without_exposing_source_bodies() {
    let root = std::env::temp_dir().join(format!("archgraph-mcp-context-{}", std::process::id()));
    for name in ["a", "b", "c"] {
        let dir = root.join("src").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{name}.ts")),
            "export const secretBody = 123;\n",
        )
        .unwrap();
    }
    let doc = cycle();
    let result = call(
        &doc,
        &root,
        "context",
        json!({"task":"change billing","files":["src/a/a.ts"],"budget_tokens":4000}),
        None,
    )
    .unwrap();
    assert!(!result["entries"].as_array().unwrap().is_empty());
    assert!(
        result["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["content"].is_null())
    );
    assert!(!result.to_string().contains("secretBody"));
    validate("context", &result);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn simulation_file_cycle_renames_preserve_identity_without_hiding_new_cycle_levels() {
    let base = document(&[]);
    let doc = archgraph::analyze(
        base.source,
        base.config,
        vec!["src/a/a.ts".into(), "src/a/other.ts".into()],
        vec![
            Edge {
                from: "src/a/a.ts".into(),
                to: "src/a/other.ts".into(),
                ..Default::default()
            },
            Edge {
                from: "src/a/other.ts".into(),
                to: "src/a/a.ts".into(),
                ..Default::default()
            },
        ],
        Vec::new(),
        None,
    )
    .unwrap();
    assert_eq!(doc.cycles.len(), 1);
    assert_eq!(doc.cycles[0].level, archgraph::model::CycleLevel::File);
    let renamed = tool(
        &doc,
        "simulate",
        json!({"moves":[{"from":"src/a/a.ts","to":"src/a/renamed.ts"},{"from":"src/a/other.ts","to":"src/a/second.ts"}]}),
    );
    assert_eq!(renamed["after"]["cycles"], 1);
    assert_eq!(renamed["new_cycles"], json!([]));
    let split = tool(
        &doc,
        "simulate",
        json!({"moves":[{"from":"src/a/other.ts","to":"src/b/other.ts"}]}),
    );
    assert_eq!(split["new_cycles"][0]["level"], "module");
    validate("simulate", &renamed);
    validate("simulate", &split);
}

#[test]
fn explain_modules_report_all_layers_and_unclassified_members() {
    let base = document(&[]);
    let make = |layers: Vec<&str>| {
        let mut config = base.config.clone();
        config.rules = Some(archgraph::model::RulesRef {
            path: "layout.yaml".into(),
            sha256: "fixture".into(),
            doc: serde_json::from_value(json!({"version":1,"layers":layers})).unwrap(),
        });
        archgraph::analyze(
            base.source.clone(),
            config,
            vec!["src/a/a.ts".into(), "src/a/other.ts".into()],
            Vec::new(),
            Vec::new(),
            None,
        )
        .unwrap()
    };
    let doc = make(vec!["src/a/a.ts", "src/a/other.ts"]);
    let mixed = tool(&doc, "explain", json!({"module":"a"}));
    assert_eq!(mixed["layer"], Value::Null);
    assert_eq!(mixed["layer_status"], "mixed");
    assert_eq!(
        mixed["layers"],
        json!([[0, "src/a/a.ts"], [1, "src/a/other.ts"]])
    );
    validate("explain", &mixed);
    let partially_classified = tool(&make(vec!["src/a/a.ts"]), "explain", json!({"module":"a"}));
    assert_eq!(partially_classified["layer"], Value::Null);
    assert_eq!(partially_classified["layer_status"], "mixed");
    let uniform = tool(&make(vec!["src/a/**"]), "explain", json!({"module":"a"}));
    assert_eq!(uniform["layer"], json!([0, "src/a/**"]));
    assert_eq!(uniform["layer_status"], "uniform");
    let unclassified = tool(&make(Vec::new()), "explain", json!({"module":"a"}));
    assert_eq!(unclassified["layer"], Value::Null);
    assert_eq!(unclassified["layer_status"], "unclassified");
    assert_eq!(unclassified["layers"], json!([]));
}
