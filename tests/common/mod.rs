#![allow(dead_code)] // each test crate uses a different subset

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn boundrail(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args(args)
        .output()
        .unwrap()
}

/// Runs `scan` and returns the parsed document, raw stdout and stderr. Fails on a non-zero exit
/// or a schema violation.
pub fn scan(args: &[&str]) -> (Value, String, String) {
    let out = boundrail(&[&["scan"], args].concat());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(out.status.success(), "scan failed: {stderr}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    validate(&doc);
    (doc, stdout, stderr)
}

/// `scan` on a fixture with its recorded collector output.
pub fn scan_recorded(name: &str, extra: &[&str]) -> (Value, String, String) {
    let dir = fixture(name);
    let input = ["depcruise.json", "grimp.json", "go.json"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.exists())
        .unwrap();
    let (dir, input) = (dir.to_str().unwrap(), input.to_str().unwrap());
    scan(&[&[dir, "--collector-input", input], extra].concat())
}

pub fn validate(doc: &Value) {
    let schema: Value = serde_json::from_str(if doc["schema"] == "boundrail/v1.2" {
        include_str!("../../schema/boundrail.v1.2.schema.json")
    } else if doc["schema"] == "boundrail/v1.1" {
        include_str!("../../schema/boundrail.v1.1.schema.json")
    } else {
        include_str!("../../schema/boundrail.v1.schema.json")
    })
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "schema violations: {errors:#?}");
    // `outgoing_to` splits `outgoing_edges` by target module.
    for m in doc["modules"].as_array().into_iter().flatten() {
        let split: u64 = m["outgoing_to"]
            .as_object()
            .map(|o| o.values().map(|n| n.as_u64().unwrap()).sum())
            .unwrap_or(0);
        assert_eq!(split, m["outgoing_edges"].as_u64().unwrap(), "{}", m["id"]);
    }
}

/// Ignore timestamp and Git-relative location when comparing scans of copied fixtures.
pub fn stable(stdout: &str) -> String {
    let mut doc: Value = serde_json::from_str(stdout).unwrap();
    if let Some(source) = doc.get_mut("source").and_then(Value::as_object_mut) {
        source.remove("generated_at");
        source.remove("scan_root");
    }
    serde_json::to_string_pretty(&doc).unwrap()
}

/// Existing graph assertions retain targets, kinds, weights and verdicts while source-evidence
/// tests separately check the optional import metadata.
pub fn without_import_metadata(value: &Value) -> Value {
    let mut projected = value.clone();
    fn visit(value: &mut Value) {
        match value {
            Value::Array(values) => values.iter_mut().for_each(visit),
            Value::Object(fields) => {
                if fields.contains_key("from") && fields.contains_key("to") {
                    for key in [
                        "names",
                        "line",
                        "specifier",
                        "confidence",
                        "original_to",
                        "resolution_reason",
                    ] {
                        fields.remove(key);
                    }
                }
                fields.values_mut().for_each(visit);
            }
            _ => {}
        }
    }
    visit(&mut projected);
    projected
}

/// Validates `sarif` against the OASIS SARIF 2.1.0 errata01 schema, vendored byte-identical from
/// https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json
/// (sha256 c3b4bb2d6093897483348925aaa73af03b3e3f4bd4ca38cef26dcb4212a2682e).
pub fn validate_sarif(sarif: &Value) {
    let schema: Value =
        serde_json::from_str(include_str!("../fixtures/sarif-schema-2.1.0.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(sarif)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "SARIF violations: {errors:#?}");
}

pub fn layout(name: &str) -> String {
    fixture(name)
        .join("layout.yaml")
        .to_str()
        .unwrap()
        .to_owned()
}

/// Writes `contents` to a file under the test temp dir and returns its path.
pub fn write_tmp(name: &str, contents: &str) -> String {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_owned()
}

/// `scan_recorded` written to a file, for the subcommands that read a scan document.
pub fn scan_file(name: &str, extra: &[&str]) -> String {
    let (_, stdout, _) = scan_recorded(name, extra);
    write_tmp(&format!("{name}-{}.json", extra.len()), &stdout)
}

/// Runs a subcommand that prints a document, returning it parsed and validated with the exit code.
pub fn run_json(args: &[&str]) -> (Value, Option<i32>) {
    let out = boundrail(args);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let doc: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("{e}: {stdout}{}", String::from_utf8_lossy(&out.stderr)));
    validate(&doc);
    (doc, out.status.code())
}

/// Every edge as (from, to, verdict, rule).
pub fn verdicts(doc: &Value) -> Vec<(String, String, String, String)> {
    doc["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let s = |k: &str| e[k].as_str().unwrap_or("null").to_owned();
            (s("from"), s("to"), s("verdict"), s("rule"))
        })
        .collect()
}

pub fn finding<'a>(doc: &'a Value, kind: &str, subject: &str) -> &'a Value {
    doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .chain(
            doc["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| &d["finding"]),
        )
        .find(|f| f["kind"] == kind && f["subject"] == subject)
        .unwrap_or_else(|| panic!("no {kind} finding for {subject}: {:#}", doc["findings"]))
}

/// Writes a depcruise document for `edges` between files that need not exist.
pub fn synthetic(name: &str, total_cruised: usize, edges: &[(String, String)]) -> (String, String) {
    let typed: Vec<(&str, &str, &[&str])> = edges
        .iter()
        .map(|(f, t)| (f.as_str(), t.as_str(), &["local", "import"][..]))
        .collect();
    synthetic_typed(name, total_cruised, &typed)
}

/// `synthetic` with each edge's depcruise `dependencyTypes`.
pub fn synthetic_typed(
    name: &str,
    total_cruised: usize,
    edges: &[(&str, &str, &[&str])],
) -> (String, String) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::create_dir_all(&dir).unwrap();
    let mut sources: Vec<&str> = edges.iter().flat_map(|(f, t, _)| [*f, *t]).collect();
    sources.sort();
    sources.dedup();
    let modules: Vec<Value> = sources
        .iter()
        .map(|s| {
            let deps: Vec<Value> = edges
                .iter()
                .filter(|(f, _, _)| f == s)
                .map(|(_, t, types)| {
                    json!({ "resolved": t, "dependencyTypes": types,
                                      "coreModule": false, "couldNotResolve": false })
                })
                .collect();
            json!({ "source": s, "dependencies": deps })
        })
        .collect();
    let doc = json!({ "modules": modules,
                      "summary": { "totalCruised": total_cruised, "environment": { "version": "18.4.0" } } });
    let input = dir.join("depcruise.json");
    fs::write(&input, doc.to_string()).unwrap();
    (
        dir.to_str().unwrap().to_owned(),
        input.to_str().unwrap().to_owned(),
    )
}

/// Legacy evidence goldens stay independent of recipe attachment and advisory classification.
/// Call only after validating the complete current-schema output.
pub fn evidence_contract(mut doc: Value) -> Value {
    if let Some(diagnostics) = doc.get("diagnostics").and_then(Value::as_array).cloned()
        && let Some(findings) = doc.get_mut("findings").and_then(Value::as_array_mut)
    {
        findings.extend(diagnostics.into_iter().map(|d| d["finding"].clone()));
    }
    if let Some(findings) = doc.get_mut("findings").and_then(Value::as_array_mut) {
        findings.retain(|f| !matches!(f["kind"].as_str(), Some("forbidden-edge" | "deep-import")));
        for finding in findings.iter_mut() {
            let fields = finding.as_object_mut().unwrap();
            fields.remove("finding_id");
            fields.remove("recipe");
        }
        findings.sort_by(|a, b| {
            (a["kind"].as_str(), a["subject"].as_str())
                .cmp(&(b["kind"].as_str(), b["subject"].as_str()))
        });
    }
    doc
}
