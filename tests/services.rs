use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "services-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
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
        serde_json::from_str(include_str!("../schema/boundrail.v1.2.schema.json")).unwrap();
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
fn forbidden_gate_preserves_reports_and_saved_json_before_exiting_one() {
    let scratch = Scratch::new();
    let mut manifest = fixture();
    manifest["forbid"] = json!([{"id":"no-billing","from":"api","to":"billing"}]);
    scratch.save("services.json", &manifest);
    for format in ["json", "markdown"] {
        let normal = scratch.run(&["services", "services.json", "--format", format]);
        let checked = scratch.run(&[
            "services",
            "services.json",
            "--format",
            format,
            "--fail-on-forbidden",
        ]);
        assert_eq!(normal.status.code(), Some(0));
        assert_eq!(checked.status.code(), Some(1));
        assert_eq!(checked.stdout, normal.stdout);
        assert!(checked.stderr.is_empty());
    }
    for index in 0..40 {
        let mut allowed = edge("worker", "database", "grpc");
        allowed["operation"] = json!(format!("method-{index}"));
        manifest["edges"].as_array_mut().unwrap().push(allowed);
    }
    scratch.save("services.json", &manifest);
    let checked = scratch.run(&[
        "--brief",
        "services",
        "services.json",
        "--format",
        "json",
        "--fail-on-forbidden",
        "--output",
        "report.json",
    ]);
    assert_eq!(checked.status.code(), Some(1));
    let brief: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(brief["schema"], "boundrail/brief/v1");
    assert!(!brief["omitted"].as_object().unwrap().is_empty());
    let saved: Value =
        serde_json::from_slice(&fs::read(scratch.0.join("report.json")).unwrap()).unwrap();
    validate(&saved);
    assert_eq!(saved["summary"]["forbidden_calls"], 1);
    assert_eq!(saved["forbid"][0]["id"], "no-billing");
    let failed_write = scratch.run(&[
        "services",
        "services.json",
        "--fail-on-forbidden",
        "--output",
        ".",
    ]);
    assert_eq!(failed_write.status.code(), Some(2));
    assert!(failed_write.stdout.is_empty());
}

#[test]
fn forbidden_gate_checks_calls_and_infrastructure_independently() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM node");
    source(&scratch, "worker/Dockerfile", "FROM node");
    source(
        &scratch,
        "package.json",
        r#"{"dependencies":{"express":"*","pg":"*"}}"#,
    );
    source(
        &scratch,
        "api/main.ts",
        "import express from 'express'; const app = express(); app.get('/x', () => {});\n",
    );
    for (call, resource) in [(false, false), (true, false), (false, true), (true, true)] {
        source(
            &scratch,
            "worker/main.ts",
            &format!(
                "import {{Pool}} from 'pg';\n{}{}",
                if call { "fetch('http://api/x');\n" } else { "" },
                if resource { "new Pool();\n" } else { "" },
            ),
        );
        scratch.save(
            "policy.json",
            &json!({"version":1,"services":["api","worker"],"edges":[],"forbid":[
                {"id":"no-api","from":"worker","to":"api"},
                {"id":"no-pg","from":"worker","to":"pkg:pg"}
            ]}),
        );
        let checked = scratch.run(&[
            "services",
            ".",
            "--manifest",
            "policy.json",
            "--format",
            "json",
            "--fail-on-forbidden",
        ]);
        assert_eq!(
            checked.status.code(),
            Some(i32::from(call || resource)),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
        validate(&report);
        assert_eq!(report["summary"]["forbidden_calls"], usize::from(call));
        assert_eq!(
            report["summary"]
                .get("forbidden_resources")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            u64::from(resource)
        );
        let default = scratch.run(&[
            "services",
            ".",
            "--manifest",
            "policy.json",
            "--format",
            "json",
        ]);
        assert_eq!(default.status.code(), Some(0));
        assert_eq!(default.stdout, checked.stdout);
    }
}

#[test]
fn forbidden_gate_rejects_unconfigured_policy_and_invalid_input() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM node");
    source(&scratch, "api/main.ts", "export const api = true;\n");
    for input in [".", "services.json"] {
        scratch.save("services.json", &fixture());
        let default = scratch.run(&["services", input, "--format", "json"]);
        assert_eq!(default.status.code(), Some(0));
        let checked = scratch.run(&["services", input, "--fail-on-forbidden"]);
        assert_eq!(checked.status.code(), Some(2));
        assert!(checked.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&checked.stderr).contains("requires authored forbid rules")
        );
    }
    source(&scratch, "invalid.yaml", "invalid: [");
    let invalid = scratch.run(&["services", "invalid.yaml", "--fail-on-forbidden"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
}

#[test]
fn mixed_transports_preserve_evidence_and_group_the_service_cycle() {
    let scratch = Scratch::new();
    let report = scratch.report(&fixture());
    assert_eq!(report["schema"], "boundrail/v1.2");
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
        serde_json::from_str(include_str!("../schema/boundrail.v1.2.schema.json")).unwrap();
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

fn source(scratch: &Scratch, path: &str, contents: &str) {
    let file = scratch.0.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contents).unwrap();
}
fn inferred(scratch: &Scratch, extra: &[&str]) -> Value {
    let mut args = vec!["services", ".", "--format", "json"];
    args.extend_from_slice(extra);
    let output = scratch.run(&args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = serde_json::from_slice(&output.stdout).unwrap();
    validate(&report);
    report
}

#[test]
fn declarative_reports_preserve_pre_inference_json_and_markdown_bytes() {
    let scratch = Scratch::new();
    source(
        &scratch,
        "services.yaml",
        include_str!("../examples/services.yaml"),
    );
    for (format, expected) in [
        (
            "json",
            include_str!("fixtures/services-declarative/report.json"),
        ),
        (
            "markdown",
            include_str!("fixtures/services-declarative/report.md"),
        ),
    ] {
        let output = scratch.run(&["services", "services.yaml", "--format", format]);
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
}

#[test]
fn a_directory_infers_services_and_attaches_infrastructure_clients() {
    let scratch = Scratch::new();
    source(
        &scratch,
        "package.json",
        r#"{"dependencies":{"pg":"*","ioredis":"*","bullmq":"*"}}"#,
    );
    source(&scratch, "src/unused.ts", "export const unrelated = 1;");
    source(&scratch, "api/Dockerfile", "FROM node");
    source(&scratch, "api/Dockerfile.dev", "FROM node");
    source(
        &scratch,
        "api/src/db.ts",
        "import {Pool} from 'pg'; export const db = new Pool();",
    );
    source(
        &scratch,
        "api/src/app.ts",
        "import {cache} from '../../shared/redis'; console.log(cache);",
    );
    source(
        &scratch,
        "shared/redis.ts",
        "import Redis from 'ioredis'; export const cache = new Redis();",
    );
    source(
        &scratch,
        "worker/package.json",
        r#"{"scripts":{"start":"node src/main.js"}}"#,
    );
    source(
        &scratch,
        "worker/src/main.ts",
        "import {Queue} from 'bullmq'; const queue = new Queue('emails');",
    );
    source(
        &scratch,
        "lib/seed.ts",
        "import {Pool} from 'pg'; new Pool();",
    );
    source(
        &scratch,
        "api/src/local.ts",
        "class Pool {} new Pool(); const fetch = () => {}; fetch();",
    );
    source(
        &scratch,
        "api/src/types.d.ts",
        "import {Pool} from 'pg'; new Pool();",
    );
    source(&scratch, "examples/noise/Dockerfile", "FROM node");
    let report = inferred(&scratch, &[]);
    assert_eq!(report["services"], json!(["api", "worker"]));
    assert_eq!(report["source"]["collector"], "inferred");
    let resources = report["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 3, "{resources:#?}");
    assert!(
        resources
            .iter()
            .any(|r| r["service"] == "api" && r["kind"] == "database" && r["confidence"] == "high")
    );
    assert!(
        resources
            .iter()
            .any(|r| r["service"] == "api" && r["kind"] == "cache" && r["confidence"] == "medium")
    );
    assert!(
        resources
            .iter()
            .any(|r| r["service"] == "worker" && r["operation"] == "emails")
    );
    assert!(report["caveats"].as_array().unwrap().iter().any(|v| {
        v.as_str()
            .unwrap()
            .contains("1 application facts are unattributed")
    }));
}

#[test]
fn outbound_calls_and_topics_match_only_one_service_and_preserve_external_calls() {
    let scratch = Scratch::new();
    source(
        &scratch,
        "package.json",
        r#"{"dependencies":{"express":"*","axios":"*","kafkajs":"*"}}"#,
    );
    for service in ["api", "worker", "other"] {
        source(&scratch, &format!("{service}/Dockerfile"), "FROM node");
    }
    source(
        &scratch,
        "api/main.ts",
        r#"
import express from 'express'; import {Kafka} from 'kafkajs';
const app = express(); app.get('/users/:id', () => {}); app.post('/invoices', () => {}); app.get('/health', () => {});
const kafka = new Kafka({}); const producer = kafka.producer(); producer.send({topic:'invoice.created'});
"#,
    );
    source(
        &scratch,
        "other/main.ts",
        "import express from 'express'; const app = express(); app.get('/health', () => {});",
    );
    source(
        &scratch,
        "worker/main.ts",
        r#"
import axios from 'axios'; import {Kafka} from 'kafkajs';
fetch('http://api/users/42'); axios.post('http://api/invoices');
fetch('/health'); fetch('https://api.stripe.com/invoices', {method:'POST'});
const kafka = new Kafka({}); const consumer = kafka.consumer({}); consumer.subscribe({topic:'invoice.created'});
"#,
    );
    let report = inferred(&scratch, &[]);
    let edges = report["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3, "{report:#?}");
    assert!(
        edges
            .iter()
            .any(|e| e["operation"] == "GET /users/:id" && e["confidence"] == "medium")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["operation"] == "POST /invoices" && e["confidence"] == "high")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == "api" && e["to"] == "worker" && e["kind"] == "queue")
    );
    assert_eq!(report["cycles"].as_array().unwrap().len(), 1);
    assert!(
        report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["operation"] == "/invoices")
    );
    assert!(
        report["caveats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().contains("multiple services"))
    );
}

#[test]
fn manifests_override_boundaries_and_judge_clients_separately_from_calls() {
    let scratch = Scratch::new();
    source(&scratch, "Dockerfile", "FROM node");
    source(
        &scratch,
        "package.json",
        r#"{"dependencies":{"express":"*","pg":"*"}}"#,
    );
    source(
        &scratch,
        "api/main.ts",
        "import express from 'express'; const app = express(); app.get('/x', () => {});",
    );
    source(
        &scratch,
        "worker/main.ts",
        "import {Pool} from 'pg'; new Pool(); fetch('http://api/x');",
    );
    scratch.save("override.json", &json!({"version":1,"services":["api","worker","billing"],"edges":[edge("billing","api","grpc")],"forbid":[
        {"id":"ordinary-service-glob","from":"worker","to":"**","kinds":["queue"]},
        {"id":"transport-only","from":"worker","to":"pkg:pg","kinds":["http"]},
        {"id":"no-pg","from":"worker","to":"pkg:pg"},
        {"id":"no-api","from":"worker","to":"api"}
    ]}));
    let report = inferred(&scratch, &["--manifest", "override.json"]);
    assert_eq!(report["services"], json!(["api", "billing", "worker"]));
    assert_eq!(report["source"]["collector"], "inferred+declarative");
    assert_eq!(report["summary"]["forbidden_calls"], 1);
    assert_eq!(report["summary"]["forbidden_resources"], 1);
    assert_eq!(report["resources"][0]["rule"], "no-pg");
    assert_eq!(report["edges"][0]["confidence"], "low");
    assert_eq!(report["resources"][0]["verdict"], "forbidden");
    let original = fs::read(scratch.0.join("override.json")).unwrap();
    let output = scratch.run(&[
        "services",
        ".",
        "--manifest",
        "override.json",
        "-o",
        "override.json",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(scratch.0.join("override.json")).unwrap(), original);
    assert_eq!(
        scratch
            .run(&["services", "override.json", "--manifest", "override.json"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn automatic_boundaries_split_a_root_image_but_skip_library_build_watchers() {
    let scratch = Scratch::new();
    source(&scratch, "Dockerfile", "FROM node");
    source(
        &scratch,
        "package.json",
        r#"{"scripts":{"start":"node tools/setup.js && npm run dev","dev":"turbo run serve"}}"#,
    );
    source(
        &scratch,
        "tools/setup.js",
        "fetch('https://example.com/setup');",
    );
    for (name, script) in [
        ("api", "tsx src/bootstrap.ts"),
        ("worker", "tsx src/bootstrap.ts"),
        ("engine", "node tools/build.mjs --watch"),
    ] {
        source(
            &scratch,
            &format!("packages/{name}/package.json"),
            &json!({"scripts":{"serve":script}}).to_string(),
        );
        source(
            &scratch,
            &format!("packages/{name}/src/bootstrap.ts"),
            "export const value = 1;",
        );
    }
    source(
        &scratch,
        "packages/library/package.json",
        r#"{"main":"src/index.ts"}"#,
    );
    source(
        &scratch,
        "packages/library/src/index.ts",
        "export class Library {}",
    );
    let report = inferred(&scratch, &[]);
    assert_eq!(
        report["services"],
        json!(["packages/api", "packages/worker"])
    );
}

#[test]
fn go_services_contribute_clients_routes_and_dispatched_http_calls() {
    let scratch = Scratch::new();
    source(&scratch, "go.mod", "module example.com/app\n\ngo 1.23\n");
    source(
        &scratch,
        "cmd/api/main.go",
        r#"package main
import "net/http"
func main() { http.HandleFunc("POST /predict", func(w http.ResponseWriter, r *http.Request) {}); http.ListenAndServe(":8080", nil) }
"#,
    );
    source(
        &scratch,
        "cmd/worker/main.go",
        r#"package main
import ("net/http"; "database/sql"; rdb "github.com/redis/go-redis/v9")
func main() { sql.Open("postgres", "dsn"); rdb.NewClient(nil); req, _ := http.NewRequest("POST", "http://api/predict", nil); http.DefaultClient.Do(req); http.NewRequest("GET", "http://api/unused", nil) }
"#,
    );
    source(
        &scratch,
        "ts/package.json",
        r#"{"scripts":{"start":"node main.js"}}"#,
    );
    source(
        &scratch,
        "ts/main.ts",
        "fetch('http://api/predict', {method:'POST'});",
    );
    let report = inferred(&scratch, &[]);
    assert_eq!(report["services"], json!(["cmd/api", "cmd/worker", "ts"]));
    assert_eq!(report["edges"].as_array().unwrap().len(), 2, "{report:#?}");
    assert!(
        report["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["confidence"] == "high")
    );
    assert_eq!(report["summary"]["resources"], 2);
}

#[test]
fn infrastructure_urls_hide_credentials_queries_and_invalid_control_characters() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM node");
    source(
        &scratch,
        "api/main.ts",
        "fetch('https://user:password@third-party.example/data?api_key=demo-secret#auth'); fetch('//user:password@external.example/path?api_key=demo-secret'); fetch('https://external.example?api_key=/demo-secret'); fetch('HTTP://user:password@external.example/path?secret=demo-secret');",
    );
    source(
        &scratch,
        "go/main.go",
        "package main\nimport \"net/http\"\nfunc main(){http.Get(\"http://external/x\\n\")}\n",
    );
    let report = inferred(&scratch, &[]);
    let encoded = report.to_string();
    assert!(
        !encoded.contains("password")
            && !encoded.contains("demo-secret")
            && !encoded.contains("#auth")
    );
    assert!(
        report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["operation"] == "/data")
    );
    assert!(
        report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["service"] == "go" && r.get("operation").is_none())
    );
}

#[test]
fn a_nested_command_does_not_remove_a_root_images_infrastructure_evidence() {
    for path in ["src/db.ts", "db.ts", "lib/db.ts"] {
        let scratch = Scratch::new();
        source(&scratch, "Dockerfile", "FROM node");
        source(&scratch, "package.json", r#"{"dependencies":{"pg":"*"}}"#);
        source(&scratch, path, "import {Pool} from 'pg'; new Pool();");
        source(
            &scratch,
            "cli/package.json",
            r#"{"scripts":{"start":"node main.js"}}"#,
        );
        source(&scratch, "cli/main.ts", "console.log('command');");
        let report = inferred(&scratch, &[]);
        assert_eq!(report["services"].as_array().unwrap().len(), 2);
        assert!(
            report["resources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "database")
        );
    }
}

#[test]
fn unknown_http_methods_match_any_routes_without_inventing_get() {
    let scratch = Scratch::new();
    source(
        &scratch,
        "api/main.go",
        "package main\nimport \"net/http\"\nfunc main(){http.HandleFunc(\"/x\", nil)}\n",
    );
    source(
        &scratch,
        "worker/main.go",
        "package main\nimport (\"net/http\"; \"os\")\nfunc main(){method := os.Getenv(\"METHOD\"); req, _ := http.NewRequest(method, \"http://api/x\", nil); http.DefaultClient.Do(req)}\n",
    );
    let report = inferred(&scratch, &[]);
    assert_eq!(report["edges"].as_array().unwrap().len(), 1);
    assert_eq!(report["edges"][0]["operation"], "ANY /x");
    assert_eq!(report["edges"][0]["confidence"], "medium");
}

#[test]
fn a_python_service_contributes_routes_clients_topics_and_shared_imports() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM node");
    source(
        &scratch,
        "api/main.ts",
        "fetch('http://ml/predict', {method:'POST'});",
    );
    source(&scratch, "ml/Dockerfile", "FROM python");
    source(
        &scratch,
        "ml/main.py",
        r#"
from fastapi import FastAPI
from sqlalchemy import create_engine
from celery import Celery
from shared.db import cache
app = FastAPI()
engine = create_engine('postgresql://localhost/db')
celery = Celery('ml')
@app.post('/predict')
def predict(): return {}
@celery.task(name='ml.compute')
def compute(): return 1
"#,
    );
    source(
        &scratch,
        "shared/db.py",
        "from redis import Redis\ncache = Redis()\n",
    );
    source(&scratch, "publisher/Dockerfile", "FROM python");
    source(
        &scratch,
        "publisher/main.py",
        "from celery import Celery\nfrom httpx import post\napp = Celery('publisher')\napp.send_task('ml.compute')\npost('http://ml/predict')\n",
    );
    let report = inferred(&scratch, &[]);
    assert_eq!(report["services"], json!(["api", "ml", "publisher"]));
    let edges = report["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3, "{report:#?}");
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == "api" && e["to"] == "ml" && e["confidence"] == "high")
    );
    assert!(edges.iter().any(|e| e["from"] == "publisher"
        && e["kind"] == "queue"
        && e["operation"] == "ml.compute"));
    let resources = report["resources"].as_array().unwrap();
    assert!(
        resources
            .iter()
            .any(|r| r["service"] == "ml" && r["kind"] == "database")
    );
    assert!(
        resources
            .iter()
            .any(|r| r["service"] == "ml" && r["kind"] == "cache" && r["confidence"] == "medium")
    );
}

#[test]
fn python_local_modules_do_not_masquerade_as_infrastructure_packages() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM python");
    source(&scratch, "api/redis.py", "class Redis:\n    pass\n");
    source(
        &scratch,
        "api/main.py",
        "from redis import Redis\nclient = Redis()\n",
    );
    let report = inferred(&scratch, &[]);
    assert_eq!(report["services"], json!(["api"]));
    assert!(report.get("resources").is_none());
}

#[test]
fn python_relative_imports_cannot_climb_beyond_the_package_root() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM python");
    source(&scratch, "api/main.py", "from ...shared.db import cache\n");
    source(
        &scratch,
        "shared/db.py",
        "from redis import Redis\ncache = Redis()\n",
    );
    source(&scratch, "apps/worker/Dockerfile", "FROM python");
    source(
        &scratch,
        "apps/worker/main.py",
        "from ..common.db import cache\n",
    );
    source(
        &scratch,
        "apps/common/db.py",
        "from redis import Redis\ncache = Redis()\n",
    );
    let report = inferred(&scratch, &[]);
    let resources = report["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0]["service"], "apps/worker");
    assert_eq!(resources[0]["confidence"], "medium");
}

#[cfg(unix)]
#[test]
fn inferred_source_locations_must_fit_the_report_schema() {
    let scratch = Scratch::new();
    source(&scratch, "api/Dockerfile", "FROM node");
    source(
        &scratch,
        "api/invalid:name.ts",
        "fetch('https://example.com/x');",
    );
    let output = scratch.run(&["services", ".", "--format", "json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}
