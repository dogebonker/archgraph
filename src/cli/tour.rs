use super::*;

#[derive(Args)]
pub(super) struct TourArgs {
    /// Saved scan document (v1 or v1.1)
    #[arg(default_value = ".boundrail/boundrail.json")]
    graph: PathBuf,
    /// Start with entrypoints by reversing dependency layers
    #[arg(long)]
    reverse: bool,
    /// Include saved export names and locations at each stop
    #[arg(long)]
    agent: bool,
}

pub(super) fn run(args: TourArgs) -> Result<()> {
    let input = read_scan(&args.graph)?;
    let tour = boundrail::tour::markdown(
        &input,
        &boundrail::tour::Options {
            reverse: args.reverse,
            agent: args.agent,
        },
    )?;
    print!("{tour}");
    Ok(())
}
