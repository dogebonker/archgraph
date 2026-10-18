use super::*;
use archgraph::context::{self, Options};

#[derive(Args)]
pub(super) struct ContextArgs {
    /// Saved scan document; scan with --symbols for export summaries
    #[arg(long, default_value = ".archgraph/archgraph.json")]
    graph: PathBuf,
    /// Source checkout corresponding to the graph
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, allow_hyphen_values = true)]
    task: String,
    /// Seed files, relative to root or absolute paths inside it
    #[arg(long, num_args = 1..)]
    files: Vec<String>,
    /// Additional names to match against the saved export index
    #[arg(long, num_args = 1..)]
    symbols: Vec<String>,
    /// Estimated token budget; k means 1000, m means 1000000
    #[arg(long, default_value = "40k", value_parser = budget)]
    budget: usize,
    #[arg(long, default_value_t = 3, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    depth: usize,
    #[arg(long, value_enum, default_value_t = Format::Json)]
    format: Format,
}

fn budget(value: &str) -> std::result::Result<usize, String> {
    let (digits, scale) = match value.as_bytes().last() {
        Some(b'k' | b'K') => (&value[..value.len() - 1], 1_000),
        Some(b'm' | b'M') => (&value[..value.len() - 1], 1_000_000),
        _ => (value, 1),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("budget must be a positive integer, optionally followed by k or m".into());
    }
    digits
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_mul(scale))
        .filter(|number| *number > 0)
        .ok_or_else(|| "budget must be positive and fit in an integer".into())
}

pub(super) fn run(args: ContextArgs) -> Result<()> {
    if matches!(args.format, Format::Json) && LEGACY_V1.load(Ordering::Relaxed) {
        bail!("context JSON requires archgraph/v1.1; omit --legacy-v1");
    }
    let root = args
        .root
        .canonicalize()
        .context("cannot read context root")?;
    if !root.is_dir() {
        bail!("context root must be a directory");
    }
    let mut files = Vec::new();
    for file in args.files {
        let path = Path::new(&file);
        if path.is_absolute() {
            let canonical = path
                .canonicalize()
                .context("cannot read absolute seed file")?;
            let relative = canonical
                .strip_prefix(&root)
                .context("seed file is outside context root")?;
            files.push(relative.to_string_lossy().replace('\\', "/"));
        } else {
            files.push(file);
        }
    }
    let input = read_scan(&args.graph)?;
    let verified = scanned_checkout(&input.source, &root, &[&args.graph]);
    let mut report = context::select(
        &input,
        &root,
        &Options {
            task: args.task,
            files,
            symbols: args.symbols,
            budget: args.budget,
            depth: args.depth,
        },
    )?;
    // Unlike package-splits, a stale checkout still yields useful context, so it is not an error.
    match verified {
        Ok(true) => {}
        Ok(false) => report.limitations.push("Source root is unverified: the graph lacks Git or scan-root provenance, or the scanned/current checkout is dirty; source text may differ from the graph.".into()),
        Err(error) => report.limitations.push(format!("Source text may differ from the graph: {error:#}")),
    }
    match args.format {
        Format::Json => write(None, &report),
        Format::Markdown => {
            print!("{}", context::markdown(&report));
            Ok(())
        }
    }
}
