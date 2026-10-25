mod common;

use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use archgraph::services::{self, Manifest};
use common::{archgraph, validate, write_tmp};
use serde_json::{Value, json};

fn call(operation: &str, line: usize) -> Value {
    json!({"from":"worker","to":"api","kind":"http","operation":operation,
        "confidence":"high","source":{"file":"worker/main.ts","line":line}})
}

fn resource(package: &str, line: usize) -> Value {
    json!({"service":"worker","kind":"database","package":package,"confidence":"high",
        "source":{"file":"worker/db.ts","line":line},"verdict":"allowed"})
}

fn rules() -> Value {
    json!([
        {"id":"no-api","from":"worker","to":"api","reason":"Use jobs, not **HTTP**."},
        {"id":"no-db","from":"worker","to":"pkg:*","reason":"Database access belongs to the API."}
    ])
}

fn report(calls: Value, resources: Value, forbid: Value) -> Value {
    let manifest: Manifest = serde_json::from_value(json!({
        "version":1,"services":["api","worker"],"edges":calls,"forbid":forbid,
    }))
    .unwrap();
    let mut report = serde_json::to_value(services::analyze(manifest, "source").unwrap()).unwrap();
    report["source"]["collector"] = json!("inferred+declarative");
    report["resources"] = resources;
    serde_json::to_value(services::read_report(&report.to_string()).unwrap()).unwrap()
}

fn run(before: &Value, after: &Value, extra: &[&str]) -> Output {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let next = NEXT.fetch_add(1, Ordering::Relaxed);
    let old = write_tmp(
        &format!("service-diff/{next}-old.json"),
        &before.to_string(),
    );
    let new = write_tmp(&format!("service-diff/{next}-new.json"), &after.to_string());
    archgraph(&[&["diff", &old, &new], extra].concat())
}

fn result(output: &Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate(&value);
    value
}

#[test]
fn classifies_calls_and_resources_without_losing_repeated_evidence() {
    let old = report(
        json!([
            call("GET /existing", 2),
            call("GET /existing", 5),
            call("GET /removed", 8)
        ]),
        json!([resource("pg", 2), resource("pg", 6), resource("mongodb", 8)]),
        rules(),
    );
    let mut new = report(
        json!([
            call("GET /existing", 20),
            call("GET /existing", 25),
            call("GET /added", 30)
        ]),
        json!([
            resource("pg", 20),
            resource("pg", 26),
            resource("mysql2", 30)
        ]),
        rules(),
    );
    new["edges"][0]["confidence"] = json!("medium");
    new["resources"][0]["confidence"] = json!("medium");
    let d = result(&run(&old, &new, &["--fail-on", "new-forbidden"]), 1);
    for domain in ["calls", "resources"] {
        let added = &d[format!("new_forbidden_{domain}")];
        let existing = &d[format!("existing_forbidden_{domain}")];
        let removed = &d[format!("resolved_forbidden_{domain}")];
        assert_eq!(added.as_array().unwrap().len(), 1);
        assert_eq!(existing.as_array().unwrap().len(), 1);
        assert_eq!(removed.as_array().unwrap().len(), 1);
        assert_eq!(added[0]["before"], json!([]));
        assert_eq!(removed[0]["after"], json!([]));
        assert_eq!(existing[0]["before"].as_array().unwrap().len(), 2);
        assert_eq!(existing[0]["after"].as_array().unwrap().len(), 2);
    }
    assert_eq!(d["policy_basis"], "after");
    assert_eq!(d["policy_changed"], false);
    assert_eq!(d["before"]["forbid"], rules());
    assert_eq!(d["after"]["forbid"], rules());
    assert!(
        d["caveats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap().contains("parity is unavailable"))
    );
    assert_eq!(d, result(&run(&old, &new, &[]), 0));
}

#[test]
fn policy_only_changes_use_after_precedence_without_becoming_new_violations() {
    let old = report(
        json!([call("GET /x", 2)]),
        json!([resource("pg", 2)]),
        json!([]),
    );
    let mut new = old.clone();
    new["forbid"] = rules();
    let d = result(&run(&old, &new, &["--fail-on", "new-forbidden"]), 0);
    assert_eq!(d["policy_changed"], true);
    assert_eq!(d["new_forbidden_calls"], json!([]));
    assert_eq!(d["new_forbidden_resources"], json!([]));
    assert_eq!(
        d["existing_forbidden_calls"][0]["before"][0]["rule"],
        "no-api"
    );
    assert_eq!(
        d["existing_forbidden_resources"][0]["before"][0]["rule"],
        "no-db"
    );

    let mut old = new.clone();
    old["forbid"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({"id":"broad","from":"*","to":"*","reason":"old"}));
    new["forbid"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"broad","from":"*","to":"*","reason":"changed"}));
    let d = result(&run(&old, &new, &["--fail-on", "new-forbidden"]), 0);
    assert_eq!(d["policy_changed"], true);
    assert_eq!(
        d["existing_forbidden_calls"][0]["before"][0]["rule"],
        "no-api"
    );
    assert_eq!(
        d["existing_forbidden_calls"][0]["after"][0]["rule"],
        "no-api"
    );
}

#[test]
fn gate_checks_calls_and_resources_independently_and_before_brief_output() {
    let old = report(json!([]), json!([]), rules());
    for (calls, resources) in [(false, false), (true, false), (false, true), (true, true)] {
        let new = report(
            if calls {
                json!([call("GET /x", 4)])
            } else {
                json!([])
            },
            if resources {
                json!([resource("pg", 4)])
            } else {
                json!([])
            },
            rules(),
        );
        let expected = i32::from(calls || resources);
        result(&run(&old, &new, &["--fail-on", "new-forbidden"]), expected);
        result(&run(&old, &new, &[]), 0);
    }
    let existing: Vec<_> = (1..=40)
        .map(|line| call(&format!("GET /existing-{line}"), line))
        .collect();
    let old = report(json!(existing), json!([]), rules());
    let mut new = old.clone();
    new["edges"].as_array_mut().unwrap().push(json!({
        "from":"worker","to":"api","kind":"http","operation":"GET /new", "confidence":"high",
        "source":{"file":"worker/zzz.ts","line":100},"verdict":"allowed"
    }));
    let output = run(&old, &new, &["--brief", "--fail-on", "new-forbidden"]);
    assert_eq!(output.status.code(), Some(1));
    let brief: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(brief["schema"], "archgraph/brief/v1");
    assert!(!brief["omitted"].as_object().unwrap().is_empty());
}

#[test]
fn moved_lines_confidence_and_extra_occurrences_are_existing_not_new() {
    let old = report(
        json!([call("GET /x", 3)]),
        json!([resource("pg", 3)]),
        rules(),
    );
    let mut new = report(
        json!([call("GET /x", 30), call("GET /x", 40)]),
        json!([resource("pg", 30), resource("pg", 40)]),
        rules(),
    );
    new["edges"][0]["confidence"] = json!("low");
    new["resources"][0]["confidence"] = json!("medium");
    let d = result(&run(&old, &new, &["--fail-on", "new-forbidden"]), 0);
    assert_eq!(d["new_forbidden_calls"], json!([]));
    assert_eq!(d["new_forbidden_resources"], json!([]));
    assert_eq!(
        d["existing_forbidden_calls"][0]["after"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        d["existing_forbidden_resources"][0]["after"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    new["edges"][0]["source"]["file"] = json!("worker/renamed.ts");
    let d = result(&run(&old, &new, &["--fail-on", "new-forbidden"]), 1);
    assert_eq!(
        d["new_forbidden_calls"][0]["after"][0]["source"]["file"],
        "worker/renamed.ts"
    );
}

#[test]
fn reader_recomputes_forged_verdicts_and_reports_source_labels_as_unverified() {
    let old = report(json!([]), json!([]), rules());
    let mut new = report(
        json!([call("GET /x", 3)]),
        json!([resource("pg", 3)]),
        rules(),
    );
    new["source"]["path"] = json!("another-label.json");
    new["summary"]["forbidden_calls"] = json!(0);
    new["forbidden_calls"] = json!([]);
    new["forbidden_resources"] = json!([]);
    new["edges"][0]["verdict"] = json!("allowed");
    new["edges"][0]["rule"] = json!("fake");
    new["resources"][0]["verdict"] = json!("allowed");
    let d = result(&run(&old, &new, &["--fail-on", "new-forbidden"]), 1);
    assert_eq!(d["new_forbidden_calls"][0]["after"][0]["rule"], "no-api");
    assert_eq!(d["new_forbidden_resources"][0]["after"][0]["rule"], "no-db");
    assert!(
        d["caveats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap().contains("Source labels differ"))
    );
}

#[test]
fn malformed_mixed_incompatible_and_unsupported_comparisons_exit_two() {
    let valid = report(json!([]), json!([]), rules());
    let cases = [
        (json!({"kind":"scan"}), valid.clone(), vec![]),
        (valid.clone(), json!({"kind":"scan"}), vec![]),
        (
            {
                let mut v = valid.clone();
                v["source"]["collector"] = json!("declarative");
                v
            },
            valid.clone(),
            vec![],
        ),
        (
            valid.clone(),
            {
                let mut v = valid.clone();
                v["source"]["complete"] = json!(true);
                v
            },
            vec![],
        ),
        (
            valid.clone(),
            {
                let mut v = valid.clone();
                v["edges"] = json!([call("GET /bad", 0)]);
                v
            },
            vec![],
        ),
        (
            valid.clone(),
            {
                let mut v = valid.clone();
                v["resources"] = json!([resource("pg", 1)]);
                v["resources"][0]["service"] = json!("unknown");
                v
            },
            vec![],
        ),
        (
            valid.clone(),
            valid.clone(),
            vec!["--fail-on", "new-cycles"],
        ),
        (valid.clone(), valid.clone(), vec!["--legacy-v1"]),
        (
            valid.clone(),
            {
                let mut v = valid.clone();
                v["forbid"] = json!([]);
                v
            },
            vec!["--fail-on", "new-forbidden"],
        ),
    ];
    for (old, new, flags) in cases {
        let output = run(&old, &new, &flags);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn markdown_discloses_policy_reasons_evidence_and_incomplete_resolution() {
    let old = report(
        json!([call("GET /removed", 3)]),
        json!([resource("pg", 3)]),
        rules(),
    );
    let new = report(json!([call("GET /<new>[x]", 4)]), json!([]), rules());
    let output = run(
        &old,
        &new,
        &["--format", "markdown", "--fail-on", "new-forbidden"],
    );
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Policy basis: after.forbid"));
    assert!(text.contains("worker/main.ts:3"));
    assert!(text.contains("worker/main.ts:4"));
    assert!(text.contains("no longer observed"));
    assert!(text.contains("Use jobs, not \\*\\*HTTP\\*\\*"));
    assert!(text.contains("&lt;new&gt;\\[x\\]"));
    assert!(!text.contains("<new>"));
}
