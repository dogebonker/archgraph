mod common;

use std::path::{Path, PathBuf};

use archgraph::{
    mcp_tools::{call, finding_id, schemas, services},
    model::{Cochange, CochangePair, Config, Document, Edge, Source},
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
fn with_claim() -> Document {
    let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":3},"git":{"commit":"test","dirty":false},"generated_at":"1970-01-01T00:00:00Z"})).unwrap();
    let config: Config = serde_json::from_value(json!({"depth":1,"source_root":"src","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"fixture","doc":{"version":1,"forbid":[{"id":"no-b","from":"src/a/**","to":"src/b/**","reason":"a never uses b"}]}}})).unwrap();
    archgraph::analyze(
        source,
        config,
        vec!["src/a/a.ts".into(), "src/b/b.ts".into()],
        vec![Edge {
            from: "src/a/a.ts".into(),
            to: "src/b/b.ts".into(),
            ..Default::default()
        }],
        Vec::new(),
        None,
    )
    .unwrap()
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
    assert_eq!(out["new_cyclic_imports"].as_array().unwrap().len(), 2);
    assert_eq!(out["stale_known"], json!([]));
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
        ("review", json!({"base":"HEAD"})),
        (
            "services",
            json!({"manifest":"services.yaml","service":"api"}),
        ),
    ];
    for (name, mut args) in examples {
        let schema = schemas().into_iter().find(|s| s.name == name).unwrap();
        let validator = jsonschema::validator_for(&schema.input).unwrap();
        assert!(validator.is_valid(&args), "{name}");
        args["unexpected"] = json!(true);
        assert!(!validator.is_valid(&args), "{name}");
    }
}

struct ServicesFixture(PathBuf);

impl ServicesFixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "archgraph-services-tool-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = Self(root);
        fixture.write(
            "package.json",
            r#"{"dependencies":{"express":"*","pg":"*"}}"#,
        );
        for id in ["api", "worker", "idle", "other"] {
            fixture.write(&format!("{id}/Dockerfile"), "FROM node\n");
        }
        fixture.write("api/main.ts", "import express from 'express'; import {Pool} from 'pg'; const app = express(); app.get('/jobs', () => {}); new Pool(); fetch('http://worker/status');\n");
        fixture.write("worker/main.ts", "import express from 'express'; const app = express(); app.get('/status', () => {}); fetch('http://api/jobs');\n");
        fixture.write("idle/main.ts", "export const idle = true;\n");
        fixture.write(
            "other/main.ts",
            "import {Pool} from 'pg'; new Pool(); fetch('http://worker/status');\n",
        );
        fixture
    }

    fn write(&self, path: &str, text: &str) {
        let file = self.0.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

impl Drop for ServicesFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn services_tool_returns_live_scoped_evidence_with_report_wide_summary() {
    let fixture = ServicesFixture::new();
    fixture.write("layout.yaml", "not: valid: layout: yaml\n");
    let all = services(&fixture.0, json!({})).unwrap();
    validate("services", &all);
    assert_eq!(all["source"]["collector"], "inferred");
    assert_eq!(all["source"]["complete"], false);
    assert_eq!(all["scope"], json!({"service":null}));
    assert_eq!(all["global_summary"]["services"], 4);
    assert_eq!(all["global_summary"]["calls"], 3);
    assert_eq!(all["global_summary"]["resources"], 2);
    assert_eq!(all["services"].as_array().unwrap().len(), 4);
    assert_eq!(all["calls"].as_array().unwrap().len(), 3);
    assert_eq!(all["resources"][0]["package"], "pg");
    assert!(all.get("schema").is_none());
    assert!(all.get("forbidden_calls").is_none());

    // The unrelated Document does not supply service facts or determine the selected scope.
    let api = call(
        &document(&[]),
        &fixture.0,
        "services",
        json!({"service":"api"}),
        None,
    )
    .unwrap();
    validate("services", &api);
    assert_eq!(api["scope"]["service"], "api");
    assert_eq!(api["global_summary"], all["global_summary"]);
    assert_eq!(api["services"].as_array().unwrap().len(), 1);
    assert_eq!(api["services"][0]["id"], "api");
    assert_eq!(api["services"][0]["fan_in"], 1);
    assert_eq!(api["services"][0]["fan_out"], 1);
    assert_eq!(api["calls"].as_array().unwrap().len(), 2);
    assert!(
        api["calls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["from"] == "api" || edge["to"] == "api")
    );
    assert!(
        api["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource["service"] == "api")
    );
    assert_eq!(api["resources"].as_array().unwrap().len(), 1);

    let idle = services(&fixture.0, json!({"service":"idle","brief":false})).unwrap();
    validate("services", &idle);
    assert_eq!(idle["global_summary"]["calls"], 3);
    assert_eq!(idle["calls"], json!([]));
    assert_eq!(idle["resources"], json!([]));
}

#[test]
fn services_tool_applies_manifest_rules_and_refreshes_after_source_edits() {
    let fixture = ServicesFixture::new();
    fixture.write(
        "policy/services.yaml",
        r#"
version: 1
services: [api, worker, idle]
edges: []
forbid:
  - {id: no-postgres, from: api, to: 'pkg:pg'}
  - {id: no-api, from: worker, to: api}
"#,
    );
    let args = json!({"manifest":"policy/services.yaml","service":"api"});
    let first = services(&fixture.0, args.clone()).unwrap();
    validate("services", &first);
    assert_eq!(first["source"]["collector"], "inferred+declarative");
    assert_eq!(first["global_summary"]["forbidden_calls"], 1);
    assert_eq!(first["global_summary"]["forbidden_resources"], 1);
    assert_eq!(first["resources"][0]["rule"], "no-postgres");
    assert_eq!(first["resources"][0]["verdict"], "forbidden");
    assert!(
        first["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["rule"] == "no-api")
    );
    assert_eq!(first, services(&fixture.0, args.clone()).unwrap());

    fixture.write(
        "worker/main.ts",
        "import express from 'express'; const app = express(); app.get('/status', () => {});\n",
    );
    let fresh = services(&fixture.0, args).unwrap();
    validate("services", &fresh);
    assert_eq!(fresh["global_summary"]["calls"], 1);
    assert_eq!(fresh["global_summary"]["forbidden_calls"], 0);
    assert_eq!(fresh["calls"][0]["from"], "api");
}

#[test]
fn services_arguments_fail_before_collection_and_unknown_services_are_errors() {
    for args in [
        json!({"unknown":true}),
        json!({"brief":"yes"}),
        json!({"service":""}),
        json!({"service":"*"}),
        json!({"service":"../api"}),
        json!({"service":"api worker"}),
        json!({"service":"api\n"}),
        json!({"manifest":""}),
        json!({"manifest":"../outside.yaml"}),
        json!({"manifest":"/outside.yaml"}),
        json!({"manifest":"a/../services.yaml"}),
        json!({"manifest":true}),
    ] {
        assert!(
            archgraph::mcp_tools::validate("services", &args).is_err(),
            "{args}"
        );
        let error = services(Path::new("/missing-mcp-services-root"), args).unwrap_err();
        assert!(
            !error
                .to_string()
                .contains("resolving services repository root")
        );
    }
    let fixture = ServicesFixture::new();
    assert!(
        services(&fixture.0, json!({"service":"missing"}))
            .unwrap_err()
            .to_string()
            .contains("unknown service")
    );
    assert!(
        services(&fixture.0, json!({"manifest":"api"}))
            .unwrap_err()
            .to_string()
            .contains("file inside")
    );
    assert!(
        services(&fixture.0.join("api/main.ts"), json!({}))
            .unwrap_err()
            .to_string()
            .contains("must be a directory")
    );
    let nullable = services(&fixture.0, json!({"manifest":null,"service":null})).unwrap();
    validate("services", &nullable);
    let schema = schemas()
        .into_iter()
        .find(|schema| schema.name == "services")
        .unwrap();
    let validator = jsonschema::validator_for(&schema.input).unwrap();
    assert!(validator.is_valid(&json!({"manifest":null,"service":null})));
}

#[cfg(unix)]
#[test]
fn services_manifest_symlinks_cannot_escape_the_repository() {
    use std::os::unix::fs::symlink;
    let fixture = ServicesFixture::new();
    let outside = ServicesFixture::new();
    outside.write("external.yaml", "version: 1\nservices: []\nedges: []\n");
    symlink(
        outside.0.join("external.yaml"),
        fixture.0.join("escape.yaml"),
    )
    .unwrap();
    assert!(
        services(&fixture.0, json!({"manifest":"escape.yaml"}))
            .unwrap_err()
            .to_string()
            .contains("file inside")
    );
    assert_eq!(
        std::fs::read_to_string(outside.0.join("external.yaml")).unwrap(),
        "version: 1\nservices: []\nedges: []\n"
    );
    fixture.write(
        "inside.yaml",
        "version: 1\nservices: [api, worker, idle]\nedges: []\n",
    );
    symlink(fixture.0.join("inside.yaml"), fixture.0.join("alias.yaml")).unwrap();
    validate(
        "services",
        &services(&fixture.0, json!({"manifest":"alias.yaml"})).unwrap(),
    );
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
            known: None,
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

#[test]
fn target_returns_claim_verdicts_or_null() {
    let with = tool(&with_claim(), "target", json!({}));
    assert_eq!(with["claims"][0]["verdict"], "broken");
    validate("target", &with);
    let without = tool(&cycle(), "target", json!({}));
    assert_eq!(without, Value::Null);
    validate("target", &without);
}

/// Slices a and b under the claim app-no-domain, a leaf rule on shared/ and `(from, to, line)`
/// imports.
fn claims(edges: &[(&str, &str, usize)]) -> Document {
    let files = [
        "src/m/a/app/x.ts",
        "src/m/a/domain/d.ts",
        "src/m/b/app/y.ts",
        "src/m/b/domain/e.ts",
        "src/shared/id.ts",
    ];
    let edges = edges.iter().map(|(from, to, line)| Edge {
        from: (*from).into(),
        to: (*to).into(),
        line: Some(*line),
        ..Default::default()
    });
    claims_in(files.map(String::from).to_vec(), edges.collect())
}

/// `files` and `edges` under the rules of `claims`.
fn claims_in(files: Vec<String>, edges: Vec<Edge>) -> Document {
    claims_with(
        json!([
        {"id":"app-no-domain","from":"src/m/<name>/app/**","to":"src/m/<name>/domain/**","reason":"apps never reach a domain","source":"docs/adr/0001.md"},
        {"from":"src/shared/**","to":"src/m/**","reason":"shared stays a leaf"}]),
        files,
        edges,
    )
}

fn claims_with(forbid: Value, files: Vec<String>, edges: Vec<Edge>) -> Document {
    let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":5},"git":{"commit":"test","dirty":false},"generated_at":"1970-01-01T00:00:00Z"})).unwrap();
    let config: Config = serde_json::from_value(json!({"depth":1,"source_root":"src","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"fixture","doc":{"version":1,"forbid":forbid}}})).unwrap();
    archgraph::analyze(source, config, files, edges, Vec::new(), None).unwrap()
}

/// `doc` with git co-change `(a, b, commits)` pairs and its target recomputed.
fn with_cochange(mut doc: Document, pairs: &[(&str, &str, usize)]) -> Document {
    doc.cochange = Some(Cochange {
        limit: 300,
        commits: 10,
        skipped: 0,
        pairs: pairs
            .iter()
            .map(|(a, b, commits)| CochangePair {
                files: [(*a).into(), (*b).into()],
                commits: *commits,
            })
            .collect(),
    });
    let rules = archgraph::rules::compile(&doc.config).unwrap().unwrap();
    doc.target = archgraph::target::analyze(&doc, &rules);
    doc
}

#[test]
fn target_lists_files_changed_together_across_a_claim_without_an_import() {
    let (x, y) = ("src/m/a/app/x.ts", "src/m/b/app/y.ts");
    let (d, e) = ("src/m/a/domain/d.ts", "src/m/b/domain/e.ts");
    let doc = with_cochange(
        claims(&[(x, d, 2)]),
        &[(x, d, 4), (y, e, 3), (x, e, 5), (y, "src/shared/id.ts", 3)],
    );
    let target = doc.target.as_ref().unwrap();
    let slices = serde_json::to_value(&target.claims[0].slices).unwrap();
    // x and d import each other; x and e are in different slices; shared/ is on no claim side.
    assert_eq!(slices[0]["slice"], "a");
    assert_eq!(slices[0].get("cochanged"), None);
    assert_eq!(
        slices[1]["cochanged"],
        json!([{"files": [y, e], "commits": 3}])
    );
    assert_eq!(
        archgraph::target::text(target),
        "claim app-no-domain: holds in 1 of 2 slices; broken in a reason=\"apps never reach a domain\" source=\"docs/adr/0001.md\"
  src/m/a/app/x.ts -> src/m/a/domain/d.ts
  co-change: src/m/b/app/y.ts <-> src/m/b/domain/e.ts, 3 commits, no import
target: 1 of 2 slices conform
"
    );
}

#[test]
fn a_pair_joined_through_another_file_of_the_slice_is_not_listed() {
    // e reaches y through z, as a repository reaches a handler through the service between them.
    let (y, z, e) = (
        "src/m/b/app/y.ts",
        "src/m/b/app/z.ts",
        "src/m/b/domain/e.ts",
    );
    let files = ["src/m/a/app/x.ts", "src/m/a/domain/d.ts", y, z, e]
        .map(String::from)
        .to_vec();
    let edge = |from: &str, to: &str| Edge {
        from: from.into(),
        to: to.into(),
        line: Some(1),
        ..Default::default()
    };
    let doc = with_cochange(claims_in(files, vec![edge(e, z), edge(z, y)]), &[(y, e, 3)]);
    let slices = serde_json::to_value(&doc.target.unwrap().claims[0].slices).unwrap();
    let b = slices
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slice"] == "b")
        .unwrap();
    assert_eq!(b.get("cochanged"), None, "{b}");
}

#[test]
fn diff_and_review_list_pairs_newly_changed_together_across_a_claim() {
    let (y, e) = ("src/m/b/app/y.ts", "src/m/b/domain/e.ts");
    let old = with_cochange(claims(&[(y, e, 3)]), &[(y, e, 3)]);
    let new = with_cochange(claims(&[]), &[(y, e, 3)]);
    let d = archgraph::diff::diff(&old, &new);
    let target = serde_json::to_value(&d.target).unwrap();
    assert_eq!(target["newly_fixed"][0]["slice"], "b");
    assert_eq!(
        target["newly_cochanged"],
        json!([{"claim": "app-no-domain", "reason": "apps never reach a domain",
                "source": "docs/adr/0001.md", "slice": "b", "files": [y, e], "commits": 3}])
    );
    let md = archgraph::diff::markdown(&d, &[]);
    assert!(
        md.contains("\nTarget: 2 of 2 slices conform (was 1 of 2), 1 newly co-changed pair.\n"),
        "{md}"
    );
    assert!(
        md.contains("\n### Changed together across a claim, no import (1)\n\n- app-no-domain, slice b: src/m/b/app/y.ts ↔ src/m/b/domain/e.ts (3 commits)\n"),
        "{md}"
    );
    // The pair is linked in the new scan, and a scan without co-change lists none.
    let reverse = serde_json::to_value(archgraph::diff::diff(&new, &old).target).unwrap();
    assert_eq!(reverse.get("newly_cochanged"), None);
    let unread =
        serde_json::to_value(archgraph::diff::diff(&claims(&[(y, e, 3)]), &new).target).unwrap();
    assert_eq!(unread.get("newly_cochanged"), None);
    let out = call(&new, Path::new("."), "review", json!({}), Some(&old)).unwrap();
    assert_eq!(
        out["markdown"],
        "**archgraph review**: 0 newly broken slices, 0 other new forbidden imports, 1 newly fixed slice.
Target: 2 of 2 slices conform (was 1 of 2).

### Newly fixed (1)

- **app-no-domain**: apps never reach a domain (docs/adr/0001.md)
  - slice b: src/m/b/app/y.ts:3 → src/m/b/domain/e.ts

### Changed together across a claim, no import (1)

- app-no-domain, slice b: src/m/b/app/y.ts ↔ src/m/b/domain/e.ts (3 commits)
"
    );
    assert_eq!(out["newly_cochanged"], target["newly_cochanged"]);
    validate("review", &out);
}

#[test]
fn diff_target_lists_newly_broken_and_newly_fixed_slices_with_lines() {
    let old = claims(&[("src/m/a/app/x.ts", "src/m/a/domain/d.ts", 2)]);
    let new = claims(&[
        ("src/m/b/app/y.ts", "src/m/b/domain/e.ts", 3),
        ("src/shared/id.ts", "src/m/a/domain/d.ts", 1),
    ]);
    let slice = |slice: &str, from: &str, to: &str, line: usize| {
        json!([{"claim": "app-no-domain", "reason": "apps never reach a domain",
                "source": "docs/adr/0001.md", "slice": slice,
                "imports": [{"from": from, "to": to, "line": line}]}])
    };
    assert_eq!(
        serde_json::to_value(archgraph::diff::diff(&old, &new).target).unwrap(),
        json!({
            "conforming": [{"done": 1, "of": 2}, {"done": 1, "of": 2}],
            "newly_broken": slice("b", "src/m/b/app/y.ts", "src/m/b/domain/e.ts", 3),
            "newly_fixed": slice("a", "src/m/a/app/x.ts", "src/m/a/domain/d.ts", 2),
        })
    );
    // Slice a broken in old and gone in new counts as fixed.
    let gone = claims_in(
        vec!["src/m/b/app/y.ts".into(), "src/m/b/domain/e.ts".into()],
        Vec::new(),
    );
    let target = archgraph::diff::diff(&old, &gone).target.unwrap();
    assert_eq!(
        serde_json::to_value(target.newly_fixed).unwrap(),
        slice("a", "src/m/a/app/x.ts", "src/m/a/domain/d.ts", 2)
    );
}

#[test]
fn review_quotes_each_newly_broken_other_forbidden_and_newly_fixed_import() {
    let old = claims(&[("src/m/a/app/x.ts", "src/m/a/domain/d.ts", 2)]);
    let new = claims(&[
        ("src/m/b/app/y.ts", "src/m/b/domain/e.ts", 3),
        ("src/shared/id.ts", "src/m/a/domain/d.ts", 1),
    ]);
    let out = call(&new, Path::new("."), "review", json!({}), Some(&old)).unwrap();
    assert_eq!(
        out["markdown"],
        "**archgraph review**: 1 newly broken slice, 1 other new forbidden import, 1 newly fixed slice.
Target: 1 of 2 slices conform.

### Newly broken (1)

- **app-no-domain**: apps never reach a domain (docs/adr/0001.md)
  - slice b: src/m/b/app/y.ts:3 → src/m/b/domain/e.ts

### Other new forbidden imports (1)

- src/shared/id.ts:1 → src/m/a/domain/d.ts, forbid\\[1\\]: shared stays a leaf

### Newly fixed (1)

- **app-no-domain**: apps never reach a domain (docs/adr/0001.md)
  - slice a: src/m/a/app/x.ts:2 → src/m/a/domain/d.ts
"
    );
    validate("review", &out);
    let plain = cycle();
    let out = call(&plain, Path::new("."), "review", json!({}), Some(&plain)).unwrap();
    assert_eq!(
        out["markdown"],
        "**archgraph review**: 0 newly broken slices, 0 other new forbidden imports, 0 newly fixed slices.
No rules: nothing was checked; add layout.yaml or pass --rules.
"
    );
    validate("review", &out);
}

#[test]
fn review_lists_a_claim_the_change_makes_vacuous() {
    let old = claims(&[]);
    // Neither app is left, so app-no-domain's `from` matches no file.
    let files = vec![
        "src/m/a/domain/d.ts".into(),
        "src/m/b/domain/e.ts".into(),
        "src/shared/id.ts".into(),
    ];
    let new = claims_in(files.clone(), Vec::new());
    let out = call(&new, Path::new("."), "review", json!({}), Some(&old)).unwrap();
    assert_eq!(
        out["markdown"],
        "**archgraph review**: 0 newly broken slices, 0 other new forbidden imports, 0 newly fixed slices.
Target: 0 of 0 slices conform (was 2 of 2).

### Newly vacuous claims (1)

- **app-no-domain**: apps never reach a domain (docs/adr/0001.md), forbid[0] `from` matches no file
"
    );
    assert_eq!(
        out["newly_vacuous"],
        json!([{"claim": "app-no-domain", "reason": "apps never reach a domain",
                "source": "docs/adr/0001.md", "notes": ["forbid[0] `from` matches no file"]}])
    );
    validate("review", &out);
    // A claim the base does not state never counts.
    let unclaimed = claims_with(
        json!([{"from":"src/shared/**","to":"src/m/**","reason":"shared stays a leaf"}]),
        files,
        Vec::new(),
    );
    let target = archgraph::diff::diff(&unclaimed, &new).target.unwrap();
    assert!(target.newly_vacuous.is_empty());
}

#[test]
fn review_marks_known_other_forbidden_imports_and_counts_stale_entries() {
    let files = [
        "src/m/a/domain/d.ts",
        "src/shared/id.ts",
        "src/shared/ids.ts",
    ];
    let edges = vec![
        Edge {
            from: "src/shared/id.ts".into(),
            to: "src/m/a/domain/d.ts".into(),
            line: Some(1),
            ..Default::default()
        },
        Edge {
            from: "src/shared/ids.ts".into(),
            to: "src/m/a/domain/d.ts".into(),
            line: Some(2),
            ..Default::default()
        },
    ];
    let build = |edges: Vec<Edge>| {
        let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":3},"git":{"commit":"test","dirty":false},"generated_at":"1970-01-01T00:00:00Z"})).unwrap();
        let known = |from: &str, to: &str| json!({"from": from, "to": to, "rule": "forbid[0]"});
        let config: Config = serde_json::from_value(json!({"depth":1,"source_root":"src","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"fixture",
        "doc":{"version":1,"forbid":[{"from":"src/shared/**","to":"src/m/**","reason":"shared stays a leaf"}]},
        "known":{"path":"layout.known.yaml","entries":[
            known("src/gone.ts", "src/m/a/domain/d.ts"),
            known("src/shared/id.ts", "src/m/a/domain/d.ts")]}}})).unwrap();
        archgraph::analyze(
            source,
            config,
            files.map(String::from).to_vec(),
            edges,
            Vec::new(),
            None,
        )
        .unwrap()
    };
    let (old, new) = (build(Vec::new()), build(edges));
    let out = call(&new, Path::new("."), "review", json!({}), Some(&old)).unwrap();
    assert_eq!(
        out["markdown"],
        "**archgraph review**: 0 newly broken slices, 2 other new forbidden imports, 0 newly fixed slices.

### Other new forbidden imports (2)

- src/shared/id.ts:1 → src/m/a/domain/d.ts, forbid\\[0\\]: shared stays a leaf, listed in layout.known.yaml
- src/shared/ids.ts:2 → src/m/a/domain/d.ts, forbid\\[0\\]: shared stays a leaf

1 known entry no longer occurs: run archgraph check --update-known shrink.
"
    );
    let rows = out["other_forbidden"].as_array().unwrap();
    assert_eq!(rows[0]["known"], true);
    assert!(rows[1].get("known").is_none());
    validate("review", &out);
}

#[test]
fn sarif_puts_each_broken_claim_and_other_forbidden_import_on_its_line() {
    let new = claims(&[
        ("src/m/b/app/y.ts", "src/m/b/domain/e.ts", 3),
        ("src/shared/id.ts", "src/m/a/domain/d.ts", 1),
    ]);
    let out = archgraph::sarif::sarif(&new);
    common::validate_sarif(&out);
    let run = &out["runs"][0];
    assert_eq!(
        run["tool"]["driver"]["rules"],
        json!([
            {"id": "app-no-domain", "shortDescription": {"text": "apps never reach a domain"},
             "fullDescription": {"text": "apps never reach a domain"},
             "help": {"text": "Source: docs/adr/0001.md"},
             "defaultConfiguration": {"level": "error"}, "properties": {"tags": ["architecture"]}},
            {"id": "forbid[1]", "shortDescription": {"text": "shared stays a leaf"},
             "fullDescription": {"text": "shared stays a leaf"},
             "help": {"text": "shared stays a leaf"},
             "defaultConfiguration": {"level": "error"}, "properties": {"tags": ["architecture"]}},
        ])
    );
    let at = |uri: &str, line: usize| {
        json!([{"physicalLocation": {"artifactLocation": {"uri": uri, "uriBaseId": "%SRCROOT%"},
                                     "region": {"startLine": line}}}])
    };
    assert_eq!(
        run["results"],
        json!([
            {"ruleId": "app-no-domain", "level": "error",
             "message": {"text": "src/m/b/app/y.ts imports src/m/b/domain/e.ts, breaking claim app-no-domain in slice b: apps never reach a domain (docs/adr/0001.md)."},
             "locations": at("src/m/b/app/y.ts", 3),
             "partialFingerprints": {"primaryLocationLineHash": "bef3e88717e6b19c6929f9300f74b5d5"}},
            {"ruleId": "forbid[1]", "level": "error",
             "message": {"text": "src/shared/id.ts imports src/m/a/domain/d.ts, forbidden by forbid[1]: shared stays a leaf."},
             "locations": at("src/shared/id.ts", 1),
             "partialFingerprints": {"primaryLocationLineHash": "74053824541e62736c72f6c88aeef584"}},
        ])
    );

    // No line: the whole file under the scan root, and the message says why.
    let mut lineless = claims_in(
        vec!["src/m/b/app/y.ts".into(), "src/m/b/domain/e.ts".into()],
        vec![Edge {
            from: "src/m/b/app/y.ts".into(),
            to: "src/m/b/domain/e.ts".into(),
            ..Default::default()
        }],
    );
    lineless.source.scan_root = Some("web".into());
    let out = archgraph::sarif::sarif(&lineless);
    common::validate_sarif(&out);
    let results = out["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["locations"],
        json!([{"physicalLocation": {"artifactLocation": {"uri": "web/src/m/b/app/y.ts", "uriBaseId": "%SRCROOT%"}}}])
    );
    assert_eq!(
        results[0]["message"]["text"],
        "src/m/b/app/y.ts imports src/m/b/domain/e.ts, breaking claim app-no-domain in slice b: apps never reach a domain (docs/adr/0001.md). Line unknown: the fixture collector records none."
    );
}

#[test]
fn sarif_reports_a_link_once_per_claim_id_and_does_not_double_a_closing_period() {
    // Two rules share an id: the link is in slice `b` and in slice `*`.
    let doc = claims_with(
        json!([
            {"id":"app-no-domain","from":"src/m/<name>/app/**","to":"src/m/<name>/domain/**","reason":"apps never reach a domain."},
            {"id":"app-no-domain","from":"src/m/b/app/**","to":"src/m/b/domain/**","reason":"apps never reach a domain."}]),
        vec!["src/m/b/app/y.ts".into(), "src/m/b/domain/e.ts".into()],
        vec![Edge {
            from: "src/m/b/app/y.ts".into(),
            to: "src/m/b/domain/e.ts".into(),
            line: Some(3),
            ..Default::default()
        }],
    );
    let out = archgraph::sarif::sarif(&doc);
    common::validate_sarif(&out);
    let results = out["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let prints: std::collections::BTreeSet<_> = results
        .iter()
        .map(|r| r["partialFingerprints"]["primaryLocationLineHash"].to_string())
        .collect();
    assert_eq!(prints.len(), results.len());
    assert_eq!(
        results[0]["message"]["text"],
        "src/m/b/app/y.ts imports src/m/b/domain/e.ts, breaking claim app-no-domain in slice b: apps never reach a domain."
    );
    assert_eq!(
        results[0]["partialFingerprints"]["primaryLocationLineHash"],
        "bef3e88717e6b19c6929f9300f74b5d5"
    );

    // A forbidden import whose reason ends in a period.
    let doc = claims_with(
        json!([{"from":"src/shared/**","to":"src/m/**","reason":"shared stays a leaf."}]),
        vec!["src/shared/id.ts".into(), "src/m/a/domain/d.ts".into()],
        vec![Edge {
            from: "src/shared/id.ts".into(),
            to: "src/m/a/domain/d.ts".into(),
            line: Some(1),
            ..Default::default()
        }],
    );
    let out = archgraph::sarif::sarif(&doc);
    assert_eq!(
        out["runs"][0]["results"][0]["message"]["text"],
        "src/shared/id.ts imports src/m/a/domain/d.ts, forbidden by forbid[0]: shared stays a leaf."
    );
}

#[test]
fn review_markdown_keeps_true_totals_within_its_budget() {
    let slices: Vec<String> = (0..40).map(|i| format!("s{i:02}")).collect();
    let files = slices
        .iter()
        .flat_map(|s| {
            [
                format!("src/m/{s}/app/x.ts"),
                format!("src/m/{s}/domain/d.ts"),
            ]
        })
        .collect::<Vec<_>>();
    let old = claims_in(files.clone(), Vec::new());
    let edges = slices
        .iter()
        .map(|s| Edge {
            from: format!("src/m/{s}/app/x.ts"),
            to: format!("src/m/{s}/domain/d.ts"),
            ..Default::default()
        })
        .collect();
    let out = call(
        &claims_in(files, edges),
        Path::new("."),
        "review",
        json!({}),
        Some(&old),
    )
    .unwrap();
    let md = out["markdown"].as_str().unwrap();
    assert!(md.len() <= 1300, "{}", md.len());
    assert!(md.contains("\n### Newly broken (40)\n"), "{md}");
    assert!(
        md.ends_with(
            "\n\n25 more rows in `archgraph review --format json`.
Lines unknown for 40 imports: the fixture collector records none.
"
        ),
        "{md}"
    );
    assert_eq!(md.matches("\n  - slice ").count(), 40 - 25);
    assert_eq!(out["newly_broken"].as_array().unwrap().len(), 40);
    validate("review", &out);
}
