mod common;

use std::fs;

use common::{archgraph, layout, scan, scan_file, synthetic, validate, write_tmp};
use serde_json::Value;

const OPEN: &str = r#"<script type="application/json" id="graph">"#;

fn render(args: &[&str], name: &str) -> String {
    let out_path = write_tmp(name, "");
    let out = archgraph(&[&["render"], args, &["-o", &out_path]].concat());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::read_to_string(out_path).unwrap()
}

/// The embedded document's JSON text, exactly as written.
fn embedded(html: &str) -> &str {
    let start = html.find(OPEN).unwrap() + OPEN.len();
    let len = html[start..].find("</script>").unwrap();
    &html[start..start + len]
}

#[test]
fn render_is_self_contained_and_embeds_the_document() {
    let graph = scan_file("ts-fda", &["--rules", &layout("ts-fda")]);
    let html = render(&[&graph], "ts-fda.html");

    for url in ["http://", "https://", "\"//", "'//", "(//"] {
        assert!(!html.contains(url), "{url} in the rendered page");
    }
    assert!(html.contains(
        r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'">"#
    ));
    // Same document, same picture; document text never reaches the page as markup or code.
    // Checked on the template: the embedded document may hold any of these in a path.
    let template = [
        include_str!("../assets/graph.html"),
        include_str!("../assets/whatif.js"),
        include_str!("../assets/whatif-ui.js"),
    ]
    .join("\n");
    for banned in [
        "Math.random",
        "Date.now",
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "new Function",
        "fetch(",
    ] {
        assert!(!template.contains(banned), "{banned} in the page template");
    }
    // The ring colour comes only from cycles, forbidden imports and findings.
    for gone in ["low cohesion, many links", "cohesive, few links"] {
        assert!(!template.contains(gone), "{gone} in the page template");
    }
    // The embedded scan is the input document recomputed with the same rules: identical.
    let doc: Value = serde_json::from_str(embedded(&html)).unwrap();
    let input: Value = serde_json::from_str(&fs::read_to_string(&graph).unwrap()).unwrap();
    assert_eq!(doc, input);
    validate(&doc);
}

#[test]
fn script_end_tag_in_a_path_stays_escaped() {
    let evil = "src/x/</script><script>alert(1)</script>.ts".to_owned();
    let edges = [(evil.clone(), "src/y/b.ts".to_owned())];
    let (dir, input) = synthetic("render-escape", 2, &edges);
    let (doc, stdout, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let graph = write_tmp("render-escape/graph.json", &stdout);
    let html = render(&[&graph], "render-escape/graph.html");

    let text = embedded(&html);
    assert!(!text.contains('<'), "unescaped < in {text}");
    assert!(!html.contains(&evil));
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed, doc);
    assert_eq!(parsed["files"][0]["path"], evil.as_str());
}

#[test]
fn asset_placeholders_in_repository_paths_remain_literal_data() {
    let paths = [
        "src/x/*__ARCHGRAPH_PREVIEW__*/{{GRAPH}}.ts".to_owned(),
        "src/y/*__ARCHGRAPH_PREVIEW_UI__*/dep.ts".to_owned(),
    ];
    let (dir, input) = synthetic("render-markers", 2, &[(paths[0].clone(), paths[1].clone())]);
    let (_, stdout, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let graph = write_tmp("render-markers/graph.json", &stdout);
    let html = render(&[&graph], "render-markers/graph.html");
    let doc: Value = serde_json::from_str(embedded(&html)).unwrap();
    for path in paths {
        assert!(
            doc["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == path)
        );
    }
    assert_eq!(html.matches("root.ArchgraphPreview = api").count(), 1);
}

#[test]
fn counts_agree_in_number_with_their_nouns() {
    // The page builds its header in the browser; the template must pick "1 cycle" over
    // "1 cycles" for every count it prints.
    let graph = scan_file("ts-fda", &["--rules", &layout("ts-fda")]);
    let html = render(&[&graph], "ts-fda-plural.html");
    // The pure engine can contain calls such as test(s); only UI assets contain count prose.
    for ui in [
        include_str!("../assets/graph.html"),
        include_str!("../assets/whatif-ui.js"),
    ] {
        assert!(!ui.contains("(s)"), "a count written as noun(s)");
    }
    for noun in [
        "file",
        "module",
        "module import",
        "cycle",
        "forbidden import",
        "finding",
    ] {
        assert!(
            html.contains(&format!(r#""{noun}", "{noun}s")"#)),
            "{noun} is not pluralized by count"
        );
    }
}

#[test]
fn simulation_accepts_dash_leading_move_paths_used_by_copied_commands() {
    let edges = [("-old.ts".to_owned(), "lib/dep.ts".to_owned())];
    let (dir, input) = synthetic("whatif-dash-path", 1, &edges);
    let (_, stdout, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let graph = write_tmp("whatif-dash-path/-graph.json", &stdout);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .current_dir(std::path::Path::new(&graph).parent().unwrap())
        .args(["simulate", "./-graph.json", "--move", "-old.ts", "-new.ts"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["move"]["file"], "-old.ts");
    assert_eq!(result["move"]["to"], "-new.ts");
}

const SERVICES_OPEN: &str = r#"<script type="application/json" id="services">"#;

fn embedded_services(html: &str) -> Value {
    let start = html.find(SERVICES_OPEN).unwrap() + SERVICES_OPEN.len();
    let len = html[start..].find("</script>").unwrap();
    let report: Value = serde_json::from_str(&html[start..start + len]).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.2.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(&report)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    report
}

fn service_report() -> Value {
    serde_json::from_str(include_str!("fixtures/services-declarative/report.json")).unwrap()
}

#[test]
fn services_render_preserves_manifest_reports_and_normalizes_derived_values() {
    let report = service_report();
    let path = write_tmp("render-services/report.json", &report.to_string());
    let first = render(&[&path], "render-services/report.html");
    assert_eq!(embedded_services(&first), report);
    assert_eq!(first, render(&[&path], "render-services/repeated.html"));
    assert!(first.contains(r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'">"#));

    let mut forged = report.clone();
    forged["summary"]["calls"] = serde_json::json!(999);
    forged["modules"] = serde_json::json!([]);
    forged["cycles"] = serde_json::json!([]);
    forged["hubs"] = serde_json::json!([]);
    forged["forbidden_calls"] = serde_json::json!([999]);
    forged["edges"][0]["verdict"] = serde_json::json!("forbidden");
    forged["edges"][0]["rule"] = serde_json::json!("forged");
    let input = write_tmp("render-services/forged.json", &forged.to_string());
    assert_eq!(
        embedded_services(&render(&[&input], "render-services/normalized.html")),
        report
    );
}

#[test]
fn services_render_includes_inferred_resources_and_recomputes_package_verdicts() {
    let package = write_tmp(
        "render-inferred-services/package.json",
        r#"{"scripts":{"start":"node main.js"},"dependencies":{"pg":"*"}}"#,
    );
    write_tmp(
        "render-inferred-services/main.ts",
        "import {Pool} from 'pg'; new Pool();",
    );
    let root = std::path::Path::new(&package).parent().unwrap();
    let output = archgraph(&["services", root.to_str().unwrap(), "--format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source"]["collector"], "inferred");
    assert_eq!(report["summary"]["resources"], 1);
    report["resources"][0]["verdict"] = serde_json::json!("forbidden");
    report["resources"][0]["rule"] = serde_json::json!("forged");
    report["forbidden_resources"] = serde_json::json!([0]);
    report["summary"]["forbidden_resources"] = serde_json::json!(1);
    let input = write_tmp("render-inferred-services/report.json", &report.to_string());
    let normalized = embedded_services(&render(&[&input], "render-inferred-services/report.html"));
    assert_eq!(normalized["resources"][0]["verdict"], "allowed");
    assert!(normalized["resources"][0].get("rule").is_none());
    assert!(normalized.get("forbidden_resources").is_none());
    assert!(normalized["summary"].get("forbidden_resources").is_none());
}

#[test]
fn service_report_strings_cannot_close_the_data_script_or_expand_asset_markers() {
    let mut report = service_report();
    let hostile = "</script><script>alert('owned')</script>{{SERVICES}}";
    report["edges"][0]["operation"] = serde_json::json!(hostile);
    report["caveats"] = serde_json::json!([hostile]);
    let input = write_tmp("render-services/hostile.json", &report.to_string());
    let html = render(&[&input], "render-services/hostile.html");
    assert!(!html.contains(hostile));
    assert_eq!(embedded_services(&html), report);
    let start = html.find(SERVICES_OPEN).unwrap() + SERVICES_OPEN.len();
    let len = html[start..].find("</script>").unwrap();
    assert!(!html[start..start + len].contains('<'));
}

#[test]
fn malformed_service_reports_fail_without_overwriting_the_output() {
    let base = service_report();
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/schema", serde_json::json!("archgraph/v1.1")),
        ("/source/collector", serde_json::json!("unknown")),
        ("/source/complete", serde_json::json!(true)),
        ("/source/path", serde_json::json!("")),
        ("/caveats/0", serde_json::json!("bad\u{0000}caveat")),
        ("/edges/0/to", serde_json::json!("unknown")),
        ("/edges/0/source/file", serde_json::json!("../outside.ts")),
        ("/edges/0/source/line", serde_json::json!(0)),
    ] {
        let mut report = base.clone();
        *report.pointer_mut(pointer).unwrap() = value;
        cases.push(report);
    }
    for resource in [
        serde_json::json!({"service":"unknown","kind":"database","package":"pg","confidence":"high","source":{"file":"db.ts","line":1},"verdict":"allowed"}),
        serde_json::json!({"service":base["services"][0],"kind":"database","package":"","confidence":"high","source":{"file":"db.ts","line":1},"verdict":"allowed"}),
        serde_json::json!({"service":base["services"][0],"kind":"database","package":"pg","confidence":"high","source":{"file":"/outside.ts","line":1},"verdict":"allowed"}),
        serde_json::json!({"service":base["services"][0],"kind":"database","package":"pg","operation":"bad\noperation","confidence":"high","source":{"file":"db.ts","line":1},"verdict":"allowed"}),
    ] {
        let mut report = base.clone();
        report["resources"] = serde_json::json!([resource]);
        cases.push(report);
    }
    let output = write_tmp("render-services/kept.html", "keep existing report");
    for (index, report) in cases.into_iter().enumerate() {
        let input = write_tmp(
            &format!("render-services/invalid-{index}.json"),
            &report.to_string(),
        );
        let result = archgraph(&["render", &input, "-o", &output]);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep existing report");
    }
    let input = write_tmp("render-services/rules-report.json", &base.to_string());
    let result = archgraph(&[
        "render",
        &input,
        "--rules",
        &layout("ts-fda"),
        "-o",
        &output,
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--rules"));
    assert_eq!(fs::read_to_string(output).unwrap(), "keep existing report");
}
