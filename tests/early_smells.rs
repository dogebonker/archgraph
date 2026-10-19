use archgraph::{
    early_smells::{self, Thresholds},
    model::{Config, Source},
    rules::Rules,
};
use serde_json::json;

fn graph(files: &[&str], pairs: &[(&str, &str)], limits: Thresholds) -> archgraph::model::Document {
    let config: Config = serde_json::from_value(json!({"depth":1,"source_root":".","excludes":[],"include_tests":false,"rules":{"path":"layout.yaml","sha256":"fixture","doc":{"version":1,"layers":["src/**"],"early-smells":limits}}})).unwrap();
    let source: Source = serde_json::from_value(json!({"language":"typescript","collector":{"name":"fixture","cruised":0},"generated_at":"test"})).unwrap();
    archgraph::analyze(
        source,
        config,
        files.iter().map(|p| (*p).into()).collect(),
        pairs
            .iter()
            .map(|(a, b)| archgraph::model::Edge {
                from: (*a).into(),
                to: (*b).into(),
                ..Default::default()
            })
            .collect(),
        vec![],
        None,
    )
    .unwrap()
}
#[test]
fn distinct_importers_production_fraction_layers_and_saved_limits() {
    let mut limits = Thresholds::default();
    limits.fan_in.max = 1;
    limits.module_reach.max = 30.0;
    let doc = graph(
        &["src/a.ts", "other/b.ts", "other/c.ts"],
        &[
            ("other/b.ts", "src/a.ts"),
            ("other/b.ts", "src/a.ts"),
            ("other/c.ts", "src/a.ts"),
        ],
        limits,
    );
    let report = early_smells::check(&doc, None).unwrap();
    assert_eq!(report.verdict, "BLOCK");
    assert_eq!(report.blocking_count, 4);
    assert_eq!(
        report
            .violations
            .iter()
            .filter(|v| v.rule == "fan-in")
            .count(),
        1
    );
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "module-reach" && v.evidence.contains("2/3"))
    );
    assert!(
        report
            .limitations
            .iter()
            .any(|l| l.contains("File size unavailable"))
    );
}
#[test]
fn rejects_invalid_caps_reasons_and_unknown_nested_keys() {
    for change in [
        json!({"file-size":{"max":0,"reason":"x"}}),
        json!({"module-reach":{"max":101,"reason":"x"}}),
        json!({"fan-in":{"max":2,"reason":" "}}),
        json!({"fan-in":{"max":2,"reason":"\u{001b}[31m"}}),
    ] {
        let mut value = serde_json::to_value(Thresholds::default()).unwrap();
        for (k, v) in change.as_object().unwrap() {
            value[k] = v.clone();
        }
        assert!(
            serde_json::from_value::<Thresholds>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let dir = archgraph::collect::TempDir::new("early-rules").unwrap();
    let path = dir.path().join("layout.yaml");
    std::fs::write(&path, "version: 1\nearly-smells:\n  typo: true\n").unwrap();
    assert!(
        archgraph::rules::load(&path)
            .unwrap_err()
            .to_string()
            .contains("parsing")
    );
    let rules: Rules =
        serde_json::from_value(json!({"version":1,"early-smells":Thresholds::default()})).unwrap();
    let compiled = archgraph::rules::Compiled::new(&rules).unwrap();
    assert!(compiled.relevant(&rules, &["src/a.ts".into()]).unwrap()["early-smells"].is_object());
}
#[test]
fn live_file_size_and_path_escape() {
    let dir = archgraph::collect::TempDir::new("early-size").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.ts"), "a\nb\n").unwrap();
    let mut limits = Thresholds::default();
    limits.file_size.max = 1;
    limits.single_implementation.enabled = false;
    let mut doc = graph(&["src/a.ts"], &[], limits);
    let report = early_smells::check(&doc, Some(dir.path())).unwrap();
    assert_eq!(report.blocking_count, 1);
    assert_eq!(report.violations[0].rule, "file-size");
    doc.files[0].path = "../escape.ts".into();
    assert!(early_smells::check(&doc, Some(dir.path())).is_err());
}
#[test]
fn explicit_interface_alias_through_barrel_counts_real_classes() {
    let dir = archgraph::collect::TempDir::new("early-implements").unwrap();
    for (path, body) in [
        (
            "port.ts",
            "export interface Port { run(): void }\nexport type Alias = Port;",
        ),
        (
            "index.ts",
            "export { type Port as PublicPort } from './port';",
        ),
        (
            "one.ts",
            "import type { PublicPort as Local } from './index'; export class One implements Local {run(){}}",
        ),
    ] {
        std::fs::write(dir.path().join(path), body).unwrap();
    }
    for count in [1, 2] {
        if count == 2 {
            std::fs::write(
                dir.path().join("two.ts"),
                "import type { Port } from './port'; export class Two implements Port {run(){}}",
            )
            .unwrap();
        }
        let policy = archgraph::collect::excludes::Policy::new(
            dir.path(),
            archgraph::model::Language::Typescript,
            Default::default(),
        )
        .unwrap();
        let out = archgraph::collect::native::typescript::collect(dir.path(), ".", &policy, true)
            .unwrap();
        let paths: Vec<_> = out.raw.files.iter().map(String::as_str).collect();
        let mut doc = graph(&paths, &[], Thresholds::default());
        doc.config.import_records = out.raw.imports.clone();
        doc.imports = out.raw.imports;
        doc.symbols = Some(out.symbols);
        let report = early_smells::check(&doc, Some(dir.path())).unwrap();
        let found: Vec<_> = report
            .violations
            .iter()
            .filter(|v| v.rule == "single-implementation")
            .collect();
        assert_eq!(found.len(), usize::from(count == 1), "{:?}", report);
        if count == 1 {
            assert_eq!(found[0].file, "port.ts");
            assert!(!found[0].blocking);
            assert_eq!(found[0].confidence, "medium");
            assert!(found[0].evidence.contains("One"));
        }
    }
}
