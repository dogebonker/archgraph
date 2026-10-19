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
}

fn policy_target(doc: &archgraph::model::Document, path: &str) -> String {
    doc.external_nodes
        .iter()
        .find(|node| node.id == path)
        .map(|node| format!("pkg:{}", node.package))
        .unwrap_or_else(|| path.to_owned())
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
    let doc = if let Some(root) = &root {
        scan_cli::live(root, args.rules.as_deref().or(default_rules.as_deref()))?
    } else {
        recompute(&args.graph, args.rules.as_deref())?
    };
    if doc.config.rules.is_none() && doc.surface.is_none() {
        bail!("no rules: pass --rules, or scan with --rules");
    }
    let smells = archgraph::early_smells::check(&doc, root.as_deref())?;
    let incomplete = smells.verdict == "INCOMPLETE";
    if incomplete {
        eprintln!("incomplete source checks: run archgraph check <live repository directory>");
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
                "early_smells": smells,
                "reasons": forbidden.iter().map(|edge| serde_json::json!({"from":edge.from,"to":edge.to,"reasons":reasons(&doc, edge.rule.as_deref().unwrap_or_default())})).collect::<Vec<_>>(),
            }),
        )?;
        return Ok(if !forbidden.is_empty() || smells.blocking_count > 0 {
            ExitCode::from(1)
        } else if incomplete {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        });
    }
    let (mut forbidden, mut unplanned) = (0, 0);
    for e in doc.edges.iter().chain(&doc.external_edges) {
        let label = match e.verdict {
            Some(Verdict::Forbidden) => {
                forbidden += 1;
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
                    .map(|p| policy_target(&doc, p))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            None => String::new(),
        };
        let rule = e.rule.as_deref().unwrap_or_default();
        let written = e.original_to.as_deref().unwrap_or(&e.to);
        let resolved = if written != e.to {
            format!(" [resolves to {}]", policy_target(&doc, &e.to))
        } else {
            String::new()
        };
        println!(
            "{label} {} -> {} ({rule}){via}{resolved}{}",
            e.from,
            policy_target(&doc, written),
            reasons(&doc, rule)
                .into_iter()
                .map(|reason| format!(" reason={reason:?}"))
                .collect::<String>()
        );
    }
    println!("{forbidden} forbidden, {unplanned} unplanned");
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
    Ok(if forbidden > 0 || smells.blocking_count > 0 {
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
