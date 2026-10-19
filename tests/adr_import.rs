mod common;
use archgraph::{adr, collect::TempDir};
use serde_json::Value;
use std::fs;
use std::process::{Command, Output};

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_archgraph"))
        .arg("--no-daemon")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn review_apply_check_and_recipe_retain_adr_provenance() {
    let fixture = TempDir::new("adr-import-cli").unwrap();
    let root = fixture.path();
    for path in ["src/domain", "src/adapters", "docs/adr"] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    fs::write(
        root.join("src/domain/a.ts"),
        "import { adapter } from '../adapters/b'; export const value = adapter;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/adapters/b.ts"),
        "export const adapter = 1;\n",
    )
    .unwrap();
    fs::write(
        root.join("layout.yaml"),
        "# keep this comment\nversion: 1\nmodules:\n- id: domain\n  path: src/domain/**\n- id: adapters\n  path: src/adapters/**\n",
    )
    .unwrap();
    let adr_path = root.join("docs/adr/ADR-001.md");
    fs::write(&adr_path,"# ADR-001 Keep the domain independent\n\n## Status\nAccepted\n\n## Decision\ndomain must not import adapters.\n").unwrap();
    let original = fs::read(root.join("layout.yaml")).unwrap();
    let proposed = run(root, &["adr", "import", "docs/adr"]);
    success(&proposed);
    let fragment: Value = serde_yaml_ng::from_slice(&proposed.stdout).unwrap();
    assert_eq!(fragment["forbid"][0]["source"], "docs/adr/ADR-001.md");
    assert_eq!(original, fs::read(root.join("layout.yaml")).unwrap());
    let applied = run(root, &["adr", "import", "docs/adr", "--json", "--apply"]);
    success(&applied);
    let report: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(report["proposals"].as_array().unwrap().len(), 1);
    assert!(
        fs::read_to_string(root.join("layout.yaml"))
            .unwrap()
            .starts_with("# keep this comment\n")
    );
    let checked = run(root, &["check"]);
    assert_eq!(checked.status.code(), Some(1));
    let text = String::from_utf8_lossy(&checked.stdout);
    assert!(
        text.contains("ADR-001 Keep the domain independent"),
        "{text}"
    );
    assert!(text.contains("docs/adr/ADR-001.md"), "{text}");
    let scanned = run(
        root,
        &[
            "scan",
            ".",
            "--engine",
            "native",
            "--symbols",
            "--rules",
            "layout.yaml",
            "-o",
            "graph.json",
        ],
    );
    success(&scanned);
    let graph: Value = serde_json::from_slice(&fs::read(root.join("graph.json")).unwrap()).unwrap();
    assert!(graph["findings"].as_array().unwrap().iter().any(|finding| {
        finding["recipe"]["risk_notes"]
            .as_array()
            .is_some_and(|notes| {
                notes.iter().any(|note| {
                    note.as_str()
                        .is_some_and(|text| text.contains("Policy source: docs/adr/ADR-001.md"))
                })
            })
    }));
    for version in ["archgraph/v1", "archgraph/v1.1", "archgraph/v1.2"] {
        let mut legacy = graph.clone();
        if version == "archgraph/v1" {
            archgraph::compat::v1(&mut legacy).unwrap();
        } else {
            legacy["schema"] = version.into();
        }
        common::validate(&legacy);
        let restored = archgraph::restore_scan(serde_json::from_value(legacy).unwrap()).unwrap();
        assert_eq!(
            restored.config.rules.unwrap().doc.forbid[0]
                .source
                .as_deref(),
            Some("docs/adr/ADR-001.md")
        );
    }
    fs::write(
        adr_path,
        "# ADR-001 Keep the domain independent\n## Status\nDeprecated\n",
    )
    .unwrap();
    let stale = run(root, &["adr", "check"]);
    assert_eq!(stale.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(report["stale"].as_array().unwrap().len(), 1);
}

#[test]
fn source_metadata_is_optional_but_normalized_and_resolved_by_decision() {
    let fixture = TempDir::new("adr-source-rules").unwrap();
    let path = fixture.path().join("layout.yaml");
    for source in [
        "../secret.md",
        "/tmp/secret.md",
        "C:\\secret.md",
        "docs//adr.md",
    ] {
        let yaml = format!(
            "version: 1\nforbid:\n- from: src/a/**\n  to: src/b/**\n  source: {}\n",
            serde_json::to_string(source).unwrap()
        );
        fs::write(&path, yaml).unwrap();
        assert!(archgraph::rules::load(&path).is_err(), "{source}");
    }
    fs::write(&path,"version: 1\nforbid:\n- from: src/a/**\n  to: src/b/**\n  source: docs/ADR-001.md\nallow:\n- id: accepted-exception\n  from: src/a/**\n  to: src/c/**\n  source: docs/ADR-002.md\n").unwrap();
    let rules = archgraph::rules::load(&path).unwrap();
    assert_eq!(rules.doc.sources("forbid[0]"), ["docs/ADR-001.md"]);
    assert_eq!(rules.doc.sources("accepted-exception"), ["docs/ADR-002.md"]);
    assert!(rules.doc.sources("unrelated").is_empty());
    assert_eq!(adr::check(fixture.path()).unwrap().missing.len(), 2);
}

#[test]
fn pinned_lazy_dependency_rule_blocks_values_but_preserves_type_and_dynamic_imports() {
    let fixture = TempDir::new("adr-lazy-packages").unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir(root.join("adrs")).unwrap();
    fs::write(root.join("package.json"),r#"{"name":"adr-fixture","type":"module","dependencies":{"inquirer":"1","posthog-node":"1","@sentry/node-core":"1"}}"#).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/adrs/archgate-real/ARCH-018-lazy-load-heavy-dependencies.md"),
        root.join("adrs/ARCH-018.md"),
    )
    .unwrap();
    fs::write(root.join("src/static.ts"),"import value from 'inquirer'; import { PostHog } from 'posthog-node'; import * as Sentry from '@sentry/node-core'; export const all = [value, PostHog, Sentry];\n").unwrap();
    fs::write(root.join("src/allowed.ts"),"import type { Prompt } from 'inquirer'; export type Input = Prompt; export const load = () => import('inquirer');\n").unwrap();
    success(&run(root, &["adr", "import", "adrs", "--apply", "--json"]));
    success(&run(
        root,
        &[
            "scan",
            ".",
            "--engine",
            "native",
            "--rules",
            "layout.yaml",
            "-o",
            "graph.json",
        ],
    ));
    let graph: Value = serde_json::from_slice(&fs::read(root.join("graph.json")).unwrap()).unwrap();
    let edges = graph["external_edges"].as_array().unwrap();
    assert_eq!(
        edges
            .iter()
            .filter(|edge| edge["verdict"] == "forbidden")
            .count(),
        3,
        "{edges:?}"
    );
    for kind in ["type", "lazy"] {
        assert!(
            edges
                .iter()
                .any(|edge| edge["kind"] == kind && edge["verdict"] != "forbidden"),
            "{kind}: {edges:?}"
        );
    }
    assert_eq!(run(root, &["check"]).status.code(), Some(1));
    fs::remove_file(root.join("src/static.ts")).unwrap();
    success(&run(root, &["check"]));
}
