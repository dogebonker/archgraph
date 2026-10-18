mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use common::{archgraph, layout, run_json, scan, scan_recorded, synthetic, write_tmp};
use serde_json::{Value, json};

/// The scan of `name` and a copy without the edge `from -> to`, as files.
fn before_after(name: &str, extra: &[&str], from: &str, to: &str) -> (String, String) {
    // Tests run in parallel: each call writes its own pair.
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let tag = format!("{name}-{}", CALLS.fetch_add(1, Ordering::Relaxed));
    let (doc, stdout, _) = scan_recorded(name, extra);
    let old = write_tmp(&format!("diff/{tag}-old.json"), &stdout);
    let mut fixed = doc.clone();
    let edges = fixed["edges"].as_array_mut().unwrap();
    let before = edges.len();
    edges.retain(|e| !(e["from"] == from && e["to"] == to));
    assert_eq!(edges.len(), before - 1);
    let new = write_tmp(&format!("diff/{tag}-new.json"), &fixed.to_string());
    (old, new)
}

#[test]
fn diff_shows_a_resolved_cycle() {
    // Dropping the break-set import c/z.ts -> a/x.ts resolves the a -> b -> c module cycle.
    let (old, new) = before_after("ts-cycles", &[], "src/c/z.ts", "src/a/x.ts");
    let (diff, code) = run_json(&["diff", &old, &new]);
    assert_eq!(code, Some(0));
    assert_eq!(diff["schema"], "archgraph/v1.2");
    assert_eq!(diff["kind"], "diff");
    // The commit of the checkout holding the fixtures, or null outside a git repository.
    let scanned: Value = serde_json::from_str(&std::fs::read_to_string(&old).unwrap()).unwrap();
    assert_eq!(diff["old"]["commit"], scanned["source"]["git"]["commit"]);
    assert!(diff["old"]["generated_at"].is_string() && diff["new"]["generated_at"].is_string());
    assert_eq!(
        diff["resolved_cycles"],
        json!([{ "level": "module", "members": ["a", "b", "c"], "exact": true,
                 "break": [{ "from": "src/c/z.ts", "to": "src/a/x.ts" }], "two_way": [] }])
    );
    assert_eq!(diff["new_cycles"], json!([]));
    assert_eq!(diff["added_modules"], json!([]));
    assert_eq!(diff["removed_modules"], json!([]));
    assert_eq!(diff["new_forbidden"], json!([]));
    assert_eq!(diff["resolved_forbidden"], json!([]));
    // a loses its importer and c its only outgoing import; every module is listed.
    assert_eq!(
        diff["modules"]["a"],
        json!({ "cohesion": [0.0, 0.0], "ca": [1, 0], "ce": [1, 1], "weighted_ca": [1.0, 0.0], "weighted_ce": [1.0, 1.0], "instability": [0.5, 1.0] })
    );
    assert_eq!(
        diff["modules"]["c"],
        json!({ "cohesion": [0.0, null], "ca": [1, 1], "ce": [1, 0], "weighted_ca": [1.0, 1.0], "weighted_ce": [1.0, 0.0], "instability": [0.5, 0.0] })
    );
    assert_eq!(
        diff["modules"]["f"],
        json!({ "cohesion": [1.0, 1.0], "ca": [0, 0], "ce": [0, 0], "weighted_ca": [0.0, 0.0], "weighted_ce": [0.0, 0.0], "instability": [null, null] })
    );
    assert_eq!(diff["modules"].as_object().unwrap().len(), 6);
}

#[test]
fn diff_shows_a_resolved_forbidden_edge_and_a_removed_module() {
    // domain/user.ts -> adapters/out/pg.ts is forbidden by `layers`.
    let rules = layout("ts-hexagonal");
    let (old, new) = before_after(
        "ts-hexagonal",
        &["--rules", &rules],
        "src/domain/user.ts",
        "src/adapters/out/pg.ts",
    );
    let (diff, _) = run_json(&["diff", &old, &new]);
    assert_eq!(
        common::without_import_metadata(&diff["resolved_forbidden"]),
        json!([{ "from": "src/domain/user.ts", "to": "src/adapters/out/pg.ts", "kind": "runtime", "weight": 1.0, "verdict": "forbidden", "rule": "layers" }])
    );
    assert_eq!(diff["new_forbidden"], json!([]));

    // Against a scan of a different tree, ts-cycles' modules all count as added.
    let (_, other, _) = scan_recorded("ts-cycles", &[]);
    let other = write_tmp("diff/other.json", &other);
    let (diff, _) = run_json(&["diff", &old, &other]);
    assert_eq!(diff["added_modules"], json!(["a", "b", "c", "d", "e", "f"]));
    assert_eq!(
        diff["removed_modules"],
        json!([
            "adapters/in",
            "adapters/out",
            "application",
            "domain",
            "main.ts"
        ])
    );
    assert_eq!(diff["modules"], json!({}));
    assert_eq!(diff["new_cycles"].as_array().unwrap().len(), 3);
    assert_eq!(diff["resolved_cycles"].as_array().unwrap().len(), 1);
}

/// Runs `diff`, returning stdout, stderr and the exit code.
fn diff(args: &[&str]) -> (String, String, Option<i32>) {
    let out = archgraph(&[&["diff"], args].concat());
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code(),
    )
}

#[test]
fn fail_on_new_forbidden_exits_1_only_when_one_appears() {
    let rules = layout("ts-hexagonal");
    let (old, new) = before_after(
        "ts-hexagonal",
        &["--rules", &rules],
        "src/domain/user.ts",
        "src/adapters/out/pg.ts",
    );
    // old -> new resolves it: nothing new.
    let (_, stderr, code) = diff(&[&old, &new, "--fail-on", "new-forbidden"]);
    assert_eq!(code, Some(0), "{stderr}");
    // new -> old adds it back; the JSON output is the plain diff document.
    let (doc, code) = run_json(&["diff", &new, &old, "--fail-on", "new-forbidden"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["new_forbidden"][0]["rule"], "layers");
    // Listed conditions only: a new forbidden import does not fail new-unplanned.
    let (_, _, code) = diff(&[&new, &old, "--fail-on", "new-unplanned"]);
    assert_eq!(code, Some(0));
}

#[test]
fn fail_on_new_cycles_exits_1_on_a_new_cycle() {
    let (old, new) = before_after("ts-cycles", &[], "src/c/z.ts", "src/a/x.ts");
    let (_, _, code) = diff(&[&old, &new, "--fail-on", "new-cycles"]);
    assert_eq!(code, Some(0));
    let (_, _, code) = diff(&[&new, &old, "--fail-on", "new-forbidden,new-cycles"]);
    assert_eq!(code, Some(1));
    // Spaces after the commas, as a YAML input `new-forbidden, new-cycles` gives them.
    let (_, stderr, code) = diff(&[&new, &old, "--fail-on", "new-forbidden, new-cycles "]);
    assert_eq!(code, Some(1), "{stderr}");
}

#[test]
fn a_grown_cycle_is_a_new_cycle() {
    // old: x <-> y. new: z joins it through y -> z -> x.
    let pairs = |edges: &[(&str, &str)]| -> Vec<(String, String)> {
        edges
            .iter()
            .map(|(f, t)| (f.to_string(), t.to_string()))
            .collect()
    };
    let scan_of = |name: &str, edges: &[(&str, &str)]| {
        let (dir, input) = synthetic(name, 3, &pairs(edges));
        let (_, stdout, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        write_tmp(&format!("diff/{name}.json"), &stdout)
    };
    let old = scan_of(
        "grow-old",
        &[
            ("x/a.ts", "y/b.ts"),
            ("y/b.ts", "x/a.ts"),
            ("y/b.ts", "z/c.ts"),
        ],
    );
    let new = scan_of(
        "grow-new",
        &[
            ("x/a.ts", "y/b.ts"),
            ("y/b.ts", "x/a.ts"),
            ("y/b.ts", "z/c.ts"),
            ("z/c.ts", "x/a.ts"),
        ],
    );
    let (doc, code) = run_json(&["diff", &old, &new, "--fail-on", "new-cycles"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["new_cycles"][0]["members"], json!(["x", "y", "z"]));
    assert_eq!(doc["resolved_cycles"][0]["members"], json!(["x", "y"]));
}

#[test]
fn fail_on_new_unplanned_reads_new_unplanned() {
    // controller.ts -> lib/clock.ts is the one unplanned import of ts-fda.
    let rules = layout("ts-fda");
    let (old, new) = before_after(
        "ts-fda",
        &["--rules", &rules],
        "src/domains/cart/controller.ts",
        "src/lib/clock.ts",
    );
    let (doc, code) = run_json(&["diff", &new, &old, "--fail-on", "new-unplanned"]);
    assert_eq!(code, Some(1));
    assert_eq!(
        common::without_import_metadata(&doc["new_unplanned"]),
        json!([{ "from": "src/domains/cart/controller.ts", "to": "src/lib/clock.ts",
                 "kind": "runtime", "weight": 1.0, "verdict": "unplanned", "rule": "none" }])
    );
    let (doc, code) = run_json(&["diff", &old, &new, "--fail-on", "new-unplanned"]);
    assert_eq!((doc["new_unplanned"].clone(), code), (json!([]), Some(0)));
}

#[test]
fn unknown_fail_on_value_is_a_usage_error() {
    let (old, new) = before_after("ts-cycles", &[], "src/c/z.ts", "src/a/x.ts");
    let (_, stderr, code) = diff(&[&old, &new, "--fail-on", "new-cycles,new-findings"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("new-findings"), "{stderr}");
}

#[test]
fn markdown_summary_lists_cycles_and_moved_metrics() {
    let (old, new) = before_after("ts-cycles", &[], "src/c/z.ts", "src/a/x.ts");
    let (md, _, code) = diff(&[
        &old,
        &new,
        "--format",
        "markdown",
        "--fail-on",
        "new-cycles",
    ]);
    assert_eq!(code, Some(0));
    assert!(md.starts_with("**archgraph diff: pass**"), "{md}");
    for part in [
        "### Resolved cycles (1)",
        "- module: a, b, c",
        "| module | cohesion | instability |",
        // a: instability 0.5 -> 1; c: 0.5 -> 0 (cohesion 0 -> undefined is not a move).
        "| a | 0.00 → 0.00 | 0.50 → 1.00 |",
        "| c | 0.00 → n/a | 0.50 → 0.00 |",
        "Modules: 0 added, 0 removed.",
    ] {
        assert!(md.contains(part), "{part:?} missing from\n{md}");
    }
    assert!(!md.contains("| f |"), "unchanged module listed:\n{md}");
    assert!(!md.contains("### New cycles"), "{md}");

    let (md, _, code) = diff(&[
        &new,
        &old,
        "--format",
        "markdown",
        "--fail-on",
        "new-cycles",
    ]);
    assert_eq!(code, Some(1));
    assert!(
        md.starts_with("**archgraph diff: fail** (new-cycles)"),
        "{md}"
    );
    assert!(
        md.contains("### New cycles (1)\n\n- module: a, b, c"),
        "{md}"
    );
}

#[test]
fn markdown_escapes_paths_from_the_scanned_repository() {
    // A module id and file path carrying every character that could make markdown.
    let evil = "ev|il/`a`*_[b]<c>";
    let file = format!("{evil}/f.ts");
    let edges = [
        (file.clone(), "y/g.ts".to_owned()),
        ("y/g.ts".to_owned(), file.clone()),
    ];
    let (dir, input) = synthetic("diff-escape", 2, &edges);
    let rules = write_tmp(
        "diff-escape/layout.yaml",
        "version: 1\nforbid:\n  - { id: no-y, from: \"**\", to: \"y/**\" }\n",
    );
    let (doc, stdout, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    let new = write_tmp("diff-escape/new.json", &stdout);
    let mut old_doc = doc.clone();
    old_doc["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["from"] != file.as_str());
    let old = write_tmp("diff-escape/old.json", &old_doc.to_string());

    let (md, _, code) = diff(&[
        &old,
        &new,
        "--format",
        "markdown",
        "--fail-on",
        "new-forbidden",
    ]);
    assert_eq!(code, Some(1));
    let escaped = r"ev\|il/\`a\`\*\_\[b\]\<c\>";
    assert!(
        md.contains(&format!("- {escaped}/f.ts → y/g.ts (no-y)")),
        "{md}"
    );
    assert!(md.contains(&format!("- module: {escaped}, y")), "{md}");
    assert!(md.contains(&format!("| {escaped} |")), "{md}");
    assert!(!md.contains(evil), "unescaped path in\n{md}");
}

#[test]
fn markdown_lists_at_most_twenty_moved_modules() {
    // 22 modules m00..m21 each gain an outgoing import: instability 0 -> 0.5.
    let scan_of = |name: &str, out: bool| {
        let mut edges = Vec::new();
        for i in 0..22 {
            edges.push(("w/w.ts".to_owned(), format!("m{i:02}/a.ts")));
            if out {
                edges.push((format!("m{i:02}/a.ts"), "z/z.ts".to_owned()));
            }
        }
        let (dir, input) = synthetic(name, edges.len(), &edges);
        let (_, stdout, _) = scan(&[
            &dir,
            "--collector-input",
            &input,
            "--language",
            "typescript",
        ]);
        write_tmp(&format!("diff/{name}.json"), &stdout)
    };
    let (old, new) = (scan_of("rows-old", false), scan_of("rows-new", true));
    let (md, _, code) = diff(&[&old, &new, "--format", "markdown"]);
    assert_eq!(code, Some(0));
    assert!(
        md.contains("### Module metrics (22 moved by 0.05 or more)"),
        "{md}"
    );
    assert_eq!(
        md.matches("| n/a → 0.00 | 0.00 → 0.50 |").count(),
        20,
        "{md}"
    );
    assert!(md.contains("| m19 |") && !md.contains("| m20 |"), "{md}");
    assert!(md.contains("\nand 2 more\n"), "{md}");
    assert!(md.contains("Modules: 1 added, 0 removed."), "{md}");
}
