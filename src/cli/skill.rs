use super::*;

#[derive(Args)]
pub(super) struct Skill {
    #[command(subcommand)]
    command: Operation,
}

#[derive(Subcommand)]
enum Operation {
    /// Analyze source into an inactive, reviewable project architecture skill
    Draft {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Service policy inside the repository (defaults to services.yaml when present)
        #[arg(long)]
        manifest: Option<String>,
    },
    /// Install the reviewed draft without changing active policy or regenerating it
    Adopt {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
}

pub(super) fn run(args: Skill) -> Result<()> {
    match args.command {
        Operation::Draft { root, manifest } => {
            let root = root.canonicalize().context("resolving project root")?;
            let layout = archgraph::project_skill::layout(&root)?;
            let doc = scan_cli::live(&root, layout.as_deref())?;
            write(
                None,
                &archgraph::project_skill::draft(&root, &doc, manifest.as_deref())?,
            )
        }
        Operation::Adopt { root } => write(
            None,
            &serde_json::json!({
                "path": archgraph::project_skill::adopt(&root)?,
                "policy_written": false,
            }),
        ),
    }
}
