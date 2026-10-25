mod common;

use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn scan_owned(name: &str, codeowners: &str) -> (Value, String, String) {
    let edges = [
        ("src/a/g.ts", "src/a/f.ts"),
        ("src/t/t1.ts", "src/a/f.ts"),
        ("src/t/t2.ts", "src/a/f.ts"),
        ("src/t/t3.ts", "src/a/f.ts"),
    ]
    .map(|(a, b)| (a.to_owned(), b.to_owned()));
    let (root, input) = common::synthetic(name, 5, &edges);
    fs::create_dir_all(Path::new(&root).join("src")).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q", &root])
            .status()
            .unwrap()
            .success()
    );
    fs::write(Path::new(&root).join("CODEOWNERS"), codeowners).unwrap();
    let rules = common::write_tmp(&format!("{name}-rules.yaml"), "version: 1\n");
    let (doc, text, _) = common::scan(&[
        &root,
        "--language",
        "typescript",
        "--depth",
        "1",
        "--collector-input",
        &input,
        "--rules",
        &rules,
    ]);
    (doc, text, root)
}

#[test]
fn ownership_supports_placement_and_preserves_shared_responsibility() {
    let (doc, _, _) = scan_owned(
        "ownership-support",
        "/src/a/ @alpha\n/src/t/ @beta @shared\n/src/a/f.ts @beta\n",
    );
    let finding = common::finding(&doc, "misplaced-file", "src/a/f.ts");
    assert_eq!(finding["evidence"]["ownership"]["assessment"], "supports");
    assert_eq!(finding["suggestion"]["move"]["to"], "src/t/f.ts");
    let target = doc["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "t")
        .unwrap();
    assert_eq!(target["ownership"]["owned_files"], 3);
    assert_eq!(target["ownership"]["owners"]["@beta"], 3);
    assert_eq!(target["ownership"]["owners"]["@shared"], 3);
    assert_eq!(
        target["ownership"]["primary"],
        serde_json::json!(["@beta", "@shared"])
    );
}

#[test]
fn ownership_conflicts_keep_the_evidence_but_require_review_before_a_move() {
    let (doc, _, _) = scan_owned("ownership-conflict", "/src/a/ @alpha\n/src/t/ @beta\n");
    let finding = common::finding(&doc, "misplaced-file", "src/a/f.ts");
    assert_eq!(finding["evidence"]["ownership"]["assessment"], "conflicts");
    assert_eq!(
        finding["evidence"]["ownership"]["destination_owners"],
        serde_json::json!(["@beta"])
    );
    assert!(finding.get("suggestion").is_none());
    assert!(finding.get("simulation").is_none());
    assert_eq!(finding["evidence"]["imports_from_target"], 3);
}

#[test]
fn embedded_ownership_survives_recompute_and_simulated_destinations_reassign_owners() {
    let (doc, text, root) = scan_owned("ownership-embedded", "/src/a/ @alpha\n/src/t/ @beta\n");
    let graph = common::write_tmp("ownership-embedded/graph.json", &text);
    fs::remove_file(Path::new(&root).join("CODEOWNERS")).unwrap();
    let out = common::write_tmp("ownership-embedded/recomputed.json", "");
    assert!(
        common::archgraph(&["check", &graph, "-o", &out])
            .status
            .success()
    );
    let recomputed: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    common::validate(&recomputed);
    assert_eq!(recomputed["files"], doc["files"]);
    assert_eq!(recomputed["findings"], doc["findings"]);
    let input: boundrail::model::ScanInput = serde_json::from_str(&text).unwrap();
    let rules = boundrail::rules::compile(&input.config).unwrap();
    let owners = boundrail::codeowners::compile(input.config.codeowners.as_ref().unwrap()).unwrap();
    let before = boundrail::build(
        input.source,
        input.config,
        input.files.into_iter().map(|f| f.path).collect(),
        input.edges,
        input.package_entries,
        rules.as_ref(),
        Some(&owners),
    );
    let after = boundrail::simulate::apply(
        &before,
        rules.as_ref(),
        &[boundrail::model::Move {
            file: "src/a/f.ts".into(),
            to: "src/t/f.ts".into(),
        }],
    )
    .unwrap();
    assert_eq!(
        after
            .files
            .iter()
            .find(|f| f.path == "src/t/f.ts")
            .unwrap()
            .owners,
        ["@beta"]
    );
}

#[test]
fn unknown_ownership_does_not_invent_a_team_or_block_a_structural_move() {
    let (doc, _, _) = scan_owned("ownership-unknown", "/src/a/ @alpha\n");
    let finding = common::finding(&doc, "misplaced-file", "src/a/f.ts");
    assert_eq!(finding["evidence"]["ownership"]["assessment"], "unknown");
    assert!(finding.get("suggestion").is_some());
}

#[test]
fn registrar_moves_do_not_cross_declared_ownership_boundaries() {
    let mut edges = vec![("app/use.ts".to_owned(), "driver/pick.ts".to_owned())];
    for module in ["pg", "my", "sq", "ms", "or"] {
        edges.push(("driver/pick.ts".to_owned(), format!("driver/{module}/a.ts")));
        for file in ["a", "b", "c"] {
            edges.push((
                format!("driver/{module}/{file}.ts"),
                "driver/base.ts".to_owned(),
            ));
        }
    }
    let (root, input) = common::synthetic(
        &format!("ownership-registrar-{}", std::process::id()),
        edges.len() + 2,
        &edges,
    );
    assert!(
        Command::new("git")
            .args(["init", "-q", &root])
            .status()
            .unwrap()
            .success()
    );
    let scan = |extra: &[&str]| {
        common::scan(
            &[
                &[
                    root.as_str(),
                    "--language",
                    "typescript",
                    "--collector-input",
                    input.as_str(),
                ][..],
                extra,
            ]
            .concat(),
        )
        .0
    };
    let default = scan(&[]);
    assert!(default["cycles"].as_array().unwrap().is_empty());
    assert_eq!(default["grabbags"].as_array().unwrap().len(), 1);
    assert_eq!(default["grabbags"][0]["candidates"][0]["module"], "driver");
    // Keep this ownership test focused on the registrar move path.
    let rules = common::write_tmp(
        "ownership-registrar-layout.yaml",
        "version: 1\ngrabbag: {cohesion_max: 0.0}\n",
    );
    let before = scan(&["--rules", &rules]);
    assert!(
        !before["cycles"][0]["registrar_moves"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fs::write(
        Path::new(&root).join("CODEOWNERS"),
        "* @other\n/driver/pick.ts @driver\n",
    )
    .unwrap();
    let after = scan(&["--rules", &rules]);
    assert_eq!(
        after["cycles"].as_array().unwrap().len(),
        before["cycles"].as_array().unwrap().len()
    );
    for cycle in after["cycles"].as_array().unwrap() {
        assert!(
            cycle
                .get("registrar_moves")
                .is_none_or(|moves| moves.as_array().unwrap().is_empty())
        );
    }
}
