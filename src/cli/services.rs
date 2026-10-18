use super::*;

#[derive(Args)]
pub(super) struct ServicesArgs {
    /// Declarative service graph, in YAML or JSON
    #[arg(default_value = "services.yaml")]
    manifest: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Markdown)]
    format: Format,
    /// Save the JSON report as well as printing the selected format
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(super) fn run(args: ServicesArgs) -> Result<()> {
    if globals().legacy_v1 && (matches!(args.format, Format::Json) || args.output.is_some()) {
        bail!("services JSON requires archgraph/v1.1; omit --legacy-v1");
    }
    if globals().granularity.is_some() {
        bail!("services use explicit service IDs; --granularity applies to import scans");
    }
    let text = fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading service declarations {}", args.manifest.display()))?;
    let path = args
        .manifest
        .file_name()
        .and_then(|name| name.to_str())
        .context("service manifest must have a UTF-8 filename")?;
    let manifest = archgraph::services::parse(&text)?;
    let report = archgraph::services::analyze(manifest, path)?;
    if let Some(path) = args.output.as_deref() {
        if path.canonicalize().ok().as_ref() == Some(&args.manifest.canonicalize()?) {
            bail!("output must differ from the service manifest");
        }
        write(Some(path), &report)?;
    }
    match args.format {
        Format::Json => write(None, &report),
        Format::Markdown => {
            use std::io::Write;
            std::io::stdout()
                .lock()
                .write_all(archgraph::services::markdown(&report).as_bytes())?;
            Ok(())
        }
    }
}
