//! Day-zero policies; inference supplies repository-specific additions, never intent.
use anyhow::Result;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::early_smells::{Limit, Switch, Thresholds};
use crate::rules::{Forbid, ModuleRule, Rules};

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    Layered,
    Hexagonal,
    FeatureSliced,
    ModularMonolith,
    Minimal,
}
#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Demo,
    Internal,
    Production,
    Regulated,
}

pub fn thresholds(scope: Scope) -> Thresholds {
    let (lines, fan_in, reach, abstraction) = match scope {
        Scope::Demo => (1500, 100, 90.0, false),
        Scope::Internal => (1000, 60, 70.0, true),
        Scope::Production => (600, 30, 40.0, true),
        Scope::Regulated => (350, 15, 25.0, true),
    };
    Thresholds {
        file_size: Limit { max: lines, reason: "Keep a file small enough to review as one responsibility.".into() },
        fan_in: Limit { max: fan_in, reason: "Limit the number of source files affected by changing one file.".into() },
        module_reach: Limit { max: reach, reason: "Avoid a module becoming a dependency of most of the repository.".into() },
        single_implementation: Switch { enabled: abstraction, reason: "Review an abstraction with only one observed explicit implementation before adding indirection.".into() },
        every_file_in_layer: Switch { enabled: true, reason: "Every source file needs an explicit architectural home.".into() },
    }
}

pub fn rules(preset: Preset, scope: Scope, source_root: &str) -> Rules {
    let directory = |name: &str| {
        if source_root == "." || source_root.is_empty() {
            format!("{name}/**")
        } else {
            format!("{source_root}/{name}/**")
        }
    };
    let names: &[&str] = match preset {
        Preset::Layered => &["domain", "application", "adapters", "presentation"],
        Preset::Hexagonal => &["domain", "ports", "application", "adapters", "composition"],
        Preset::FeatureSliced => &["shared", "entities", "features", "widgets", "pages", "app"],
        Preset::ModularMonolith => &["shared", "modules", "composition"],
        Preset::Minimal => &[],
    };
    let mut rules = Rules {
        version: 1,
        layer_reason: Some("Foundational layers must not depend on outer layers.".into()),
        early_smells: Some(thresholds(scope)),
        grabbag: Some(crate::grabbag::Thresholds {
            fan_in_min: match scope {
                Scope::Demo => 20.0,
                Scope::Internal => 10.0,
                Scope::Production => 5.0,
                Scope::Regulated => 3.0,
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    for name in names {
        let path = directory(name);
        rules.modules.push(ModuleRule {
            id: (*name).into(),
            path: path.clone(),
        });
        rules.layers.push(path);
    }
    let boundaries: &[(&str, &str, &str)] = match preset {
        Preset::Layered => &[
            (
                "domain",
                "adapters",
                "Domain logic must not depend on adapter implementations.",
            ),
            (
                "application",
                "presentation",
                "Application workflows must not depend on presentation code.",
            ),
        ],
        Preset::Hexagonal => &[
            (
                "domain",
                "adapters",
                "Domain logic must remain independent of infrastructure adapters.",
            ),
            (
                "ports",
                "adapters",
                "Ports define contracts; adapters implement them.",
            ),
        ],
        Preset::FeatureSliced => &[
            (
                "shared",
                "features",
                "Shared foundations must not depend on feature behavior.",
            ),
            (
                "entities",
                "pages",
                "Entities must not depend on page composition.",
            ),
        ],
        Preset::ModularMonolith => &[
            (
                "shared",
                "modules",
                "Shared contracts must not depend on a business module.",
            ),
            (
                "modules",
                "composition",
                "Business modules must not depend on their composition root.",
            ),
        ],
        Preset::Minimal => &[],
    };
    for (from, to, reason) in boundaries {
        rules.forbid.push(Forbid {
            id: Some(format!("{from}-no-{to}")),
            from: directory(from),
            to: directory(to),
            reason: Some((*reason).into()),
            ..Default::default()
        });
    }
    if matches!(preset, Preset::Minimal) {
        rules.layers.push("**".into());
    }
    rules
}

pub fn render(preset: Preset, scope: Scope, rules: &Rules, comments: &[String]) -> Result<String> {
    let mut output = format!(
        "# archgraph preset: {preset:?}; scope: {scope:?}\n# Enforcement comes from imports; this draft needs an ownership review.\n# reason: Layer order keeps foundational code independent of outer concerns.\n# reason: Grab-bag thresholds surface weakly related, widely imported modules.\n#\n# ## How to change this\n# Rename paths to match your source tree; layers are foundation-first.\n# Review CONFLICT comments before adopting inferred boundaries.\n# Keep a reason on every explicit rule and rerun archgraph check after changes.\n# Single observed implementations are medium-confidence review candidates.\n"
    );
    for comment in comments {
        output.push_str("# ");
        output.push_str(&comment.replace(['\r', '\n'], " "));
        output.push('\n');
    }
    output.push_str(&serde_yaml_ng::to_string(rules)?);
    Ok(output)
}
