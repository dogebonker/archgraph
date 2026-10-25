use super::*;
use boundrail::presets::{self, Preset, Scope};
use std::io::{IsTerminal, Write};

#[derive(Args)]
pub(super) struct Init {
    #[arg(default_value = ".")]
    root: PathBuf,
    #[arg(long, value_enum, default_value_t = Preset::Layered)]
    preset: Preset,
    #[arg(long, value_enum)]
    scope: Option<Scope>,
    /// Replace an existing layout.yaml after generating and validating the whole draft
    #[arg(long)]
    force: bool,
}

fn scope(value: Option<Scope>) -> Result<Scope> {
    if let Some(value) = value {
        return Ok(value);
    }
    if !std::io::stdin().is_terminal() {
        bail!("non-interactive init requires --scope demo|internal|production|regulated");
    }
    eprintln!("Scope [demo | internal | production | regulated] (internal):");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let value = answer.trim();
    Scope::from_str(if value.is_empty() { "internal" } else { value }, true)
        .map_err(anyhow::Error::msg)
}

fn merge(draft: &mut rules::Rules, inferred: rules::Rules) -> Vec<String> {
    let mut comments = Vec::new();
    for module in inferred.modules {
        if let Some(preset) = draft.modules.iter().find(|m| m.id == module.id) {
            if preset.path != module.path {
                comments.push(format!(
                    "CONFLICT: inferred module {} at {}, preset says {}",
                    module.id, module.path, preset.path
                ));
            }
        } else {
            draft.modules.push(module);
        }
    }
    let mut previous = None;
    for layer in inferred.layers {
        if let Some(index) = draft.layers.iter().position(|p| p == &layer) {
            if let Some((prior, name)) = &previous
                && *prior > index
            {
                comments.push(format!(
                    "CONFLICT: inferred {name} before {layer}, preset says {layer} before {name}"
                ));
            }
            previous = Some((index, layer));
        } else {
            draft.layers.push(layer);
        }
    }
    for rule in inferred.forbid {
        if !draft
            .forbid
            .iter()
            .any(|r| r.from == rule.from && r.to == rule.to)
        {
            draft.forbid.push(rule);
        }
    }
    comments
}

pub(super) fn run(args: Init) -> Result<()> {
    let root = args
        .root
        .canonicalize()
        .context("init requires an existing repository directory")?;
    anyhow::ensure!(root.is_dir(), "init root must be a directory");
    let path = root.join("layout.yaml");
    history::refuse_symlink(&path)?;
    anyhow::ensure!(
        !path.exists() || path.is_file(),
        "layout.yaml must be a regular file"
    );
    if path.exists() && !args.force {
        bail!(
            "{} exists; review it or explicitly use --force",
            path.display()
        );
    }
    let scope = scope(args.scope)?;
    let source_root = collect::source_dir(&root);
    let before = match collect::detect_language(&root, source_root) {
        Ok(_) => Some(scan_cli::live(&root, None)?),
        Err(error) if error.to_string().starts_with("no .ts/.js,") => None,
        Err(error) => return Err(error),
    };
    let mut draft = presets::rules(args.preset, scope, source_root);
    let mut comments = Vec::new();
    if let Some(doc) = &before {
        let input = serde_json::from_value(serde_json::to_value(doc)?)?;
        let inferred = boundrail::infer_layout::infer(
            &input,
            boundrail::infer_layout::Options {
                max_rules: if matches!(args.preset, Preset::Minimal) {
                    0
                } else {
                    20
                },
                ..Default::default()
            },
        )?;
        comments = merge(&mut draft, serde_yaml_ng::from_str(&inferred)?);
        let compiled = rules::Compiled::new(&draft)?;
        let modules = doc
            .files
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    compiled
                        .module_of(&f.path)
                        .unwrap_or_else(|| f.module.clone()),
                )
            })
            .collect();
        let mut edges = doc.edges.clone();
        compiled.decide(&mut edges, &modules);
        for edge in edges
            .iter()
            .filter(|e| e.verdict == Some(Verdict::Forbidden))
        {
            comments.push(format!(
                "CONFLICT: inferred import {} -> {}, preset says forbidden ({})",
                edge.from,
                edge.to,
                edge.rule.as_deref().unwrap_or("rule")
            ));
        }
    }
    comments.sort();
    comments.dedup();
    rules::Compiled::new(&draft)?;
    let rendered = presets::render(args.preset, scope, &draft, &comments)?;
    if args.force {
        write_file(&path, rendered.as_bytes())?;
    } else {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(rendered.as_bytes())?;
        file.sync_all()?;
    }
    let synced = boundrail::sync_rules::run(&root, false, false)?;
    if !synced.conflicts.is_empty() {
        for conflict in &synced.conflicts {
            eprintln!("{}", conflict.diff);
        }
        bail!(
            "layout written; hand-edited agent rule blocks were preserved; resolve them before syncing"
        );
    }
    let mut findings = Vec::new();
    let mut limitations = Vec::new();
    if before.is_some() {
        let doc = scan_cli::live(&root, Some(&path))?;
        let smells = boundrail::early_smells::check(&doc, Some(&root))?;
        for finding in &doc.findings {
            let reasons =
                if let boundrail::model::Evidence::ForbiddenEdge { edge } = &finding.evidence {
                    draft.reasons(edge.rule.as_deref().unwrap_or_default())
                } else {
                    vec![]
                };
            findings.push(serde_json::json!({"finding_id":finding.finding_id,"kind":finding.kind,"subject":finding.subject,"recipe":finding.recipe,"reasons":reasons}));
        }
        for finding in smells.violations {
            findings.push(serde_json::to_value(finding)?);
        }
        limitations = smells.limitations;
    }
    let total = findings.len();
    findings.truncate(3);
    if globals().brief {
        write(
            None,
            &serde_json::json!({"kind":"init","path":path,"preset":args.preset,"scope":scope,"status":if total == 0 {"clean"} else {"findings"},"total_findings":total,"findings":findings,"conflicts":comments,"limitations":limitations,"rule_files_written":synced.written}),
        )?;
    } else {
        println!("Wrote {} ({:?}, {:?})", path.display(), args.preset, scope);
        for file in synced.written {
            println!("Synced owned block in {file}");
        }
        if total == 0 {
            println!("clean");
        }
        for finding in findings {
            println!("{}", serde_json::to_string(&finding)?);
        }
        for comment in comments {
            println!("{comment}");
        }
        for limitation in limitations {
            eprintln!("limitation: {limitation}");
        }
    }
    Ok(())
}
