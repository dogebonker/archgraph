use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "services-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_archgraph"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    fn save(&self, name: &str, doc: &Value) {
        fs::write(self.0.join(name), serde_json::to_vec(doc).unwrap()).unwrap();
    }
    fn report(&self, doc: &Value) -> Value {
        self.save("services.json", doc);
        let output = self.run(&["services", "services.json", "--format", "json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        validate(&report);
        report
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn validate(report: &Value) {
    let schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.1.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(report)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

fn edge(from: &str, to: &str, kind: &str) -> Value {
    json!({"from":from,"to":to,"kind":kind,
        "source":{"file":"src/calls.ts","line":7}})
}
fn fixture() -> Value {
    json!({"version":1,"services":["api","billing","worker","database"],
        "edges":[edge("api","billing","http"),edge("billing","api","grpc"),
            edge("billing","worker","queue"),edge("api","database","http"),
            edge("worker","database","grpc")]})
}
fn module<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == name)
        .unwrap()
}

#[test]
fn mixed_transports_preserve_evidence_and_group_the_service_cycle() {
    let scratch = Scratch::new();
    let report = scratch.report(&fixture());
    assert_eq!(report["schema"], "archgraph/v1.1");
    assert_eq!(report["kind"], "services");
    assert_eq!(report["source"]["collector"], "declarative");
    assert_eq!(report["source"]["complete"], false);
    let cycles = report["cycles"].as_array().unwrap();
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0]["members"], json!(["api", "billing"]));
    let cycle_edges = cycles[0]["edge_indices"].as_array().unwrap();
    assert_eq!(cycle_edges.len(), 2);
    for index in cycle_edges {
        let e = &report["edges"][index.as_u64().unwrap() as usize];
        assert!(matches!(e["kind"].as_str(), Some("http" | "grpc")));
    }
    let queue = report["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "queue")
        .unwrap();
    assert_eq!(queue["from"], "billing");
    assert_eq!(queue["to"], "worker");
    assert_eq!(queue["source"], json!({"file":"src/calls.ts","line":7}));
    assert_eq!(report["hubs"][0]["service"], "database");
    assert!(!report["caveats"].as_array().unwrap().is_empty());
}

#[test]
fn fan_in_counts_distinct_services_and_self_calls_do_not_create_hubs() {
    let scratch = Scratch::new();
    let report = scratch.report(&json!({"version":1,"services":["a","platform/b","isolated"],
        "edges":[edge("a","platform/b","http"),edge("a","platform/b","grpc"),edge("platform/b","platform/b","queue")]}));
    assert_eq!(module(&report, "platform/b")["fan_in"], 1);
    assert_eq!(module(&report, "a")["fan_out"], 1);
    assert_eq!(module(&report, "isolated")["fan_in"], 0);
    assert!(module(&report, "isolated")["instability"].is_null());
    assert!(report["hubs"].as_array().unwrap().is_empty());
    assert!(report["cycles"].as_array().unwrap().is_empty());
}

#[test]
fn forbidden_calls_use_transport_filters_globs_and_first_matching_rule() {
    let scratch = Scratch::new();
    let mut doc = fixture();
    doc["edges"]
        .as_array_mut()
        .unwrap()
        .push(edge("api", "worker", "grpc"));
    doc["forbid"] = json!([
        {"id":"http-only","from":"api","to":"*","kinds":["http"],"reason":"boundary"},
        {"id":"all-api","from":"api","to":"*"},
        {"id":"no-grpc","from":"*","to":"api","kinds":["queue"]}
    ]);
    let report = scratch.report(&doc);
    assert_eq!(report["forbidden_calls"].as_array().unwrap().len(), 3);
    for e in report["edges"].as_array().unwrap() {
        if e["from"] == "api" {
            assert_eq!(e["verdict"], "forbidden");
            assert_eq!(
                e["rule"],
                if e["kind"] == "http" {
                    "http-only"
                } else {
                    "all-api"
                }
            );
        } else {
            assert_eq!(e["verdict"], "allowed");
        }
    }
}

#[test]
fn yaml_json_default_input_and_output_are_deterministic_and_read_only() {
    let scratch = Scratch::new();
    let yaml = include_str!("../examples/services.yaml");
    fs::write(scratch.0.join("services.yaml"), yaml).unwrap();
    let markdown = scratch.run(&["services"]);
    assert!(
        markdown.status.success(),
        "{}",
        String::from_utf8_lossy(&markdown.stderr)
    );
    let text = String::from_utf8(markdown.stdout).unwrap();
    assert!(text.contains("frontend-bff") && text.contains("db-service"));
    let output = scratch.run(&["services", "--format", "json", "-o", "report.json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let yaml_report: Value =
        serde_json::from_slice(&fs::read(scratch.0.join("report.json")).unwrap()).unwrap();
    validate(&yaml_report);
    let doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
    scratch.save("input.json", &doc);
    let json_output = scratch.run(&["services", "input.json", "--format", "json"]);
    assert!(json_output.status.success());
    let mut json_report: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    json_report["source"]["path"] = yaml_report["source"]["path"].clone();
    assert_eq!(json_report, yaml_report);
    let repeated = scratch.run(&["services", "input.json", "--format", "json"]);
    assert_eq!(repeated.stdout, json_output.stdout);
    assert_eq!(
        fs::read_to_string(scratch.0.join("services.yaml")).unwrap(),
        yaml
    );
    let mut files: Vec<_> = fs::read_dir(&scratch.0)
        .unwrap()
        .map(|f| f.unwrap().file_name())
        .collect();
    files.sort();
    assert_eq!(files, ["input.json", "report.json", "services.yaml"]);
}

#[test]
fn default_confidence_is_low_and_cycle_confidence_is_the_weakest_edge() {
    let scratch = Scratch::new();
    let mut doc = fixture();
    doc["edges"][0]["confidence"] = json!("high");
    doc["edges"][1]["confidence"] = json!("medium");
    let report = scratch.report(&doc);
    assert_eq!(report["cycles"][0]["confidence"], "medium");
    assert_eq!(report["edges"][2]["confidence"], "low");
    doc["edges"][1]
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    assert_eq!(scratch.report(&doc)["cycles"][0]["confidence"], "low");
}

#[test]
fn malformed_inputs_are_rejected_with_exit_two() {
    let scratch = Scratch::new();
    let base = fixture();
    let mut cases = Vec::new();
    for (key, value) in [
        ("version", json!(2)),
        ("unknown", json!(true)),
        ("services", json!(["api", "api"])),
    ] {
        let mut doc = base.clone();
        doc[key] = value;
        cases.push(doc);
    }
    for id in [
        "", "../api", "/api", "a//b", "a/./b", "a\\b", "a*b", "api\n", "<api>",
    ] {
        let mut doc = base.clone();
        doc["services"][0] = json!(id);
        cases.push(doc);
    }
    for (field, value) in [
        ("to", json!("missing")),
        ("kind", json!("runtime")),
        ("confidence", json!("certain")),
        ("extra", json!(1)),
        ("operation", json!("GET /api\nspoofed")),
        ("source", json!({"file":"calls.ts","line":0})),
        ("source", json!({"file":"../calls.ts","line":1})),
    ] {
        let mut doc = base.clone();
        doc["edges"][0][field] = value;
        cases.push(doc);
    }
    let mut doc = base.clone();
    doc["edges"][0].as_object_mut().unwrap().remove("source");
    cases.push(doc);
    let mut doc = base.clone();
    let duplicate = doc["edges"][0].clone();
    doc["edges"].as_array_mut().unwrap().push(duplicate);
    cases.push(doc);
    for forbid in [
        json!([{"id":"bad","from":"[","to":"*"}]),
        json!([{"id":"bad","from":"<name>","to":"*"}]),
        json!([{"id":"bad","from":"api","to":"*","reason":"bad\treason"}]),
        json!([{"id":"same","from":"*","to":"*"},{"id":"same","from":"api","to":"billing"}]),
    ] {
        let mut doc = base.clone();
        doc["forbid"] = forbid;
        cases.push(doc);
    }
    for (index, doc) in cases.iter().enumerate() {
        scratch.save("invalid.json", doc);
        let output = scratch.run(&["services", "invalid.json", "--format", "json"]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "case {index}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn legacy_json_is_rejected_before_overwriting_an_output() {
    let scratch = Scratch::new();
    scratch.save("services.json", &fixture());
    fs::write(scratch.0.join("report.json"), "keep me").unwrap();
    for args in [
        vec![
            "--legacy-v1",
            "services",
            "services.json",
            "--format",
            "json",
        ],
        vec![
            "--legacy-v1",
            "services",
            "services.json",
            "-o",
            "report.json",
        ],
    ] {
        let output = scratch.run(&args);
        assert_eq!(output.status.code(), Some(2));
    }
    assert_eq!(
        fs::read_to_string(scratch.0.join("report.json")).unwrap(),
        "keep me"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(scratch.0.join("services.json")).unwrap())
            .unwrap(),
        fixture()
    );
}

#[test]
fn report_output_cannot_replace_its_input_and_invalid_names_are_escaped_in_errors() {
    let scratch = Scratch::new();
    scratch.save("services.json", &fixture());
    let input = fs::read(scratch.0.join("services.json")).unwrap();
    let output = scratch.run(&["services", "services.json", "-o", "./services.json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(scratch.0.join("services.json")).unwrap(), input);
    let mut doc = fixture();
    doc["edges"][0]["from"] = json!("unknown\u{1b}[31m");
    scratch.save("invalid.json", &doc);
    let output = scratch.run(&["services", "invalid.json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn manifest_and_report_schemas_reject_blank_policy_globs() {
    let manifest_schema: Value =
        serde_json::from_str(include_str!("../schema/services.v1.schema.json")).unwrap();
    let report_schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.1.schema.json")).unwrap();
    jsonschema::draft202012::meta::validate(&manifest_schema).unwrap();
    let manifest_validator = jsonschema::validator_for(&manifest_schema).unwrap();
    let report_validator = jsonschema::validator_for(&report_schema).unwrap();
    let scratch = Scratch::new();
    let mut doc = fixture();
    doc["forbid"] = json!([{"id":"valid","from":"api","to":"database"}]);
    assert!(manifest_validator.is_valid(&doc));
    let mut report = scratch.report(&doc);
    for field in ["from", "to"] {
        let mut invalid = doc.clone();
        invalid["forbid"][0][field] = json!("   ");
        assert!(!manifest_validator.is_valid(&invalid));
        report["forbid"][0][field] = json!("   ");
        assert!(!report_validator.is_valid(&report));
    }
}
