use std::collections::BTreeSet;

use boundrail::known;

use super::*;

#[derive(Args)]
pub(super) struct Check {
    /// Saved scan document, or a directory to check live
    #[arg(default_value = ".")]
    graph: PathBuf,
    /// Rules replacing the ones embedded in the document
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Write the recomputed document here
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Text, or SARIF 2.1.0 for GitHub code scanning (skips the agent-rule and early-smell checks)
    #[arg(long, value_enum, default_value_t = CheckFormat::Text)]
    format: CheckFormat,
    /// Rewrite the known-violations file beside the rules (layout.known.yaml), then check
    #[arg(long, value_enum)]
    update_known: Option<known::Mode>,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum CheckFormat {
    Text,
    Sarif,
}

pub(super) fn run(args: Check) -> Result<ExitCode> {
    let root = args
        .graph
        .is_dir()
        .then(|| args.graph.canonicalize())
        .transpose()?;
    let default_rules = root
        .as_ref()
        .map(|root| root.join("layout.yaml"))
        .filter(|p| p.is_file());
    let mut doc = if let Some(root) = &root {
        scan_cli::live(root, args.rules.as_deref().or(default_rules.as_deref()))?
    } else {
        recompute(&args.graph, args.rules.as_deref())?
    };
    if doc.config.rules.is_none() && doc.surface.is_none() {
        bail!("no rules: pass --rules, or scan with --rules");
    }
    if let Some(mode) = args.update_known {
        let rules = args
            .rules
            .as_deref()
            .or(default_rules.as_deref())
            .context("--update-known writes beside the rules file: pass --rules")?;
        let path = known::path(rules);
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let listed = doc.config.rules.as_ref().and_then(|r| r.known.as_ref());
        if mode == known::Mode::Shrink && listed.is_none() {
            bail!("no {name} to shrink: create it with boundrail check --update-known accept");
        }
        let entries = known::update(&doc, mode);
        let old: BTreeSet<_> = listed.into_iter().flat_map(|f| &f.entries).collect();
        let new: BTreeSet<_> = entries.iter().collect();
        let rules_name = rules
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        write_file(&path, known::render(&rules_name, &entries).as_bytes())?;
        eprintln!(
            "{name}: {} kept, {} dropped, {} added",
            old.intersection(&new).count(),
            old.difference(&new).count(),
            new.difference(&old).count()
        );
        if let Some(rules) = &mut doc.config.rules {
            rules.known = Some(known::File {
                path: name,
                entries,
            });
        }
        known::mark(&mut doc);
        doc.summary = simulate::summary(&doc);
    }
    if args.format == CheckFormat::Sarif {
        if let Some(path) = &args.output {
            write(Some(path), &doc)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&boundrail::sarif::sarif(&doc))?
        );
        return Ok(ExitCode::from(u8::from(known::fails(&doc))));
    }
    let active_policy = args.rules.as_deref().map(Path::canonicalize).transpose()?;
    let default_policy = default_rules
        .as_deref()
        .map(Path::canonicalize)
        .transpose()?;
    if let Some(root) = &root
        && default_policy.is_some()
        && (active_policy.is_none() || active_policy == default_policy)
    {
        match boundrail::sync_rules::run(root, false, true) {
            Ok(report) if !report.stale.is_empty() => eprintln!(
                "warning: generated agent rules are stale in {}; run boundrail sync-rules",
                report.stale.join(", ")
            ),
            Err(error) => eprintln!(
                "warning: cannot verify generated agent rules: {error:#}; run boundrail sync-rules --check"
            ),
            _ => (),
        }
    }
    let smells = boundrail::early_smells::check(&doc, root.as_deref())?;
    let incomplete = smells.verdict == "INCOMPLETE";
    if incomplete {
        eprintln!("incomplete source checks: run boundrail check <live repository directory>");
    }
    if globals().brief {
        let forbidden: Vec<_> = doc
            .edges
            .iter()
            .chain(&doc.external_edges)
            .filter(|edge| edge.verdict == Some(Verdict::Forbidden))
            .collect();
        let unplanned: Vec<_> = doc
            .edges
            .iter()
            .chain(&doc.external_edges)
            .filter(|edge| edge.verdict == Some(Verdict::Unplanned))
            .collect();
        if let Some(path) = &args.output {
            write(Some(path), &doc)?;
        }
        write(
            None,
            &serde_json::json!({
                "schema": SCHEMA, "kind": "check", "summary": doc.summary,
                "cycles": doc.cycles, "forbidden": forbidden, "unplanned": unplanned,
                "surface": doc.surface,
                "target": doc.target,
                "stale_known": doc.stale_known,
                "early_smells": smells,
                "reasons": forbidden.iter().map(|edge| serde_json::json!({"from":edge.from,"to":edge.to,"reasons":reasons(&doc, edge.rule.as_deref().unwrap_or_default()),"sources":sources(&doc, edge.rule.as_deref().unwrap_or_default())})).collect::<Vec<_>>(),
            }),
        )?;
        return Ok(if known::fails(&doc) || smells.blocking_count > 0 {
            ExitCode::from(1)
        } else if incomplete {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        });
    }
    let (mut forbidden, mut listed, mut unplanned) = (0, 0, 0);
    for e in doc.edges.iter().chain(&doc.external_edges) {
        let label = match e.verdict {
            Some(Verdict::Forbidden) => {
                forbidden += 1;
                if e.known {
                    listed += 1;
                    continue;
                }
                "forbidden"
            }
            Some(Verdict::Unplanned) => {
                unplanned += 1;
                "unplanned"
            }
            _ => continue,
        };
        let via = match &e.reach {
            Some(path) => format!(
                " via {}",
                path.iter()
                    .map(|p| known::policy_target(&doc, p))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            None => String::new(),
        };
        let rule = e.rule.as_deref().unwrap_or_default();
        let written = e.original_to.as_deref().unwrap_or(&e.to);
        let resolved = if written != e.to {
            format!(" [resolves to {}]", known::policy_target(&doc, &e.to))
        } else {
            String::new()
        };
        println!(
            "{label} {} -> {} ({rule}){via}{resolved}{}{}",
            e.from,
            known::policy_target(&doc, written),
            reasons(&doc, rule)
                .into_iter()
                .map(|reason| format!(" reason={reason:?}"))
                .collect::<String>(),
            sources(&doc, rule)
                .into_iter()
                .map(|source| format!(" source={source:?}"))
                .collect::<String>()
        );
    }
    for entry in &doc.stale_known {
        println!("{}", known::stale_line(entry));
    }
    match listed {
        0 => println!("{forbidden} forbidden, {unplanned} unplanned"),
        listed => println!("{forbidden} forbidden ({listed} known), {unplanned} unplanned"),
    }
    if let Some(target) = &doc.target {
        print!("{}", boundrail::target::text(target));
    }
    for finding in &smells.violations {
        println!(
            "{} {}: {} reason={:?}",
            if finding.blocking {
                "violation"
            } else {
                "advisory"
            },
            finding.rule,
            finding.evidence,
            finding.reason
        );
    }
    for limitation in &smells.limitations {
        eprintln!("limitation: {limitation}");
    }
    if let Some(surface) = &doc.surface {
        surface_cli::print_findings(surface);
    }
    if let Some(path) = &args.output {
        write(Some(path), &doc)?;
    }
    Ok(if known::fails(&doc) || smells.blocking_count > 0 {
        ExitCode::from(1)
    } else if incomplete {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

fn reasons<'a>(doc: &'a Document, rule: &str) -> Vec<&'a str> {
    let Some(rules) = &doc.config.rules else {
        return vec![];
    };
    rules.doc.reasons(rule)
}

fn sources<'a>(doc: &'a Document, rule: &str) -> Vec<&'a str> {
    doc.config
        .rules
        .as_ref()
        .map_or_else(Vec::new, |rules| rules.doc.sources(rule))
}
