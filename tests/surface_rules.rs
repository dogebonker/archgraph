use archgraph::model::{Edge, EdgeKind, Verdict};
use archgraph::rules::{self, Compiled, Rules};
use std::collections::BTreeMap;

fn compile(yaml: &str) -> Compiled {
    Compiled::new(&serde_yaml_ng::from_str::<Rules>(yaml).unwrap()).unwrap()
}

fn decision(rules: &Compiled, from: &str, to: &str) -> (Verdict, String) {
    let mut edges = vec![Edge {
        from: from.into(),
        to: to.into(),
        kind: EdgeKind::Static,
        verdict: None,
        rule: None,
        reach: None,
        ..archgraph::model::Edge::default()
    }];
    // Explicit module glob boundaries must survive coarse file grouping.
    rules.decide(
        &mut edges,
        &BTreeMap::from([(from.into(), "src".into()), (to.into(), "src".into())]),
    );
    (edges[0].verdict.unwrap(), edges[0].rule.clone().unwrap())
}

#[test]
fn entrypoints_reject_every_external_bypass_before_allow_or_grouping() {
    let rules = compile(
        "version: 1\nentrypoints:\n  src/billing/**: src/billing/index.ts\nallow:\n  - {from: '**', to: '**'}\n",
    );
    assert_eq!(
        decision(&rules, "src/cart/order.ts", "src/billing/internal/pay.ts"),
        (Verdict::Forbidden, "entrypoints".into())
    );
    assert_eq!(
        decision(&rules, "src/billing/pay.ts", "src/billing/internal/pay.ts").0,
        Verdict::Allowed
    );
    assert_eq!(
        decision(&rules, "src/cart/order.ts", "src/billing/index.ts").0,
        Verdict::Allowed
    );
}

#[test]
fn explicit_forbid_wins_and_legacy_public_keeps_allow_precedence() {
    let rules = compile(
        "version: 1\nentrypoints:\n  src/billing/**: src/billing/index.ts\nforbid:\n  - {id: explicit, from: '**', to: '**'}\n",
    );
    assert_eq!(
        decision(&rules, "src/cart/a.ts", "src/billing/private.ts").1,
        "explicit"
    );
    let rules = compile(
        "version: 1\npublic:\n  - {module: 'src/billing/**', entries: ['src/billing/index.ts']}\nallow:\n  - {from: '**', to: '**'}\n",
    );
    assert_eq!(
        decision(&rules, "src/cart/a.ts", "src/billing/private.ts").0,
        Verdict::Allowed
    );
}

#[test]
fn captures_bind_entrypoints_and_same_domain_is_internal() {
    let rules = compile("version: 1\nentrypoints:\n  src/<name>/**: src/<name>/index.ts\n");
    assert_eq!(
        rules.entrypoints_for("src/billing/private.ts"),
        Some(("src/billing/**".into(), vec!["src/billing/index.ts".into()]))
    );
    assert_eq!(
        rules.public_for("src/billing/private.ts"),
        rules.entrypoints_for("src/billing/private.ts")
    );
    assert_eq!(
        decision(&rules, "src/cart/a.ts", "src/billing/private.ts").0,
        Verdict::Forbidden
    );
    assert_eq!(
        decision(&rules, "src/billing/a.ts", "src/billing/private.ts").0,
        Verdict::Allowed
    );
}

#[test]
fn entrypoints_load_dynamic_keys_without_weakening_typo_checks() {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("surface-layout.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "version: 1\nentrypoints:\n  src/billing/**: src/billing/index.ts\n",
    )
    .unwrap();
    let rules = rules::load(&path).unwrap();
    assert_eq!(
        rules.doc.entrypoints["src/billing/**"],
        "src/billing/index.ts"
    );
    for yaml in [
        "version: 1\nentrypoint: {}\n",
        "version: 1\npublic:\n  - {module: '**', entris: []}\n",
    ] {
        std::fs::write(&path, yaml).unwrap();
        assert!(format!("{:#}", rules::load(&path).unwrap_err()).contains("unknown field"));
    }
    let legacy: Rules = serde_yaml_ng::from_str("version: 1").unwrap();
    assert!(legacy.entrypoints.is_empty());
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("entrypoints")
            .is_none()
    );
}

#[test]
fn entrypoints_reject_unsafe_paths_globs_and_unbound_captures() {
    for entry in [
        "",
        "/index.ts",
        "../index.ts",
        "src/../index.ts",
        "src//index.ts",
        "./index.ts",
        "C:/index.ts",
        "src\\index.ts",
        "src/*.ts",
        "src/{a,b}.ts",
        "src/[ab].ts",
        "src/?.ts",
        "src/<other>/index.ts",
        "src/\nindex.ts",
    ] {
        let rules = Rules {
            version: 1,
            entrypoints: BTreeMap::from([("src/<name>/**".into(), entry.into())]),
            ..Rules::default()
        };
        assert!(Compiled::new(&rules).is_err(), "accepted {entry:?}");
    }
    for module in ["src/[", "src/\n**"] {
        let rules = Rules {
            version: 1,
            entrypoints: BTreeMap::from([(module.into(), "src/index.ts".into())]),
            ..Rules::default()
        };
        assert!(Compiled::new(&rules).is_err(), "accepted {module:?}");
    }
}
