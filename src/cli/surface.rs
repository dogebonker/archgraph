use super::*;
use archgraph::surface::{self, SurfaceReport};

#[derive(Args)]
pub(super) struct SurfaceArgs {
    /// Scan document, collected with --symbols for symbol-level findings
    graph: PathBuf,
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Flag modules used externally through more than this many symbols
    #[arg(long, default_value_t = surface::DEFAULT_MAX_SYMBOLS, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    max_symbols: usize,
    #[arg(long, value_enum, default_value_t = Format::Markdown)]
    format: Format,
    /// Save the enriched graph for render/check
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(super) fn run(args: SurfaceArgs) -> Result<()> {
    let mut doc = recompute(&args.graph, args.rules.as_deref())?;
    doc.surface = Some(surface::analyze(&doc, args.max_symbols)?);
    if let Some(path) = &args.output {
        write(Some(path), &doc)?;
    }
    let report = doc.surface.as_ref().unwrap();
    if globals().brief {
        return write(None, report);
    }
    match args.format {
        Format::Json => write(None, report),
        Format::Markdown => {
            println!(
                "Public surface: {} modules; symbol coverage {}",
                report.modules.len(),
                if report.complete {
                    "complete within this scan"
                } else {
                    "incomplete"
                }
            );
            for module in &report.modules {
                println!(
                    "\n{}: {} exposed symbols, {} symbols used externally",
                    module.module,
                    module.exported.len(),
                    module.used_externally.len()
                );
                println!(
                    "  entrypoints: {}",
                    if module.entrypoints.is_empty() {
                        "direct file/package exports".into()
                    } else {
                        module.entrypoints.join(", ")
                    }
                );
                for export in &module.exported {
                    println!(
                        "  export {} ({}, defined {}::{})",
                        export.name, export.file, export.target.file, export.target.name
                    );
                }
                for usage in &module.used_externally {
                    println!(
                        "  used {}::{} by {}",
                        usage.target.file,
                        usage.target.name,
                        usage.importers.join(", ")
                    );
                }
            }
            print_findings(report);
            Ok(())
        }
    }
}

pub(super) fn print_findings(report: &SurfaceReport) {
    for module in &report.modules {
        for export in &module.could_be_internal {
            println!(
                "could be internal: {}::{} ({}) — no external consumer in this scan",
                module.module, export.name, export.file
            );
        }
        for import in &module.deep_imports {
            println!(
                "deep import: {} -> {} [{}]; {}",
                import.from,
                import.to,
                import.symbols.join(", "),
                import.suggestion
            );
        }
        if module.oversized {
            println!(
                "oversized surface: {} has {} externally used symbols (threshold {}); consider a facade",
                module.module,
                module.used_externally.len(),
                report.max_symbols
            );
        }
    }
    for note in &report.limitations {
        println!("note: {note}");
    }
}
