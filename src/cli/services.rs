use super::*;

#[derive(Args)]
pub(super) struct ServicesArgs {
    /// Repository directory to infer, or a declarative YAML/JSON service manifest
    path: Option<PathBuf>,
    /// Override inferred service boundaries and add declarations and forbid rules
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Markdown)]
    format: Format,
    /// Save the JSON report as well as printing the selected format
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Exit 1 for observed forbidden calls or clients. Requires authored forbid rules
    #[arg(long)]
    fail_on_forbidden: bool,
}

pub(super) fn run(args: ServicesArgs) -> Result<ExitCode> {
    if globals().legacy_v1 && (matches!(args.format, Format::Json) || args.output.is_some()) {
        bail!("services JSON requires the current schema; omit --legacy-v1");
    }
    if globals().granularity.is_some() {
        bail!("services use explicit service IDs; --granularity applies to import scans");
    }
    let path = args.path.unwrap_or_else(|| {
        if Path::new("services.yaml").is_file() {
            "services.yaml".into()
        } else {
            ".".into()
        }
    });
    let directory = path.is_dir();
    if !directory && args.manifest.is_some() {
        bail!("--manifest requires a directory input");
    }
    let manifest_path = if directory {
        args.manifest.as_deref()
    } else {
        Some(path.as_path())
    };
    let manifest = manifest_path
        .map(|manifest| {
            let text = fs::read_to_string(manifest)
                .with_context(|| format!("reading service declarations {}", manifest.display()))?;
            archgraph::services::parse(&text)
        })
        .transpose()?;
    if let (Some(output), Some(input)) = (args.output.as_deref(), manifest_path)
        && output.canonicalize().ok().as_ref() == Some(&input.canonicalize()?)
    {
        bail!("output must differ from the service manifest");
    }
    let report = if directory {
        archgraph::infer_services::infer(&path, manifest)?
    } else {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("service manifest must have a UTF-8 filename")?;
        archgraph::services::analyze(manifest.context("missing service manifest")?, name)?
    };
    if args.fail_on_forbidden && report.forbid.is_empty() {
        bail!("--fail-on-forbidden requires authored forbid rules in a service manifest");
    }
    let failed = args.fail_on_forbidden
        && (!report.forbidden_calls.is_empty() || !report.forbidden_resources.is_empty());
    if let Some(path) = args.output.as_deref() {
        write(Some(path), &report)?;
    }
    match args.format {
        Format::Json => write(None, &report)?,
        Format::Markdown => {
            print!("{}", archgraph::services::markdown(&report));
        }
    }
    Ok(ExitCode::from(u8::from(failed)))
}
