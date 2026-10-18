use archgraph::rules::{Compiled, Rules};
use serde_json::Value;

fn select(yaml: &str, paths: &[&str]) -> Value {
    let doc: Rules = serde_yaml_ng::from_str(yaml).unwrap();
    Compiled::new(&doc)
        .unwrap()
        .relevant(&doc, &paths.iter().map(|p| (*p).into()).collect::<Vec<_>>())
        .unwrap()
}

#[test]
fn binds_boundary_entries_and_preserves_kind_constraints() {
    let selected = select(
        "version: 1\nmodules:\n  - { id: 'domains/<name>', path: 'src/<name>/**' }\npublic:\n  - { module: 'src/<name>/**', entries: ['src/<name>/index.ts'], kinds: [runtime] }\nentrypoints:\n  'src/<name>/**': 'src/<name>/index.ts'\n",
        &["src/billing/new.ts", "src/billing/new.ts"],
    );
    assert_eq!(selected["modules"][0]["id"], "domains/billing");
    assert_eq!(selected["public"][0]["module"], "src/billing/**");
    assert_eq!(selected["public"][0]["entries"][0], "src/billing/index.ts");
    assert_eq!(selected["public"][0]["kinds"][0], "runtime");
    assert_eq!(selected["entrypoints"][0]["entry"], "src/billing/index.ts");
    assert_eq!(
        selected["public"][0]["matched_paths"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn retains_inbound_and_outbound_rules_even_when_other_endpoint_is_unrequested() {
    let yaml = "version: 1\nforbid:\n  - { id: outbound, from: 'src/app/**', to: 'src/db/**' }\n  - { id: inbound, from: 'src/ui/**', to: 'src/app/**' }\n  - { id: irrelevant, from: 'src/x/**', to: 'src/y/**' }\nallow:\n  - { from: 'src/app/**', to: 'src/shared/**' }\n  - { from: 'src/cli/**', to: 'src/app/**' }\n";
    let selected = select(yaml, &["src/app/new.ts"]);
    assert_eq!(selected["forbid"].as_array().unwrap().len(), 2);
    assert_eq!(selected["forbid"][0]["id"], "outbound");
    assert_eq!(selected["forbid"][1]["id"], "inbound");
    assert_eq!(selected["allow"].as_array().unwrap().len(), 2);
}

#[test]
fn binds_capture_from_inbound_target() {
    let selected = select(
        "version: 1\nforbid:\n  - { from: 'src/<name>/ui/**', to: 'src/<name>/db/**' }\n",
        &["src/billing/db/new.ts"],
    );
    assert_eq!(selected["forbid"][0]["from"], "src/billing/ui/**");
    assert_eq!(selected["forbid"][0]["to"], "src/billing/db/**");
}

#[test]
fn retains_global_layer_order_defaults_and_referenced_categories() {
    let yaml = "version: 1\ndefault: forbidden\nlayers: ['src/domain/**', 'src/app/**']\npkg-category:\n  orm: [typeorm, prisma]\n  ui: [react]\nforbid:\n  - { from: 'src/domain/**', to: 'category:orm' }\n";
    let selected = select(yaml, &["src/domain/new.ts"]);
    assert_eq!(selected["default"], "forbidden");
    assert_eq!(selected["edge_weights"]["lazy"], 0.3);
    assert_eq!(selected["layers"][1]["index"], 1);
    assert_eq!(selected["pkg-category"]["orm"][0], "typeorm");
    assert!(selected["pkg-category"].get("ui").is_none());
    let doc: Rules = serde_yaml_ng::from_str(yaml).unwrap();
    let compiled = Compiled::new(&doc).unwrap();
    assert_eq!(
        compiled.layer_for("src/app/new.ts"),
        Some((1, "src/app/**".into()))
    );
    assert_eq!(compiled.layer_for("other/new.ts"), None);
}

#[test]
fn reachable_rules_are_explicitly_conservative_and_selection_is_deterministic() {
    let yaml = "version: 1\nforbid:\n  - { from: 'src/browser/**', to: 'src/server/**', reachable: true }\n";
    let a = select(yaml, &["src/shared/b.ts", "src/shared/a.ts"]);
    let b = select(
        yaml,
        &["src/shared/a.ts", "src/shared/b.ts", "src/shared/b.ts"],
    );
    assert_eq!(a, b);
    assert_eq!(a["forbid"].as_array().unwrap().len(), 1);
    assert!(
        a["forbid"][0]["selection"]
            .as_str()
            .unwrap()
            .starts_with("conservative:")
    );
}

#[test]
fn rejects_escaping_control_and_glob_paths_but_accepts_new_files() {
    let doc: Rules = serde_yaml_ng::from_str("version: 1").unwrap();
    let compiled = Compiled::new(&doc).unwrap();
    for path in [
        "../secret",
        "/tmp/a",
        "a/../b",
        "a\nb",
        "src/**",
        "src/{a,b}.ts",
        "a\\b",
    ] {
        assert!(compiled.relevant(&doc, &[path.into()]).is_err(), "{path:?}");
    }
    assert!(
        compiled
            .relevant(&doc, &["src/not-created-yet.ts".into()])
            .is_ok()
    );
}

#[test]
fn selecting_an_entry_retains_its_boundary_and_overlapping_layers_use_first_match() {
    let yaml = "version: 1\nlayers: ['src/**', 'src/app/**']\npublic:\n  - { module: 'src/private/**', entries: ['api/public.ts'] }\nentrypoints:\n  'src/private/**': 'api/public.ts'\n";
    let selected = select(yaml, &["api/public.ts"]);
    assert_eq!(selected["public"][0]["module"], "src/private/**");
    assert_eq!(selected["entrypoints"][0]["module"], "src/private/**");
    let doc: Rules = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(
        Compiled::new(&doc).unwrap().layer_for("src/app/new.ts"),
        Some((0, "src/**".into()))
    );
}
