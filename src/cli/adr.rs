use super::*;

#[derive(Args)]
pub(super) struct Adr {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Propose import rules from ADR decisions; does not execute companion checks
    Import {
        dir: PathBuf,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Append reviewed proposals to layout.yaml, preserving existing content
        #[arg(long)]
        apply: bool,
        /// Emit the complete JSON proposal inventory (use without --brief)
        #[arg(long)]
        json: bool,
    },
    /// Flag imported rules whose ADR is stale, missing, or has unknown lifecycle
    Check {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Also fail on forbid and allow rules that cite no ADR `source`
        #[arg(long)]
        require_source: bool,
    },
}
pub(super) fn run(args: Adr) -> Result<ExitCode> {
    match args.command {
        Command::Import {
            dir,
            root,
            apply,
            json,
        } => {
            let report = archgraph::adr::import(&root, &dir)?;
            if apply {
                archgraph::adr::apply(&root, &report)?;
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if globals().brief {
                write(None, &report)?;
            } else {
                print!("{}", archgraph::adr::yaml(&report)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            root,
            require_source,
        } => {
            let report = archgraph::adr::check(&root, require_source)?;
            let failed = !report.stale.is_empty()
                || !report.missing.is_empty()
                || report
                    .unsourced
                    .as_ref()
                    .is_some_and(|rules| !rules.is_empty());
            write(None, &report)?;
            Ok(if failed {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}
