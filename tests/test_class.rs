use archgraph::model::{Config, Edge, Language, ModuleClass, Source};
use serde_json::json;

#[test]
fn included_tests_never_close_cycles_and_report_direct_import_coverage() {
    for (language, test, implementation) in [
        (
            Language::Typescript,
            "app/service.spec.ts",
            "app/service.ts",
        ),
        (Language::Python, "tests/test_service.py", "app/service.py"),
        (Language::Go, "app/service_test.go", "app/service.go"),
    ] {
        let source: Source = serde_json::from_value(json!({"language":language,"collector":{"name":"test","version":null,"cruised":3},"git":null,"generated_at":"test"})).unwrap();
        let config: Config = serde_json::from_value(
            json!({"depth":1,"source_root":".","excludes":[],"include_tests":true,"rules":null}),
        )
        .unwrap();
        let edges = [(test, implementation), (implementation, test)]
            .into_iter()
            .map(|(from, to)| Edge {
                from: from.into(),
                to: to.into(),
                ..Default::default()
            })
            .collect();
        let doc = archgraph::analyze(
            source,
            config,
            vec![
                test.into(),
                implementation.into(),
                "untested/other.ts".into(),
            ],
            edges,
            vec![],
            None,
        )
        .unwrap();
        assert!(doc.cycles.is_empty(), "{language:?}");
        assert!(doc.type_only_cycles.is_empty());
        assert_eq!(
            doc.files.iter().find(|f| f.path == test).unwrap().class,
            Some(ModuleClass::Tests)
        );
        let coverage = doc.test_coverage.as_ref().unwrap();
        assert_eq!((coverage.modules, coverage.covered), (2, 1));
        assert_eq!(coverage.without_importing_tests, ["untested"]);
        let restored = archgraph::restore_scan(
            serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap(),
        )
        .unwrap();
        assert!(restored.cycles.is_empty());
        assert!(
            restored
                .modules
                .iter()
                .any(|m| m.class == Some(ModuleClass::Tests))
        );
        let mut legacy = serde_json::to_value(&doc).unwrap();
        archgraph::compat::v1(&mut legacy).unwrap();
        assert!(legacy["cycles"].as_array().unwrap().is_empty());
        assert_eq!(legacy["test_coverage"]["covered"], 1);
        let legacy_input: archgraph::model::ScanInput = serde_json::from_value(legacy).unwrap();
        for file in &legacy_input.files {
            assert_eq!(file.class, None);
            assert_eq!(
                file.module,
                archgraph::modules::assigned(&file.path, &legacy_input.config, language, None)
            );
        }
        let migrated = archgraph::restore_scan(legacy_input).unwrap();
        assert!(migrated.cycles.is_empty());
    }
}
