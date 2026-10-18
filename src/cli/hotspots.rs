use super::*;

#[derive(Args)]
pub(super) struct HotspotsArgs {
    graph: PathBuf,
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, default_value = "12 months ago")]
    since: String,
    #[arg(long, default_value_t = 20, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    top: usize,
    #[arg(long, default_value_t = 30, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    max_files: usize,
    #[arg(long)]
    authors: bool,
    #[arg(long, value_enum, default_value_t = Format::Markdown)]
    format: Format,
    /// Save the enriched scan for rendering and subsequent commands
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(super) fn run(args: HotspotsArgs) -> Result<()> {
    let mut doc = recompute(&args.graph, None)?;
    let report = archgraph::hotspots::collect(
        &args.root,
        &doc,
        &archgraph::hotspots::Options {
            since: args.since,
            max_files: args.max_files,
            authors: args.authors,
        },
    )?;
    archgraph::apply_hotspots(&mut doc, report);
    if let Some(path) = args.output {
        write(Some(&path), &doc)?;
    }
    let report = doc.hotspots.as_ref().unwrap();
    match args.format {
        Format::Json => {
            let mut value = serde_json::to_value(report)?;
            value["schema"] = SCHEMA.into();
            value["kind"] = "hotspots".into();
            value["files"].as_array_mut().unwrap().truncate(args.top);
            value["findings"] = serde_json::to_value(&doc.findings)?;
            write(None, &value)
        }
        Format::Markdown => {
            println!(
                "Hotspots: {} commits, {} mechanical commits skipped, since {}",
                report.commits, report.skipped, report.since
            );
            for limitation in &report.limitations {
                println!("note: {limitation}");
            }
            for h in report.files.iter().take(args.top) {
                println!(
                    "\n{}: changed {} times by {} authors; {} lines added, {} removed",
                    h.path, h.commits, h.authors, h.lines_added, h.lines_deleted
                );
                println!(
                    "  churn {:.3} × (coupling {:.3} + cycle {:.1}) × (1 + authors {:.3}) = {:.3}",
                    h.churn, h.coupling, h.cycle, h.author_factor, h.score
                );
                if h.cycle_size > 0 {
                    println!(
                        "  in a {}-module cycle{}",
                        h.cycle_size,
                        if h.cut_candidate {
                            "; import cut candidate"
                        } else {
                            ""
                        }
                    );
                }
                for &index in &h.finding_indices {
                    let finding = &doc.findings[index];
                    println!(
                        "  finding {index}: {} — {}",
                        finding.kind.as_str(),
                        finding.subject
                    );
                }
            }
            println!(
                "\nFix-message concentration: {} of {} unique commits touch the top {} files (in-sample association).",
                report.validation.top_bugfix_commits,
                report.validation.bugfix_commits,
                report.validation.top_files
            );
            Ok(())
        }
    }
}
