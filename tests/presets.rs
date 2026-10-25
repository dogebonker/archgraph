use boundrail::presets::{self, Preset, Scope};

#[test]
fn all_twenty_preset_scope_policies_match_reviewed_goldens() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/presets");
    for (preset, name) in [
        (Preset::Layered, "layered"),
        (Preset::Hexagonal, "hexagonal"),
        (Preset::FeatureSliced, "feature-sliced"),
        (Preset::ModularMonolith, "modular-monolith"),
        (Preset::Minimal, "minimal"),
    ] {
        for (scope, label) in [
            (Scope::Demo, "demo"),
            (Scope::Internal, "internal"),
            (Scope::Production, "production"),
            (Scope::Regulated, "regulated"),
        ] {
            let rules = presets::rules(preset, scope, "src");
            boundrail::rules::Compiled::new(&rules).unwrap();
            let text = presets::render(preset, scope, &rules, &[]).unwrap();
            let path = directory.join(format!("{name}-{label}.yaml"));
            if std::env::var_os("UPDATE_ARCHGRAPH_PRESET_GOLDENS").is_some() {
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(&path, &text).unwrap();
            }
            assert_eq!(
                text,
                std::fs::read_to_string(path).unwrap(),
                "{name}/{label}"
            );
            assert!(
                rules
                    .forbid
                    .iter()
                    .all(|r| r.reason.as_ref().is_some_and(|r| !r.is_empty()))
            );
            assert!(text.contains("## How to change this"));
        }
    }
}

#[test]
fn scope_strictness_increases_and_root_paths_do_not_gain_dot_prefixes() {
    let demo = presets::thresholds(Scope::Demo);
    let regulated = presets::thresholds(Scope::Regulated);
    assert!(demo.file_size.max > regulated.file_size.max);
    assert!(demo.fan_in.max > regulated.fan_in.max);
    assert!(demo.module_reach.max > regulated.module_reach.max);
    let rules = presets::rules(Preset::Layered, Scope::Internal, ".");
    assert_eq!(rules.layers[0], "domain/**");
}
