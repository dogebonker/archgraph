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
