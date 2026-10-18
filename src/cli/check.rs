use super::*;

#[derive(Args)]
pub(super) struct Check {
    /// Scan document
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
    let doc = recompute(&args.graph, args.rules.as_deref())?;
    if doc.config.rules.is_none() && doc.surface.is_none() {
        bail!("no rules: pass --rules, or scan with --rules");
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
            }),
        )?;
        return Ok(if forbidden.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
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
            "{label} {} -> {} ({rule}){via}{resolved}",
            e.from,
            policy_target(&doc, written)
        );
    }
    println!("{forbidden} forbidden, {unplanned} unplanned");
    if let Some(surface) = &doc.surface {
        surface_cli::print_findings(surface);
    }
    if let Some(path) = &args.output {
        write(Some(path), &doc)?;
    }
    Ok(if forbidden > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
