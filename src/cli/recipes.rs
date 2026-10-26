//! One recipe contract for the CLI, agents and mechanical application.
use super::*;
use boundrail::{apply as executor, recipes};
use serde_json::json;

#[derive(Args)]
pub(super) struct Target {
    finding_id: String,
    #[arg(long, default_value = ".")]
    pub(super) root: PathBuf,
    /// Optional saved graph; apply --yes always rescans its collection configuration
    #[arg(long)]
    graph: Option<PathBuf>,
    #[command(flatten)]
    opts: ScanOpts,
    #[command(flatten)]
    hotspots: HotspotOpts,
    #[command(flatten)]
    collector: CollectorOpts,
}
#[derive(Args)]
pub(super) struct Apply {
    #[command(flatten)]
    target: Target,
    #[arg(long, conflicts_with = "yes", required_unless_present = "yes")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run", required_unless_present = "dry_run")]
    yes: bool,
}
#[derive(Args)]
pub(super) struct Verify {
    #[command(flatten)]
    pub(super) target: Target,
    /// Structured predicate JSON; otherwise use the saved proposal/application contract
    #[arg(long)]
    expect: Option<String>,
}

fn root(args: &Target) -> Result<PathBuf> {
    if globals().legacy_v1 {
        bail!("recipe outputs require the current schema: omit --legacy-v1");
    }
    let root = args
        .root
        .canonicalize()
        .context("cannot read recipe root: pass --root <existing-repo>")?;
    if !root.is_dir() {
        bail!("recipe root is not a directory: pass --root <repo>");
    }
    Ok(root)
}
fn document(args: &Target, fresh: bool) -> Result<Document> {
    if args.opts.collector_input.is_some() && fresh {
        bail!("apply/verify require live source collection: omit --collector-input");
    }
    let root = root(args)?;
    let local_rules = root.join("layout.yaml");
    let rule_path = args
        .opts
        .rules
        .as_deref()
        .or_else(|| local_rules.is_file().then_some(local_rules.as_path()));
    let mut doc = if let Some(graph) = &args.graph {
        let original = recompute(graph, rule_path)?;
        if fresh {
            let engine = original
                .config
                .collection
                .as_ref()
                .map(|c| c.engine)
                .unwrap_or(collect::Engine::Reference);
            let collectors = provenance_cli::collector_opts(
                &root,
                &args.collector,
                engine,
                Some(original.source.language),
            )?;
            provenance_cli::probe_with_symbols(
                &root,
                &original,
                &collectors,
                args.opts.symbols || original.symbols.is_some(),
            )?
        } else {
            original
        }
    } else {
        let mut opts = args.opts.clone();
        opts.rules = rule_path.map(Path::to_owned);
        scan_cli::scan_doc(
            &root,
            &opts,
            &args.hotspots,
            &args.collector,
            opts.rules.as_deref(),
            &[],
        )?
    };
    recipes::rank(&mut doc, globals().rank);
    Ok(doc)
}

pub(super) fn propose(args: Target) -> Result<()> {
    let root = root(&args)?;
    let doc = document(&args, false)?;
    let mut recipe = recipes::recipe_for(&doc, &args.finding_id)?.clone();
    // Saved graphs remain useful offline; only verified source roots get a live baseline cache.
    let trusted =
        args.graph.is_none() || scanned_checkout(&doc.source, &root, &[]).unwrap_or(false);
    if trusted {
        recipe.verify.structured.baseline_id = Some(args.finding_id.clone());
        executor::remember_proposal(&root, &doc, &args.finding_id, &recipe)?;
    } else {
        recipe.risk_notes.push("Saved graph source is unverified: run propose without --graph in its checkout to save a live verification baseline.".into());
    }
    let mut value = serde_json::to_value(recipe)?;
    value["schema"] = json!(SCHEMA);
    value["kind"] = json!("recipe");
    value["finding_id"] = json!(args.finding_id);
    write(None, &value)
}
pub(super) fn apply(args: Apply) -> Result<ExitCode> {
    let root = root(&args.target)?;
    let before = document(&args.target, args.yes)?;
    let plan = executor::plan(&root, &before, &args.target.finding_id)?;
    if args.dry_run || !plan.can_apply {
        write(None, &plan)?;
        return Ok(if plan.can_apply {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(2)
        });
    }
    let report = executor::execute(&root, &before, &plan, || document(&args.target, true))?;
    if !report.verify.pass {
        eprintln!(
            "{}",
            report
                .hint
                .as_deref()
                .unwrap_or("verify: FAILED; rescan and review the recipe")
        );
    }
    write(None, &report)?;
    Ok(if report.verify.pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
pub(super) fn verify(args: Verify) -> Result<ExitCode> {
    let root = root(&args.target)?;
    let doc = document(&args.target, true)?;
    let verification = if let Some(expect) = &args.expect {
        let predicate: recipes::Predicate = serde_json::from_str(expect)
            .context("invalid --expect: pass the recipe.verify.structured JSON object")?;
        executor::verify_predicate(&root, &args.target.finding_id, &doc, &predicate)?
    } else {
        executor::verify_receipt(&root, &args.target.finding_id, &doc)?
    };
    write(
        None,
        &json!({"schema":SCHEMA,"kind":"verification","finding_id":args.target.finding_id,"verify":verification}),
    )?;
    Ok(if verification.pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
