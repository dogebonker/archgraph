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
        json!([{ "level": "module", "members": ["a", "b", "c"], "exact": true, "grouping": false,
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
fn fail_on_stale_known_exits_1_when_a_listed_import_is_fixed() {
    let rules = write_tmp(
        "diff/known/layout.yaml",
        &std::fs::read_to_string(layout("ts-hexagonal")).unwrap(),
    );
    std::fs::write(
        std::path::Path::new(&rules).with_file_name("layout.known.yaml"),
        "version: 1\nknown:\n  - { rule: \"layers\", from: \"src/domain/user.ts\", to: \"src/adapters/out/pg.ts\" }\n",
    )
    .unwrap();
    let (old, new) = before_after(
        "ts-hexagonal",
        &["--rules", &rules],
        "src/domain/user.ts",
        "src/adapters/out/pg.ts",
    );
    let (stdout, stderr, code) = diff(&[
        &old,
        &new,
        "--fail-on",
        "stale-known",
        "--format",
        "markdown",
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stdout.starts_with("**archgraph diff: fail** (stale-known): "),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "\n### Known entries no longer forbidden (1)\n\n- src/domain/user.ts → src/adapters/out/pg.ts (layers)\n"
        ),
        "{stdout}"
    );
    let (doc, code) = run_json(&["diff", &old, &new, "--fail-on", "stale-known"]);
    assert_eq!(code, Some(1));
    assert_eq!(
        doc["stale_known"],
        json!([{ "from": "src/domain/user.ts", "to": "src/adapters/out/pg.ts", "rule": "layers" }])
    );
    // Not listed in --fail-on: the row shows, the exit stays 0. Old -> old has nothing stale.
    let (stdout, _, code) = diff(&[&old, &new, "--format", "markdown"]);
    assert_eq!(code, Some(0));
    assert!(stdout.starts_with("**archgraph diff: pass**"), "{stdout}");
    let (_, _, code) = diff(&[&new, &old, "--fail-on", "stale-known"]);
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
fn cycle_failure_uses_file_evidence_without_hiding_module_observations() {
    let edges = [
        ("app/main.ts".to_owned(), "api/router.ts".to_owned()),
        ("api/router.ts".to_owned(), "app/deps.ts".to_owned()),
    ];
    let (dir, input) = synthetic("diff-composition", 3, &edges);
    let (dag, stdout, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
    ]);
    let dag_path = write_tmp("diff/composition-dag.json", &stdout);
    let mut empty = dag.clone();
    empty["edges"] = json!([]);
    let empty_path = write_tmp("diff/composition-empty.json", &empty.to_string());
    let (observations, code) =
        run_json(&["diff", &empty_path, &dag_path, "--fail-on", "new-cycles"]);
    assert_eq!(code, Some(0));
    assert!(!observations["new_cycles"].as_array().unwrap().is_empty());
    assert_eq!(observations["new_cyclic_imports"], json!([]));

    let mut cyclic = dag;
    cyclic["edges"].as_array_mut().unwrap().push(json!({
        "from":"api/router.ts", "to":"app/main.ts", "kind":"runtime", "weight":1.0,
    }));
    let cyclic_path = write_tmp("diff/composition-cyclic.json", &cyclic.to_string());
    // The display-folder SCC already existed; the new concrete cycle still fails.
    let (actual, code) = run_json(&["diff", &dag_path, &cyclic_path, "--fail-on", "new-cycles"]);
    assert_eq!(code, Some(1));
    assert_eq!(actual["new_cycles"], json!([]));
    assert_eq!(
        actual["new_cyclic_imports"],
        json!([
            {"from":"api/router.ts", "to":"app/main.ts"},
            {"from":"app/main.ts", "to":"api/router.ts"},
        ])
    );
    let (markdown, _, _) = diff(&[
        &dag_path,
        &cyclic_path,
        "--format",
        "markdown",
        "--fail-on",
        "new-cycles",
    ]);
    assert!(
        markdown.contains("### New cyclic runtime imports (2)"),
        "{markdown}"
    );
    assert!(
        markdown.contains("api/router.ts → app/main.ts"),
        "{markdown}"
    );
    let (_, code) = run_json(&["diff", &cyclic_path, &dag_path, "--fail-on", "new-cycles"]);
    assert_eq!(code, Some(0));
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
        "### Resolved dependency components (1)",
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
    assert!(!md.contains("### New dependency components"), "{md}");

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
        md.contains("### New dependency components (1)\n\n- module: a, b, c"),
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

/// Two slices under one captured claim; `extra` is the head's added import and `moved` renames
/// base files.
fn target_scan(name: &str, extra: &[(&str, &str)], moved: &[(&str, &str)]) -> String {
    let rename = |p: &str| {
        moved
            .iter()
            .find(|(from, _)| *from == p)
            .map_or(p, |(_, to)| to)
            .to_string()
    };
    let mut edges: Vec<(String, String)> = [
        ("src/m/a/app/x.ts", "src/shared/id.ts"),
        ("src/m/a/domain/d.ts", "src/shared/id.ts"),
        ("src/m/b/app/y.ts", "src/shared/id.ts"),
        ("src/m/b/domain/e.ts", "src/shared/id.ts"),
    ]
    .iter()
    .chain(extra)
    .map(|(f, t)| (rename(f), rename(t)))
    .collect();
    edges.sort();
    let (dir, input) = synthetic(name, 5, &edges);
    let rules = write_tmp(
        &format!("diff/{name}.yaml"),
        "version: 1\nforbid:\n\
         \x20 - { id: app-no-domain, from: \"src/m/<name>/app/**\", to: \"src/m/<name>/domain/**\", reason: \"apps never reach a domain\" }\n\
         moves:\n\
         \x20 - { from: \"src/m/<name>/app/**\", to: \"src/app/<name>/\" }\n",
    );
    let (_, stdout, _) = scan(&[
        &dir,
        "--collector-input",
        &input,
        "--language",
        "typescript",
        "--rules",
        &rules,
    ]);
    write_tmp(&format!("diff/{name}.json"), &stdout)
}

#[test]
fn markdown_reports_target_progress_and_newly_broken_slices() {
    let old = target_scan("target-old", &[], &[]);
    let new = target_scan(
        "target-new",
        &[("src/m/b/app/y.ts", "src/m/b/domain/e.ts")],
        &[],
    );
    // A slice that moved shows its old count too. The claim covers only today's paths, so the
    // moved slice b no longer binds it.
    let moved = target_scan(
        "target-moved",
        &[],
        &[("src/m/b/app/y.ts", "src/app/b/y.ts")],
    );
    let (markdown, _, _) = diff(&[&old, &moved, "--format", "markdown"]);
    assert!(
        markdown.contains(
            "\nTarget: 1 of 1 slices conform (was 2 of 2), 1 of 2 slices moved (was 0 of 2).\n"
        ),
        "{markdown}"
    );
    let (markdown, _, code) = diff(&[&old, &new, "--format", "markdown"]);
    assert_eq!(code, Some(0));
    assert!(
        markdown.contains(
            "\nTarget: 1 of 2 slices conform (was 2 of 2), 0 of 2 slices moved, 1 newly broken slice.\n"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains("### Newly broken slices (1)"),
        "{markdown}"
    );
    assert!(!markdown.contains("Conforming slices"), "{markdown}");
    assert!(
        markdown.contains("- app-no-domain, slice b: src/m/b/app/y.ts → src/m/b/domain/e.ts"),
        "{markdown}"
    );
    let (doc, _) = run_json(&["diff", &old, &new]);
    common::validate(&doc);
    assert_eq!(
        doc["target"]["conforming"],
        json!([{"done": 2, "of": 2}, {"done": 1, "of": 2}])
    );
    assert_eq!(
        doc["target"]["moved"],
        json!([{"done": 0, "of": 2}, {"done": 0, "of": 2}])
    );
    // depcruise records no lines, so no import has one.
    let b = json!([{"claim": "app-no-domain", "reason": "apps never reach a domain", "slice": "b",
                    "imports": [{"from": "src/m/b/app/y.ts", "to": "src/m/b/domain/e.ts"}]}]);
    assert_eq!(doc["target"]["newly_broken"], b);
    assert_eq!(doc["target"]["newly_fixed"], json!([]));
    let (reverse, _) = run_json(&["diff", &new, &old]);
    common::validate(&reverse);
    assert_eq!(reverse["target"]["newly_broken"], json!([]));
    assert_eq!(reverse["target"]["newly_fixed"], b);
    // T16: the same diff fails the gate.
    let (_, _, code) = diff(&[&old, &new, "--fail-on", "new-forbidden"]);
    assert_eq!(code, Some(1));
    let (markdown, _, _) = diff(&[&old, &old, "--format", "markdown"]);
    assert!(
        markdown.contains("\nTarget: 2 of 2 slices conform, 0 of 2 slices moved.\n"),
        "{markdown}"
    );
    assert!(!markdown.contains("Newly broken"), "{markdown}");
    let plain = before_after("ts-cycles", &[], "src/c/z.ts", "src/a/x.ts");
    let (markdown, _, _) = diff(&[&plain.0, &plain.1, "--format", "markdown"]);
    assert!(!markdown.contains("Target:"), "{markdown}");
}

#[test]
fn fail_on_new_vacuous_exits_1_when_a_change_empties_a_claim() {
    let old = target_scan("vacuous-old", &[], &[]);
    // Both apps move out of src/m/<name>/app/, so app-no-domain's `from` matches no file.
    let gone = target_scan(
        "vacuous-gone",
        &[],
        &[
            ("src/m/a/app/x.ts", "src/app/a/x.ts"),
            ("src/m/b/app/y.ts", "src/app/b/y.ts"),
        ],
    );
    let (markdown, stderr, code) = diff(&[
        &old,
        &gone,
        "--fail-on",
        "new-vacuous",
        "--format",
        "markdown",
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        markdown.starts_with("**archgraph diff: fail** (new-vacuous): "),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "\nTarget: 0 of 0 slices conform (was 2 of 2), 2 of 2 slices moved (was 0 of 2), 1 newly vacuous claim.\n"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "\n### Newly vacuous claims (1)\n\n- app-no-domain: forbid[0] `from` matches no file\n"
        ),
        "{markdown}"
    );
    let (doc, code) = run_json(&["diff", &old, &gone, "--fail-on", "new-vacuous"]);
    assert_eq!(code, Some(1));
    common::validate(&doc);
    assert_eq!(
        doc["target"]["newly_vacuous"],
        json!([{"claim": "app-no-domain", "reason": "apps never reach a domain",
                "notes": ["forbid[0] `from` matches no file"]}])
    );
    // A claim vacuous in the base, or not vacuous in the head, never counts.
    for (from, to) in [(&gone, &old), (&old, &old)] {
        let (doc, code) = run_json(&["diff", from, to, "--fail-on", "new-vacuous"]);
        assert_eq!(code, Some(0));
        assert!(doc["target"].get("newly_vacuous").is_none(), "{doc}");
    }
    // One app moved: the claim still binds slice a.
    let partial = target_scan(
        "vacuous-partial",
        &[],
        &[("src/m/b/app/y.ts", "src/app/b/y.ts")],
    );
    let (markdown, _, code) = diff(&[
        &old,
        &partial,
        "--fail-on",
        "new-vacuous",
        "--format",
        "markdown",
    ]);
    assert_eq!(code, Some(0));
    assert!(!markdown.contains("vacuous"), "{markdown}");
}

const GO_CLAIM: &str = "version: 1\nforbid:\n\
    \x20 - { id: no-adapters, from: \"domain/**\", to: \"adapters/**\", reason: \"the domain stays pure\" }\n";

#[test]
fn go_target_imports_take_the_line_of_their_first_use() {
    // A Go edge carries the first line using its target (8, `postgres.Insert`), which wins over
    // the import record's line (3, the import statement).
    let rules = write_tmp("diff/go-claim.yaml", GO_CLAIM);
    let (broken, held) = before_after(
        "go-layers",
        &["--rules", &rules],
        "domain/order.go",
        "adapters/postgres/repo.go",
    );
    let scanned: Value = serde_json::from_str(&std::fs::read_to_string(&broken).unwrap()).unwrap();
    assert!(scanned["edges"].as_array().unwrap().iter().any(|e| {
        e["from"] == "domain/order.go" && e["to"] == "adapters/postgres/repo.go" && e["line"] == 8
    }));
    let (doc, _) = run_json(&["diff", &held, &broken]);
    assert_eq!(
        doc["target"]["newly_broken"],
        json!([{"claim": "no-adapters", "reason": "the domain stays pure", "slice": "*",
                "imports": [{"from": "domain/order.go", "to": "adapters/postgres/repo.go", "line": 8}]}])
    );
}

#[test]
fn go_target_imports_without_edge_lines_take_the_import_record_line() {
    // A recording made before edges carried lines: the import statement's line (3) stands in.
    let dir = common::fixture("go-layers");
    let mut recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("go.json")).unwrap()).unwrap();
    for edge in recorded["edges"].as_array_mut().unwrap() {
        edge.as_object_mut().unwrap().remove("line");
    }
    let input = write_tmp("diff/go-lineless-input.json", &recorded.to_string());
    let rules = write_tmp("diff/go-lineless.yaml", GO_CLAIM);
    let (mut held, broken, _) = scan(&[
        dir.to_str().unwrap(),
        "--collector-input",
        &input,
        "--rules",
        &rules,
    ]);
    held["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|e| !(e["from"] == "domain/order.go" && e["to"] == "adapters/postgres/repo.go"));
    let broken = write_tmp("diff/go-lineless-broken.json", &broken);
    let held = write_tmp("diff/go-lineless-held.json", &held.to_string());
    let (doc, _) = run_json(&["diff", &held, &broken]);
    assert_eq!(
        doc["target"]["newly_broken"],
        json!([{"claim": "no-adapters", "reason": "the domain stays pure", "slice": "*",
                "imports": [{"from": "domain/order.go", "to": "adapters/postgres/repo.go", "line": 3}]}])
    );
}
