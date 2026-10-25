use super::*;
use boundrail::infer_layout::{self, Options};
use std::io::Write;

#[derive(Args)]
pub(super) struct InferLayoutArgs {
    /// Scan document (v1 or v1.1)
    graph: PathBuf,
    /// Minimum source-file × target-file pairs supporting a forbid proposal
    #[arg(long, default_value_t = 100, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    min_potential_edges: usize,
    /// Maximum observed file-pair density for a forbid proposal (0–1)
    #[arg(long, default_value_t = 0.01, value_parser = density)]
    max_density: f64,
    /// Maximum forbid proposals; zero emits layers only
    #[arg(long, default_value_t = 20)]
    max_rules: usize,
    /// Minimum sibling slices behind a drafted claim
    #[arg(long, default_value_t = 5, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(2..))]
    min_slices: usize,
    /// Minimum share of bound slices a drafted claim holds in (above 0, at most 1)
    #[arg(long, default_value_t = 0.8, value_parser = share)]
    min_share: f64,
    /// Create a draft file instead of printing; existing files are never overwritten
    #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "layout.yaml")]
    apply: Option<PathBuf>,
}

fn density(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or_else(|| "expected a finite density between 0 and 1".into())
}

fn share(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| *value > 0.0 && *value <= 1.0)
        .ok_or_else(|| "expected a share above 0 and at most 1".into())
}

pub(super) fn run(args: InferLayoutArgs) -> Result<()> {
    let input = read_scan(&args.graph)?;
    let draft = infer_layout::infer(
        &input,
        Options {
            min_potential_edges: args.min_potential_edges,
            max_density: args.max_density,
            max_rules: args.max_rules,
            min_slices: args.min_slices,
            min_share: args.min_share,
        },
    )?;
    match args.apply {
        Some(path) => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| {
                    format!(
                        "creating {}; choose a new path if it exists",
                        path.display()
                    )
                })?;
            file.write_all(draft.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!(
                "Wrote draft {}; review before using it with check",
                path.display()
            );
        }
        None => print!("{draft}"),
    }
    Ok(())
}
