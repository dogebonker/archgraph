use super::*;

#[derive(Args)]
pub(super) struct SyncRules {
    #[arg(default_value = ".")]
    root: PathBuf,
    /// Exit 1 when an owned block is missing or stale; write nothing
    #[arg(long)]
    check: bool,
    /// Include every rule rather than targeting 600 estimated tokens
    #[arg(long)]
    full: bool,
}
pub(super) fn run(args: SyncRules) -> Result<ExitCode> {
    let report = archgraph::sync_rules::run(&args.root, args.full, args.check)?;
    if globals().brief {
        write(None, &report)?;
    } else {
        for file in &report.written {
            println!("Wrote owned block in {file}");
        }
        if args.check {
            for file in &report.stale {
                println!("Stale: {file}");
            }
        }
        for conflict in &report.conflicts {
            eprintln!(
                "Refusing hand-edited block in {}:\n{}",
                conflict.path, conflict.diff
            );
        }
    }
    Ok(if !report.conflicts.is_empty() && !args.check {
        ExitCode::from(2)
    } else if args.check && !report.stale.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
