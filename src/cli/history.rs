use super::*;
use boundrail::collect::elixir;
use sha2::Digest;

#[derive(Args)]
pub(super) struct HistoryArgs {
    /// Repository root (or a directory within it)
    #[arg(default_value = ".")]
    root: PathBuf,
    /// Revision or range to walk along its first-parent history
    #[arg(long, default_value = "HEAD")]
    rev: String,
    /// Maximum number of recent commits, ordered oldest to newest
    #[arg(long, default_value_t = 24, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    limit: usize,
    /// Monthly first commits, every-N commits, or tagged releases
    #[arg(long, value_enum, default_value_t = history::Sampling::Monthly)]
    sample: history::Sampling,
    #[arg(long, default_value_t = 1, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    every: usize,
    /// Start date as YYYY-MM-DD (00:00 UTC) or '<N> days/weeks/months/years ago' before the endpoint (default: two years)
    #[arg(long)]
    since: Option<String>,
    /// Keep cached points and scan only missing samples
    #[arg(long)]
    append: bool,
    /// Compare two release refs and print release-note Markdown
    #[arg(long, num_args = 2, value_names = ["OLD", "NEW"])]
    diff: Vec<String>,
    /// Verify the commits behind cycle increases (uses additional scan probes)
    #[arg(long)]
    annotate: bool,
    #[command(flatten)]
    opts: ScanOpts,
    #[command(flatten)]
    hotspots: HotspotOpts,
    #[command(flatten)]
    collector: CollectorOpts,
    /// Directory for history.json and history.html [default: <root>/.archgraph]
    #[arg(long)]
    out_dir: Option<PathBuf>,
}

pub(super) fn run(mut args: HistoryArgs) -> Result<()> {
    if args.opts.collector_input.is_some() {
        bail!(
            "history needs live collectors; --collector-input would reuse one graph for every commit"
        );
    }
    let language = match args.opts.language {
        Some(language) => language,
        None => collect::detect_language(&args.root, collect::source_dir(&args.root))?,
    };
    args.opts.language = Some(language);
    args.collector = provenance_cli::collector_opts(
        &args.root,
        &args.collector,
        args.opts.engine,
        Some(language),
    )?;
    // Explicit rules stay fixed across the timeline; otherwise each commit supplies its own.
    let rules = args
        .opts
        .rules
        .as_deref()
        .map(fs::canonicalize)
        .transpose()?;
    let scan = |root: &Path| {
        let local = root.join("layout.yaml");
        scan_cli::scan_doc(
            root,
            &args.opts,
            &args.hotspots,
            &args.collector,
            rules
                .as_deref()
                .or_else(|| local.is_file().then_some(local.as_path())),
            &[],
        )
    };
    if let [old, new] = args.diff.as_slice() {
        let resolve = |rev: &str| {
            boundrail::git::resolve(&args.root, rev)
                .with_context(|| format!("unknown release ref {rev}"))
        };
        let checkout = history::Checkout::new(&args.root)?;
        let old = scan(&checkout.at(&resolve(old)?)?)?;
        let new = scan(&checkout.at(&resolve(new)?)?)?;
        print!("{}", diff::markdown(&diff::diff(&old, &new), &[]));
        return Ok(());
    }
    let out_dir = args
        .out_dir
        .clone()
        .unwrap_or_else(|| args.root.join(".archgraph"));
    let identity = cache_identity(&args, language, rules.as_deref())?;
    let history_options = history::HistoryOptions {
        sampling: args.sample,
        limit: args.limit,
        every: args.every,
        since: args.since,
        cache_dir: Some(out_dir.join("history")),
        cache_key: identity,
        append: args.append,
    };
    let mut history = history::collect_with_options(&args.root, &args.rev, &history_options, scan)?;
    if args.annotate {
        let setup = std::cell::OnceCell::new();
        for i in 1..history.snapshots.len() {
            let (before, after) = history.snapshots.split_at_mut(i);
            let previous = &before[i - 1];
            let current = &mut after[0];
            if (current.summary.modules_in_cycles_percent
                > previous.summary.modules_in_cycles_percent
                || current.summary.cycles > previous.summary.cycles)
                && current
                    .graph
                    .findings
                    .iter()
                    .all(|f| f.provenance.is_none())
            {
                provenance_cli::enrich_from(
                    &args.root,
                    &args.collector,
                    &mut current.graph,
                    Some(previous.commit.clone()),
                    &setup,
                )?;
                history::persist_context(&args.root, &history_options, &current.graph)?;
            }
        }
    }
    let html = history::render(&history)?;
    let json = out_dir.join("history.json");
    let page = out_dir.join("history.html");
    history::refuse_symlink(&out_dir)?;
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    write(Some(&json), &history)?;
    write_file(&page, html.as_bytes())?;
    println!("commit       files modules cycles  % in cycles  cycles/100");
    for point in &history.snapshots {
        let s = &point.summary;
        println!(
            "{} {:>6} {:>7} {:>6} {:>12.1} {:>11.2}",
            &point.commit[..point.commit.len().min(10)],
            s.files,
            s.modules,
            s.cycles,
            s.modules_in_cycles_percent,
            s.cycles_per_100_modules
        );
    }
    println!(
        "{} snapshots\n{}\n{}",
        history.snapshots.len(),
        json.display(),
        page.display()
    );
    Ok(())
}

/// The cache key for history snapshots: scan options, global flags, this build and the
/// collector versions that can change a snapshot.
fn cache_identity(args: &HistoryArgs, language: Language, rules: Option<&Path>) -> Result<String> {
    let mut key = serde_json::to_vec(&(&args.opts, &args.hotspots, &args.collector))?;
    key.push(u8::from(globals().all_kinds));
    key.extend_from_slice(&serde_json::to_vec(&globals().granularity)?);
    key.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    key.extend_from_slice(include_bytes!("../../Cargo.lock"));
    key.extend_from_slice(&sha2::Sha256::digest(fs::read(std::env::current_exe()?)?));
    if let Some(path) = rules {
        key.extend_from_slice(&fs::read(path)?);
    }
    if args.opts.engine == collect::Engine::Native
        && language == Language::Go
        && let Ok(output) = collect::run(
            Command::new(&args.collector.go)
                .arg("version")
                .current_dir(std::env::temp_dir())
                .env("GOTOOLCHAIN", "local"),
        )
    {
        key.extend_from_slice(&output.stdout);
    }
    if args.opts.engine != collect::Engine::Native {
        // A caller-supplied collector may change in place without its path changing.
        let executable = match language {
            Language::Typescript => args.collector.depcruise.as_ref(),
            Language::Rust => args.collector.rust_analyzer.as_ref(),
            _ => None,
        };
        if let Some(executable) = executable {
            key.extend_from_slice(&fs::read(executable)?);
        }
        let runtime = match language {
            Language::Typescript => ("node", vec!["--version"]),
            Language::Python => (args.collector.python.as_str(), vec!["--version"]),
            Language::Go => (args.collector.go.as_str(), vec!["version"]),
            Language::Java => (args.collector.java.as_str(), vec!["-version"]),
            Language::Elixir => (args.collector.elixir.as_str(), vec!["--version"]),
            Language::Rust => ("rustup", vec!["show", "active-toolchain"]),
        };
        {
            let (program, version_args) = runtime;
            let mut command = Command::new(program);
            command
                .args(version_args)
                .current_dir(std::env::temp_dir())
                .env("RUSTUP_AUTO_INSTALL", "0");
            if language == Language::Elixir {
                elixir::sanitize(&mut command);
            }
            if let Ok(output) = collect::run(&mut command) {
                key.extend_from_slice(&output.stdout);
                key.extend_from_slice(&output.stderr);
            }
        }
        // Collector packages can change independently of their interpreter or toolchain label.
        for (query_language, program, version_args, cwd) in [
            (
                Language::Python,
                args.collector.python.clone(),
                vec![
                    "-I".to_owned(),
                    "-c".to_owned(),
                    "import importlib.metadata; print(importlib.metadata.version('grimp'))"
                        .to_owned(),
                ],
                std::env::temp_dir(),
            ),
            (
                Language::Typescript,
                args.collector
                    .depcruise
                    .as_ref()
                    .map_or_else(|| "npx".to_owned(), |p| p.to_string_lossy().into_owned()),
                if args.collector.depcruise.is_some() {
                    vec!["--version".to_owned()]
                } else {
                    vec![
                        "--no-install".to_owned(),
                        "depcruise".to_owned(),
                        "--version".to_owned(),
                    ]
                },
                fs::canonicalize(&args.root)?,
            ),
        ] {
            if language != query_language {
                continue;
            }
            if let Ok(output) =
                collect::run(Command::new(program).args(version_args).current_dir(cwd))
                && output.status.success()
            {
                key.extend_from_slice(&output.stdout);
            }
        }
        let analyzer = (language == Language::Rust)
            .then(|| {
                args.collector.rust_analyzer.clone().or_else(|| {
                    let output = collect::run(
                        Command::new("rustup")
                            .args(["which", "rust-analyzer"])
                            .current_dir(std::env::temp_dir())
                            .env("RUSTUP_AUTO_INSTALL", "0"),
                    )
                    .ok()?;
                    output
                        .status
                        .success()
                        .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
                })
            })
            .flatten();
        if let Some(analyzer) = analyzer
            && let Ok(output) = collect::run(
                Command::new(analyzer)
                    .arg("--version")
                    .current_dir(std::env::temp_dir()),
            )
            && output.status.success()
        {
            key.extend_from_slice(&output.stdout);
        }
        let mut package_roots = Vec::new();
        if language == Language::Typescript {
            package_roots.push(fs::canonicalize(&args.root)?);
        }
        if let Some(executable) = &args.collector.depcruise {
            package_roots.push(fs::canonicalize(executable)?);
        }
        for root in package_roots {
            for directory in root.ancestors() {
                for package in ["dependency-cruiser", "typescript"] {
                    if let Ok(bytes) = fs::read(
                        directory
                            .join("node_modules")
                            .join(package)
                            .join("package.json"),
                    ) {
                        key.extend_from_slice(&bytes);
                    }
                }
            }
        }
    }
    Ok(format!("{:?}", sha2::Sha256::digest(&key)))
}
