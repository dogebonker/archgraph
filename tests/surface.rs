mod common;

use archgraph::{
    model::{Config, Document, Edge, EdgeKind, Source},
    surface::{self, SymbolExport, SymbolImport, SymbolTarget, Symbols},
};
use serde_json::json;

fn document(files: &[&str], edges: &[(&str, &str)]) -> Document {
    let source: Source = serde_json::from_value(json!({
        "language": "typescript", "collector": {"name": "dependency-cruiser", "version": null, "cruised": files.len()},
        "git": null, "generated_at": "2026-10-17T00:00:00Z"
    })).unwrap();
    let config: Config = serde_json::from_value(json!({
        "depth": 1, "source_root": "src", "excludes": [], "include_tests": false, "rules": null
    }))
    .unwrap();
    archgraph::analyze(
        source,
        config,
        files.iter().map(|s| (*s).into()).collect(),
        edges
            .iter()
            .map(|(from, to)| Edge {
                from: (*from).into(),
                to: (*to).into(),
                kind: EdgeKind::Static,
                verdict: None,
                rule: None,
                reach: None,
                ..archgraph::model::Edge::default()
            })
            .collect(),
        Vec::new(),
        None,
    )
    .unwrap()
}
fn target(file: &str, name: &str) -> SymbolTarget {
    SymbolTarget {
        file: file.into(),
        name: name.into(),
    }
}
fn export(file: &str, name: &str, source: &str, original: &str) -> SymbolExport {
    SymbolExport {
        file: file.into(),
        name: name.into(),
        target: target(source, original),
    }
}
fn import(from: &str, to: &str, name: &str, canonical: Option<(&str, &str)>) -> SymbolImport {
    SymbolImport {
        from: from.into(),
        to: to.into(),
        name: name.into(),
        target: canonical.map(|(f, n)| target(f, n)),
        wildcard: false,
    }
}
fn symbols(exports: Vec<SymbolExport>, imports: Vec<SymbolImport>) -> Symbols {
    Symbols {
        complete: true,
        exports,
        imports,
        limitations: Vec::new(),
    }
}
fn module<'a>(report: &'a surface::SurfaceReport, name: &str) -> &'a surface::ModuleSurface {
    report.modules.iter().find(|m| m.module == name).unwrap()
}
fn api() -> Document {
    document(
        &[
            "src/billing/index.ts",
            "src/billing/internal.ts",
            "src/billing/local.ts",
            "src/app/main.ts",
        ],
        &[
            ("src/billing/index.ts", "src/billing/internal.ts"),
            ("src/app/main.ts", "src/billing/index.ts"),
            ("src/billing/local.ts", "src/billing/internal.ts"),
        ],
    )
}

#[test]
fn inferred_entries_respect_extension_specific_module_rules() {
    for (extension, entry, language) in [
        ("py", "__init__.py", "python"),
        ("tsx", "index.tsx", "typescript"),
    ] {
        let facade = format!("src/billing/{entry}");
        let implementation = format!("src/billing/internal.{extension}");
        let caller = format!("src/app/main.{extension}");
        let mut doc = document(
            &[&facade, &implementation, &caller],
            &[(&caller, &implementation)],
        );
        doc.source.language = serde_json::from_value(json!(language)).unwrap();
        let mut config = doc.config.clone();
        config.rules = Some(serde_json::from_value(json!({
            "path":"layout.yaml", "sha256":"0".repeat(64),
            "doc":{"version":1,"modules":[{"id":"payments","path":format!("src/billing/**/*.{extension}")}]}
        })).unwrap());
        let doc = archgraph::analyze(
            doc.source,
            config,
            doc.files.into_iter().map(|f| f.path).collect(),
            doc.edges,
            vec![],
            None,
        )
        .unwrap();
        let report = surface::analyze(&doc, 50).unwrap();
        let payments = module(&report, "payments");
        assert_eq!(payments.entrypoints, [facade]);
        assert_eq!(payments.deep_imports.len(), 1);
    }
}
fn api_exports() -> Vec<SymbolExport> {
    vec![
        export(
            "src/billing/index.ts",
            "Public",
            "src/billing/internal.ts",
            "Used",
        ),
        export(
            "src/billing/index.ts",
            "Unused",
            "src/billing/internal.ts",
            "Unused",
        ),
        export(
            "src/billing/internal.ts",
            "Used",
            "src/billing/internal.ts",
            "Used",
        ),
        export(
            "src/billing/internal.ts",
            "Unused",
            "src/billing/internal.ts",
            "Unused",
        ),
    ]
}
#[test]
fn aliases_are_compared_by_canonical_symbol_and_local_usage_is_not_external() {
    let mut doc = api();
    surface::attach(
        &mut doc,
        symbols(
            api_exports(),
            vec![
                import(
                    "src/app/main.ts",
                    "src/billing/index.ts",
                    "Public",
                    Some(("src/billing/internal.ts", "Used")),
                ),
                import(
                    "src/billing/local.ts",
                    "src/billing/internal.ts",
                    "Unused",
                    Some(("src/billing/internal.ts", "Unused")),
                ),
            ],
        ),
        1,
    )
    .unwrap();
    let report = doc.surface.as_ref().unwrap();
    assert!(report.complete);
    let billing = module(report, "billing");
    assert_eq!(billing.entrypoints, ["src/billing/index.ts"]);
    assert_eq!(billing.exported.len(), 2);
    assert_eq!(billing.used_externally.len(), 1);
    assert_eq!(billing.used_externally[0].target.name, "Used");
    assert_eq!(billing.used_externally[0].importers, ["src/app/main.ts"]);
    assert_eq!(billing.could_be_internal.len(), 1);
    assert_eq!(billing.could_be_internal[0].name, "Unused");
    assert!(!billing.oversized);
    assert!(billing.deep_imports.is_empty());
}
#[test]
fn deep_imports_list_importers_and_required_symbols_and_surface_threshold_is_strict() {
    let mut doc = api();
    doc.edges.push(Edge {
        from: "src/app/main.ts".into(),
        to: "src/billing/internal.ts".into(),
        kind: EdgeKind::Static,
        verdict: None,
        rule: None,
        reach: None,
        ..archgraph::model::Edge::default()
    });
    surface::attach(
        &mut doc,
        symbols(
            api_exports(),
            vec![
                import(
                    "src/app/main.ts",
                    "src/billing/internal.ts",
                    "Used",
                    Some(("src/billing/internal.ts", "Used")),
                ),
                import(
                    "src/app/main.ts",
                    "src/billing/internal.ts",
                    "Unused",
                    Some(("src/billing/internal.ts", "Unused")),
                ),
            ],
        ),
        1,
    )
    .unwrap();
    let billing = module(doc.surface.as_ref().unwrap(), "billing");
    assert!(billing.oversized);
    assert_eq!(billing.deep_imports.len(), 1);
    assert_eq!(billing.deep_imports[0].from, "src/app/main.ts");
    assert_eq!(billing.deep_imports[0].to, "src/billing/internal.ts");
    assert_eq!(billing.deep_imports[0].symbols, ["Unused", "Used"]);
    assert!(
        billing.deep_imports[0]
            .suggestion
            .contains("src/billing/index.ts")
    );
    assert!(billing.could_be_internal.is_empty());
    assert!(!module(&surface::analyze(&doc, 2).unwrap(), "billing").oversized);
}
#[test]
fn absent_or_incomplete_symbol_data_never_suggests_removing_exports() {
    let mut doc = api();
    let legacy = surface::analyze(&doc, 50).unwrap();
    assert!(!legacy.complete);
    assert!(legacy.limitations.iter().any(|s| s.contains("absent")));
    assert!(module(&legacy, "billing").could_be_internal.is_empty());
    let mut incomplete = symbols(api_exports(), Vec::new());
    incomplete.complete = false;
    incomplete
        .limitations
        .push("Dynamic __all__ could not be resolved".into());
    surface::attach(&mut doc, incomplete, 50).unwrap();
    let report = doc.surface.as_ref().unwrap();
    assert!(!report.complete);
    assert_eq!(module(report, "billing").exported.len(), 2);
    assert!(module(report, "billing").could_be_internal.is_empty());
    assert!(
        report
            .limitations
            .iter()
            .any(|s| s.contains("Dynamic __all__"))
    );
}
#[test]
fn namespace_consumers_conservatively_use_all_exports() {
    let mut doc = api();
    let mut namespace = import("src/app/main.ts", "src/billing/index.ts", "*", None);
    namespace.wildcard = true;
    surface::attach(&mut doc, symbols(api_exports(), vec![namespace]), 50).unwrap();
    let billing = module(doc.surface.as_ref().unwrap(), "billing");
    assert_eq!(billing.used_externally.len(), 2);
    assert!(billing.could_be_internal.is_empty());
}
#[test]
fn modules_without_facades_include_direct_file_exports() {
    let mut doc = document(
        &["src/api/used.ts", "src/api/unused.ts", "src/app/main.ts"],
        &[("src/app/main.ts", "src/api/used.ts")],
    );
    surface::attach(
        &mut doc,
        symbols(
            vec![
                export("src/api/used.ts", "Used", "src/api/used.ts", "Used"),
                export("src/api/unused.ts", "Unused", "src/api/unused.ts", "Unused"),
            ],
            vec![import(
                "src/app/main.ts",
                "src/api/used.ts",
                "Used",
                Some(("src/api/used.ts", "Used")),
            )],
        ),
        50,
    )
    .unwrap();
    let api = module(doc.surface.as_ref().unwrap(), "api");
    assert!(api.entrypoints.is_empty());
    assert_eq!(api.exported.len(), 2);
    assert_eq!(api.could_be_internal[0].name, "Unused");
    assert!(api.deep_imports.is_empty());
}
#[test]
fn nested_indexes_do_not_become_a_coarse_modules_facade() {
    let mut doc = document(
        &[
            "src/common/a/index.ts",
            "src/common/b/helper.ts",
            "src/app/main.ts",
        ],
        &[("src/app/main.ts", "src/common/b/helper.ts")],
    );
    surface::attach(
        &mut doc,
        symbols(
            vec![
                export("src/common/a/index.ts", "A", "src/common/a/index.ts", "A"),
                export("src/common/b/helper.ts", "B", "src/common/b/helper.ts", "B"),
            ],
            vec![import(
                "src/app/main.ts",
                "src/common/b/helper.ts",
                "B",
                Some(("src/common/b/helper.ts", "B")),
            )],
        ),
        50,
    )
    .unwrap();
    let common = module(doc.surface.as_ref().unwrap(), "common");
    assert!(common.entrypoints.is_empty());
    assert_eq!(common.exported.len(), 2);
    assert!(common.deep_imports.is_empty());
}
#[test]
fn symbol_targets_and_positive_threshold_are_validated() {
    let mut doc = api();
    assert!(
        surface::attach(
            &mut doc,
            symbols(
                vec![export("src/billing/index.ts", "Oops", "outside.ts", "Oops")],
                Vec::new()
            ),
            50
        )
        .is_err()
    );
    assert!(surface::analyze(&doc, 0).is_err());
    assert!(
        surface::attach(
            &mut doc,
            symbols(
                Vec::new(),
                vec![import("outside.ts", "src/billing/index.ts", "Oops", None)]
            ),
            50
        )
        .is_err()
    );
}
#[test]
fn declared_entrypoints_enforce_boundaries_even_without_symbol_data() {
    let mut doc = document(
        &[
            "src/billing/public.ts",
            "src/billing/internal.ts",
            "src/app/main.ts",
        ],
        &[("src/app/main.ts", "src/billing/internal.ts")],
    );
    doc.config.rules = Some(serde_json::from_value(json!({
        "path": "layout.yaml", "sha256": "fixture", "doc": {"version": 1, "entrypoints": {"src/billing/**": "src/billing/public.ts"}}
    })).unwrap());
    let report = surface::analyze(&doc, 50).unwrap();
    let billing = module(&report, "billing");
    assert_eq!(billing.entrypoints, ["src/billing/public.ts"]);
    assert_eq!(billing.deep_imports.len(), 1);
    assert!(!report.complete);
}

#[test]
fn bound_folder_names_match_literally_in_public_boundaries() {
    // `[slug]` is a glob class and a lone `[` an invalid glob unless the bound folder is escaped.
    for folder in ["[slug]", "[x"] {
        let page = format!("src/app/{folder}/page.tsx");
        let helper = format!("src/app/{folder}/x.ts");
        let mut doc = document(
            &[&page, &helper, "src/main.ts"],
            &[(&page, &helper), ("src/main.ts", &helper)],
        );
        doc.config.rules = Some(serde_json::from_value(json!({
            "path": "layout.yaml", "sha256": "fixture", "doc": {"version": 1, "public": [{"module": "src/app/<r>/**", "entries": ["src/app/<r>/page.tsx"]}]}
        })).unwrap());
        let report = surface::analyze(&doc, 50).unwrap();
        let app = module(&report, "app");
        assert_eq!(app.entrypoints, std::slice::from_ref(&page), "{folder}");
        let deep: Vec<_> = app.deep_imports.iter().map(|d| d.from.as_str()).collect();
        assert_eq!(deep, ["src/main.ts"], "{folder}");
        assert_eq!(
            app.deep_imports[0].suggestion,
            format!(
                "Import through {page}; re-export the required symbols there or declare a boundary rule."
            )
        );
        assert!(
            !report
                .limitations
                .iter()
                .any(|l| l.contains("declares entrypoint")),
            "{:?}",
            report.limitations
        );
    }
}

#[test]
fn restoring_a_scan_retains_symbol_evidence_and_surface_threshold() {
    let mut doc = api();
    surface::attach(&mut doc, symbols(api_exports(), Vec::new()), 3).unwrap();
    let saved = serde_json::to_value(&doc).unwrap();
    let restored = archgraph::restore_scan(serde_json::from_value(saved).unwrap()).unwrap();
    let surface = restored.surface.as_ref().unwrap();
    assert_eq!(surface.max_symbols, 3);
    assert!(surface.complete);
    assert_eq!(module(surface, "billing").could_be_internal.len(), 2);
    assert_eq!(restored.symbols.as_ref().unwrap().exports.len(), 4);
}

#[test]
fn restoring_surface_without_symbols_retains_the_report_and_threshold() {
    let mut doc = api();
    doc.surface = Some(surface::analyze(&doc, 7).unwrap());
    let restored = archgraph::restore_scan(
        serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(restored.symbols.is_none());
    let report = restored.surface.as_ref().unwrap();
    assert_eq!(report.max_symbols, 7);
    assert!(!report.complete);
    assert_eq!(
        module(report, "billing").entrypoints,
        ["src/billing/index.ts"]
    );
}

#[test]
fn symbol_names_and_limitations_reject_terminal_control_characters() {
    let mut invalid = Vec::new();
    let mut export_name = symbols(api_exports(), Vec::new());
    export_name.exports[0].name = "fake\u{1b}[2J".into();
    invalid.push(export_name);
    let mut target_name = symbols(api_exports(), Vec::new());
    target_name.exports[0].target.name = "fake\0".into();
    invalid.push(target_name);
    let mut import_name = symbols(
        Vec::new(),
        vec![import(
            "src/app/main.ts",
            "src/billing/index.ts",
            "fake\rforged",
            None,
        )],
    );
    invalid.push(import_name.clone());
    import_name.imports[0].name = "safe".into();
    import_name.imports[0].target = Some(target("src/billing/internal.ts", "fake\t"));
    invalid.push(import_name);
    let mut limitation = symbols(Vec::new(), Vec::new());
    limitation.limitations.push("safe\nforged finding".into());
    invalid.push(limitation);
    for evidence in invalid {
        let error = surface::attach(&mut api(), evidence, 50).unwrap_err();
        assert!(error.to_string().contains("control characters"), "{error}");
    }
}

#[test]
fn cli_surface_enrichment_survives_rules_check_schema_and_render() {
    use std::fs;
    let mut doc = document(
        &[
            "src/billing/index.ts",
            "src/billing/internal.ts",
            "src/app/main.ts",
        ],
        &[
            ("src/billing/index.ts", "src/billing/internal.ts"),
            ("src/app/main.ts", "src/billing/internal.ts"),
        ],
    );
    surface::attach(
        &mut doc,
        symbols(
            api_exports(),
            vec![
                import(
                    "src/app/main.ts",
                    "src/billing/internal.ts",
                    "Used",
                    Some(("src/billing/internal.ts", "Used")),
                ),
                import(
                    "src/app/main.ts",
                    "src/billing/internal.ts",
                    "Unused",
                    Some(("src/billing/internal.ts", "Unused")),
                ),
            ],
        ),
        50,
    )
    .unwrap();
    let input = common::write_tmp(
        "surface-cli-input.json",
        &serde_json::to_string(&doc).unwrap(),
    );
    let enriched = common::write_tmp("surface-cli-enriched.json", "");
    let output = common::archgraph(&[
        "surface",
        &input,
        "--format",
        "json",
        "--max-symbols",
        "1",
        "-o",
        &enriched,
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: surface::SurfaceReport = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.kind, "surface");
    assert_eq!(report.max_symbols, 1);
    assert!(module(&report, "billing").oversized);
    assert_eq!(module(&report, "billing").deep_imports.len(), 1);
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&enriched).unwrap()).unwrap();
    common::validate(&saved);
    assert_eq!(saved["surface"]["max_symbols"], 1);
    assert_eq!(saved["symbols"]["complete"], true);
    let rules = common::write_tmp(
        "surface-cli-layout.yaml",
        "version: 1\nentrypoints:\n  'src/billing/**': src/billing/index.ts\n",
    );
    let checked = common::write_tmp("surface-cli-checked.json", "");
    let output = common::archgraph(&["check", &enriched, "--rules", &rules, "-o", &checked]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("forbidden src/app/main.ts -> src/billing/internal.ts (entrypoints)"),
        "{text}"
    );
    assert!(
        text.contains("deep import: src/app/main.ts -> src/billing/internal.ts"),
        "{text}"
    );
    assert!(text.contains("oversized surface: billing"), "{text}");
    let checked: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(checked).unwrap()).unwrap();
    common::validate(&checked);
    let html = common::write_tmp("surface-cli-render.html", "");
    let output = common::archgraph(&["render", &enriched, "-o", &html]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = fs::read_to_string(html).unwrap();
    let marker = "<script type=\"application/json\" id=\"graph\">";
    let embedded = html
        .split_once(marker)
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let rendered: serde_json::Value = serde_json::from_str(embedded).unwrap();
    assert_eq!(rendered["surface"], saved["surface"]);
    assert_eq!(rendered["symbols"], saved["symbols"]);
}
