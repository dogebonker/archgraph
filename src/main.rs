use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static ALL_KINDS: AtomicBool = AtomicBool::new(false);
static LEGACY_V1: AtomicBool = AtomicBool::new(false);
static GRANULARITY: AtomicU8 = AtomicU8::new(0);

use anyhow::{Context, Result, bail};
use archgraph::collect::{self, depcruise, go, grimp, java, rust};
use archgraph::model::{
    Config, CycleLevel, Document, Git, Granularity, Language, SCHEMA, ScanInput, Source, Verdict,
};
use archgraph::{cochange, codeowners, diff, history, modules, render, rules, simulate};
use clap::{Args, Parser, Subcommand, ValueEnum};
use sha2::Digest;

#[path = "cli/context.rs"]
mod context_cli;
#[path = "cli/hotspots.rs"]
mod hotspots_cli;
#[path = "cli/impact.rs"]
mod impact_cli;
#[path = "cli/impact_context.rs"]
mod impact_context;
#[path = "cli/impact_typescript.rs"]
mod impact_typescript;
#[path = "cli/infer_layout.rs"]
mod infer_layout_cli;
#[path = "cli/package_splits.rs"]
mod package_splits_cli;
#[path = "cli/provenance.rs"]
mod provenance_cli;
#[path = "cli/services.rs"]
mod services_cli;
#[path = "cli/surface.rs"]
mod surface_cli;
#[path = "cli/tour.rs"]
mod tour_cli;

#[derive(Parser)]
#[command(version, about = "Import-graph evidence for architecture decisions")]
struct Cli {
    /// Include every import kind in cycle detection, for comparison
    #[arg(long, global = true)]
    all_kinds: bool,
    /// Export JSON using legacy v1 kind names and schema discriminator
    #[arg(long, global = true)]
    legacy_v1: bool,
    /// Graph module granularity; file overrides folder/layout grouping
    #[arg(long, global = true, value_enum)]
    granularity: Option<Granularity>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Collect the import graph and write a scan document
    Scan(Scan),
    /// Scan, apply rules, write archgraph.json and archgraph.html, and print a summary
    Report(Report),
    /// Recompute a scan document under rules and list forbidden and unplanned imports
    Check(Check),
    /// Write one self-contained HTML page showing the module graph
    Render(Render),
    /// Compare forbidden imports, cycles and module metrics before and after moving a file
    Simulate(Simulate),
    /// Compare two scan documents: module metrics, cycles and forbidden imports
    Diff(DiffArgs),
    /// Scan recent Git commits and write an architecture timeline
    History(HistoryArgs),
    /// Find a verified commit boundary that introduced a cycle or forbidden edge
    Bisect(provenance_cli::BisectArgs),
    /// Rank changed files by churn, coupling, cycles and coordination cost
    Hotspots(hotspots_cli::HotspotsArgs),
    /// Show affected modules, tests and reviewers for a Git diff
    Impact(impact_cli::ImpactArgs),
    /// Compare exposed symbols with external usage and intended entrypoints
    Surface(surface_cli::SurfaceArgs),
    /// Draft layout rules from observed module dependencies, for review
    InferLayout(infer_layout_cli::InferLayoutArgs),
    /// Rank folders that could become packages, with cuts and cycle evidence
    PackageSplits(package_splits_cli::PackageSplitsArgs),
    /// Select ordered source context within an estimated token budget
    Context(context_cli::ContextArgs),
    /// Print a module reading tour, grouping cycles into a single stop
    Tour(tour_cli::TourArgs),
    /// Analyze declared HTTP, gRPC and queue dependencies between services
    Services(services_cli::ServicesArgs),
    /// List default file exclusions and their categories
    ShowExcludes,
    /// List imports whose file or external package could not be resolved
    Unresolved(Unresolved),
}

#[derive(Args)]
struct Unresolved {
    #[arg(default_value = ".archgraph/archgraph.json")]
    graph: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct HistoryArgs {
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
    /// Start date (default: two years before the selected endpoint)
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
    /// Directory for history.json and history.html [default: <root>/.archgraph]
    #[arg(long)]
    out_dir: Option<PathBuf>,
}

#[derive(Args)]
struct Scan {
    /// Repository root
    root: PathBuf,
    #[command(flatten)]
    opts: ScanOpts,
    /// Write the document here instead of stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct Report {
    /// Repository root
    #[arg(default_value = ".")]
    root: PathBuf,
    #[command(flatten)]
    opts: ScanOpts,
    /// Directory for archgraph.json and archgraph.html [default: <root>/.archgraph]
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Skip bounded Git provenance probes
    #[arg(long)]
    no_provenance: bool,
}

/// What `scan`, `report` and `history` share.
#[derive(Args, Clone, serde::Serialize)]
struct ScanOpts {
    /// Directory segments below the source root that form a module [default: 2; Go/Java: each
    /// package is a module]
    #[arg(long)]
    depth: Option<usize>,
    /// Additional glob to exclude (repeatable)
    #[arg(long)]
    exclude: Vec<String>,
    /// Drop the default generated, vendored and test excludes
    #[arg(long)]
    no_default_excludes: bool,
    /// Keep test files
    #[arg(long)]
    include_tests: bool,
    /// Keep generated and vendored files excluded by default
    #[arg(long)]
    include_generated: bool,
    #[arg(skip)]
    collection_includes: Vec<String>,
    /// Override language detection
    #[arg(long, value_enum)]
    language: Option<Language>,
    /// Read recorded collector JSON instead of running the collector
    #[arg(long)]
    collector_input: Option<PathBuf>,
    /// dependency-cruiser executable to run instead of `npx --no-install depcruise`
    #[arg(long)]
    depcruise: Option<PathBuf>,
    /// Python interpreter with grimp 3.17
    #[arg(long, default_value = "python3")]
    python: String,
    /// Python package to scan (default: the single package under the source root)
    #[arg(long)]
    package: Option<String>,
    /// Additional Python import search root (repeatable)
    #[arg(long)]
    python_path: Vec<PathBuf>,
    /// Go toolchain (1.21 or later) that runs the Go helper
    #[arg(long, default_value = "go")]
    go: String,
    /// Java launcher from a JDK 11 or later
    #[arg(long, default_value = "java")]
    java: String,
    /// rust-analyzer to run instead of the active rustup toolchain's
    #[arg(long)]
    rust_analyzer: Option<PathBuf>,
    /// Cargo features rust-analyzer enables
    #[arg(long, value_enum, default_value_t = rust::Features::All)]
    rust_features: rust::Features,
    /// Run build scripts and proc macros, as rust-analyzer does in an editor. This compiles
    /// and runs code from the scanned repository with your permissions
    #[arg(long)]
    rust_build_scripts: bool,
    /// layout.yaml with modules, layers, public entries and forbid/allow rules
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Read the last N commits (default 300) for files that change together
    #[arg(long, value_name = "N", num_args = 0..=1, require_equals = true, default_missing_value = "300",
          value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    cochange: Option<usize>,
    /// Collect explainable churn factors [default window: 12 months ago]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "12 months ago")]
    hotspots: Option<String>,
    /// Include the distinct-author coordination factor in hotspot scores
    #[arg(long)]
    hotspot_authors: bool,
    /// Ignore commits touching more than this many repository files
    #[arg(long, default_value_t = 30, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    hotspot_max_files: usize,
    /// Collect symbol-level public API evidence (TypeScript, Python and Go)
    #[arg(long)]
    symbols: bool,
}

#[derive(Args)]
struct Check {
    /// Scan document
    graph: PathBuf,
    /// Rules replacing the ones embedded in the document
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Write the recomputed document here
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct Render {
    /// Scan document
    graph: PathBuf,
    /// Rules replacing the ones embedded in the document
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Write the page here instead of stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct DiffArgs {
    /// The earlier scan document
    old: PathBuf,
    /// The later scan document
    new: PathBuf,
    /// Exit 1 when any of these is in the diff: new-forbidden, new-cycles, new-unplanned
    #[arg(long, value_delimiter = ',', value_parser = fail_on)]
    fail_on: Vec<FailOn>,
    /// The diff document, or a markdown summary for a pull request
    #[arg(long, value_enum, default_value_t = Format::Json)]
    format: Format,
}

#[derive(Clone, Copy, ValueEnum)]
enum FailOn {
    #[value(name = "new-forbidden")]
    Forbidden,
    #[value(name = "new-cycles")]
    Cycles,
    #[value(name = "new-unplanned")]
    Unplanned,
}

/// A `--fail-on` item, spaces around it ignored (`new-forbidden, new-cycles`).
fn fail_on(s: &str) -> Result<FailOn, String> {
    FailOn::from_str(s.trim(), false)
        .map_err(|_| "expected new-forbidden, new-cycles or new-unplanned".into())
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Markdown,
}

#[derive(Args)]
struct Simulate {
    /// Scan document
    graph: PathBuf,
    /// The file to move and its destination path; the destination decides module and layer
    #[arg(long = "move", num_args = 2, value_names = ["FILE", "DEST"], required = true, allow_hyphen_values = true)]
    moves: Vec<String>,
    /// Rules replacing the ones embedded in the document
    #[arg(long)]
    rules: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    ALL_KINDS.store(cli.all_kinds, Ordering::Relaxed);
    GRANULARITY.store(
        match cli.granularity {
            None => 0,
            Some(Granularity::Folder) => 1,
            Some(Granularity::File) => 2,
        },
        Ordering::Relaxed,
    );
    LEGACY_V1.store(cli.legacy_v1, Ordering::Relaxed);
    let result = match cli.command {
        Cmd::Scan(args) => scan(args).map(|()| ExitCode::SUCCESS),
        Cmd::Report(args) => report(args),
        Cmd::Check(args) => check(args),
        Cmd::Render(args) => render(args).map(|()| ExitCode::SUCCESS),
        Cmd::Simulate(args) => simulate(args).map(|()| ExitCode::SUCCESS),
        Cmd::Diff(args) => diff(args),
        Cmd::History(args) => history(args).map(|()| ExitCode::SUCCESS),
        Cmd::Bisect(args) => provenance_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Hotspots(args) => hotspots_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Impact(args) => impact_cli::run(args),
        Cmd::Surface(args) => surface_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::InferLayout(args) => infer_layout_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::PackageSplits(args) => package_splits_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Context(args) => context_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Tour(args) => tour_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Services(args) => services_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::ShowExcludes => show_excludes().map(|()| ExitCode::SUCCESS),
        Cmd::Unresolved(args) => unresolved(args).map(|()| ExitCode::SUCCESS),
    };
    result.unwrap_or_else(|e| {
        eprintln!("error: {e:#}");
        ExitCode::from(2)
    })
}

fn scan(args: Scan) -> Result<()> {
    let doc = scan_doc(&args.root, &args.opts, args.opts.rules.as_deref())?;
    write(args.output.as_deref(), &doc)
}

fn history(mut args: HistoryArgs) -> Result<()> {
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
    let program = match language {
        Language::Python => Some(&mut args.opts.python),
        Language::Go => Some(&mut args.opts.go),
        Language::Java => Some(&mut args.opts.java),
        _ => None,
    };
    if let Some(program) = program
        && program.contains(['/', '\\'])
    {
        *program = fs::canonicalize(&*program)?.to_string_lossy().into_owned();
    }
    // Ignored node_modules are absent from snapshots; reuse the installed collector.
    if language == Language::Typescript && args.opts.depcruise.is_none() {
        args.opts.depcruise = fs::canonicalize(&args.root)?
            .ancestors()
            .map(|dir| dir.join("node_modules/.bin/depcruise"))
            .find(|path| path.is_file());
    }
    if language == Language::Typescript {
        args.opts.depcruise = args
            .opts
            .depcruise
            .as_ref()
            .map(fs::canonicalize)
            .transpose()?;
    }
    // Explicit rules stay fixed across the timeline; otherwise each commit supplies its own.
    let rules = args
        .opts
        .rules
        .as_deref()
        .map(fs::canonicalize)
        .transpose()?;
    let scan = |root: &Path| {
        let local = root.join("layout.yaml");
        scan_doc(
            root,
            &args.opts,
            rules
                .as_deref()
                .or_else(|| local.is_file().then_some(local.as_path())),
        )
    };
    if let [old, new] = args.diff.as_slice() {
        let resolve = |rev: &str| -> Result<String> {
            let out = Command::new("git")
                .arg("-C")
                .arg(&args.root)
                .args([
                    "-c",
                    "core.fsmonitor=false",
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                ])
                .arg(format!("{rev}^{{commit}}"))
                .output()?;
            if !out.status.success() {
                bail!(
                    "unknown release ref {rev}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Ok(String::from_utf8(out.stdout)?.trim().to_owned())
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
    let mut key = serde_json::to_vec(&args.opts)?;
    key.push(u8::from(ALL_KINDS.load(Ordering::Relaxed)));
    key.push(GRANULARITY.load(Ordering::Relaxed));
    key.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    key.extend_from_slice(include_bytes!("../Cargo.lock"));
    key.extend_from_slice(&sha2::Sha256::digest(fs::read(std::env::current_exe()?)?));
    if let Some(path) = &rules {
        key.extend_from_slice(&fs::read(path)?);
    }
    // A caller-supplied collector may change in place without its path changing.
    let executable = match language {
        Language::Typescript => args.opts.depcruise.as_ref(),
        Language::Rust => args.opts.rust_analyzer.as_ref(),
        _ => None,
    };
    if let Some(executable) = executable {
        key.extend_from_slice(&fs::read(executable)?);
    }
    let runtime = match language {
        Language::Typescript => ("node", vec!["--version"]),
        Language::Python => (args.opts.python.as_str(), vec!["--version"]),
        Language::Go => (args.opts.go.as_str(), vec!["version"]),
        Language::Java => (args.opts.java.as_str(), vec!["-version"]),
        Language::Rust => ("rustup", vec!["show", "active-toolchain"]),
    };
    {
        let (program, version_args) = runtime;
        if let Ok(output) = Command::new(program)
            .args(version_args)
            .current_dir(std::env::temp_dir())
            .env("RUSTUP_AUTO_INSTALL", "0")
            .output()
        {
            key.extend_from_slice(&output.stdout);
            key.extend_from_slice(&output.stderr);
        }
    }
    // Collector packages can change independently of their interpreter or toolchain label.
    for (query_language, program, version_args, cwd) in [
        (
            Language::Python,
            args.opts.python.clone(),
            vec![
                "-I".to_owned(),
                "-c".to_owned(),
                "import importlib.metadata; print(importlib.metadata.version('grimp'))".to_owned(),
            ],
            std::env::temp_dir(),
        ),
        (
            Language::Typescript,
            args.opts
                .depcruise
                .as_ref()
                .map_or_else(|| "npx".to_owned(), |p| p.to_string_lossy().into_owned()),
            if args.opts.depcruise.is_some() {
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
        if let Ok(output) = Command::new(program)
            .args(version_args)
            .current_dir(cwd)
            .output()
            && output.status.success()
        {
            key.extend_from_slice(&output.stdout);
        }
    }
    let analyzer = (language == Language::Rust)
        .then(|| {
            args.opts.rust_analyzer.clone().or_else(|| {
                let output = Command::new("rustup")
                    .args(["which", "rust-analyzer"])
                    .current_dir(std::env::temp_dir())
                    .env("RUSTUP_AUTO_INSTALL", "0")
                    .output()
                    .ok()?;
                output
                    .status
                    .success()
                    .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
            })
        })
        .flatten();
    if let Some(analyzer) = analyzer
        && let Ok(output) = Command::new(analyzer)
            .arg("--version")
            .current_dir(std::env::temp_dir())
            .output()
        && output.status.success()
    {
        key.extend_from_slice(&output.stdout);
    }
    let mut package_roots = Vec::new();
    if language == Language::Typescript {
        package_roots.push(fs::canonicalize(&args.root)?);
    }
    if let Some(executable) = &args.opts.depcruise {
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
    let identity = format!("{:?}", sha2::Sha256::digest(&key));
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
                    &args.opts,
                    &mut current.graph,
                    Some(previous.commit.clone()),
                )?;
                history::persist_context(&args.root, &history_options, &current.graph)?;
            }
        }
    }
    let html = history::render(&history)?;
    let json = out_dir.join("history.json");
    let page = out_dir.join("history.html");
    for path in [&out_dir, &json, &page] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "{} is a symbolic link; refusing to write through it",
                path.display()
            );
        }
    }
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    write(Some(&json), &history)?;
    fs::write(&page, html).with_context(|| format!("writing {}", page.display()))?;
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

fn unresolved(args: Unresolved) -> Result<()> {
    let doc = recompute(&args.graph, None)?;
    if doc.imports.is_none() {
        bail!("import accounting unavailable in this saved graph; rescan with the live collector");
    }
    if args.json {
        return write(None, &doc.unresolved);
    }
    for import in &doc.unresolved {
        println!(
            "{}:{} {} — {}",
            serde_json::to_string(&import.from)?,
            import.line.unwrap_or(0),
            serde_json::to_string(&import.specifier)?,
            serde_json::to_value(import.unresolved)?
                .as_str()
                .unwrap_or("missing")
        );
    }
    println!("{} unresolved imports", doc.unresolved.len());
    Ok(())
}

fn show_excludes() -> Result<()> {
    for language in [
        Language::Typescript,
        Language::Python,
        Language::Go,
        Language::Java,
        Language::Rust,
    ] {
        println!("{language:?}:");
        for pattern in collect::excludes::defaults(language) {
            println!("  {pattern}");
        }
    }
    println!(
        "Also respects .gitignore and .archgraphignore. Layout include overrides defaults/ignore files; explicit exclude wins."
    );
    Ok(())
}

/// Runs the collector under `root` and analyzes its output with `rules`.
fn scan_doc(root: &Path, args: &ScanOpts, rules: Option<&Path>) -> Result<Document> {
    // Resolved here: a relative program path is not reliably resolved against the child's
    // working directory.
    let depcruise = args
        .depcruise
        .as_deref()
        .map(|p| {
            fs::canonicalize(p)
                .with_context(|| format!("--depcruise {}: no such executable", p.display()))
        })
        .transpose()?;
    let source_dir = collect::source_dir(root);
    let loaded_rules = rules.map(rules::load).transpose()?;
    let layout = loaded_rules.as_ref().map(|r| &r.doc);
    let includes: Vec<_> = layout
        .into_iter()
        .flat_map(|r| r.include.iter().cloned())
        .chain(args.collection_includes.iter().cloned())
        .collect();
    let selection = collect::excludes::Options {
        no_defaults: args.no_default_excludes,
        include_tests: args.include_tests,
        include_generated: args.include_generated,
        cli_excludes: args.exclude.clone(),
        excludes: layout.map(|r| r.exclude.clone()).unwrap_or_default(),
        includes: includes.clone(),
    };
    let language = match args.language {
        Some(language) => language,
        None => collect::detect_language_with_options(root, source_dir, selection.clone())?,
    };
    let policy = collect::excludes::Policy::new(root, language, selection)?;
    let excludes = policy.patterns().to_vec();
    // Discovery must retain candidates that layout includes can restore after collection.
    let discovery_excludes = args
        .exclude
        .iter()
        .cloned()
        .chain(layout.into_iter().flat_map(|r| r.exclude.iter().cloned()))
        .collect::<Vec<_>>();

    let input = args
        .collector_input
        .as_ref()
        .map(|p| fs::read(p).with_context(|| format!("reading {}", p.display())))
        .transpose()?;
    let excludes_set = collect::globset(&discovery_excludes)?;
    // Go: files that cannot be split across packages, in pairs.
    let mut bound = Vec::new();
    let mut generated = Vec::new();
    let mut test_support = Vec::new();
    let mut python_package = None;
    let (raw, source_root) = match language {
        Language::Typescript => {
            let (json, type_marking) = match input {
                Some(json) => (json, None),
                None => depcruise::run(
                    root,
                    if args.include_tests
                        || args.include_generated
                        || args.no_default_excludes
                        || !includes.is_empty()
                    {
                        "."
                    } else {
                        source_dir
                    },
                    depcruise.as_deref(),
                )?,
            };
            let (json, imports) = collect::typescript_resolution::enrich(root, &json)?;
            let mut raw = depcruise::parse_at(&json, root)?;
            raw.imports = imports;
            for warning in collect::barrels::typescript(root, &json, &mut raw)? {
                eprintln!("warning: {warning}");
            }
            raw.collector.type_marking = type_marking.map(str::to_owned);
            (raw, source_dir.to_owned())
        }
        Language::Python => {
            let package = grimp::package(root, source_dir, args.package.as_deref(), &excludes_set)?;
            python_package = Some(package.clone());
            let json = match input {
                Some(json) => json,
                None => grimp::run(
                    root,
                    source_dir,
                    &package,
                    &args.python,
                    args.include_tests || args.no_default_excludes || !includes.is_empty(),
                    &args.python_path,
                )?,
            };
            (
                grimp::parse(&json, root, source_dir)?,
                collect::join(source_dir, &package),
            )
        }
        // Package paths are what the go command reports, from the scan root, `src/` or not.
        Language::Go => {
            let json = match input {
                Some(json) => json,
                None => {
                    let (dirs, warnings) = go::module_dirs(root, &excludes_set)?;
                    for w in warnings {
                        eprintln!("warning: {w}");
                    }
                    if dirs.is_empty() {
                        bail!("no go.mod with a module line under {}", root.display());
                    }
                    go::run(root, &args.go, &dirs)?
                }
            };
            if !args.include_tests && !args.no_default_excludes && includes.is_empty() {
                let omitted = go::parse(&json, true, false)?.left_out;
                if !omitted.is_empty() {
                    eprintln!(
                        "warning: left out as test support (--include-tests keeps them): {}",
                        omitted.join(", ")
                    );
                }
            }
            let parsed = go::parse(&json, true, true)?;
            generated = parsed.generated;
            test_support = parsed.test_support;
            bound = parsed.bound;
            (parsed.raw, ".".to_owned())
        }
        Language::Rust => {
            let json = match input {
                Some(json) => json,
                None => rust::run(
                    root,
                    &rust::Options {
                        rust_analyzer: args.rust_analyzer.as_deref(),
                        features: args.rust_features,
                        build_scripts: args.rust_build_scripts,
                    },
                )?,
            };
            (rust::parse(&json)?, ".".to_owned())
        }
        Language::Java => {
            let json = match input {
                Some(json) => json,
                None => java::run(root, &args.java, &excludes_set)?,
            };
            (java::parse(&json)?, ".".to_owned())
        }
    };
    let mut raw = raw;
    let kept: std::collections::HashSet<_> = policy
        .select(&raw.files)?
        .into_iter()
        .filter(|path| {
            (args.no_default_excludes
                || args.include_generated
                || !generated.contains(path)
                || policy.included(path))
                && (args.no_default_excludes
                    || args.include_tests
                    || !test_support.contains(path)
                    || policy.included(path))
        })
        .collect();
    let excluded_files = raw
        .files
        .iter()
        .filter(|p| !kept.contains(*p))
        .cloned()
        .collect();
    raw.files.retain(|p| kept.contains(p));
    raw.edges
        .retain(|e| kept.contains(&e.from) && kept.contains(&e.to));
    if let Some(imports) = &mut raw.imports {
        imports.retain(|i| kept.contains(&i.from));
    }
    raw.collector.bound_files = go::bound_files(&bound, &raw.files);
    if let Some(n) = raw.collector.unresolved.filter(|&n| n > 0) {
        if raw.imports.is_some() {
            eprintln!(
                "warning: {n} unresolved collector reference(s) reported by {}; explicit import accounting is reported separately",
                raw.collector.name
            );
        } else {
            eprintln!(
                "warning: {n} unresolved import(s) reported by {}; their edges are missing",
                raw.collector.name
            );
        }
    }

    let source = Source {
        language,
        collector: raw.collector,
        git: git(root),
        generated_at: now_utc(),
        scan_root: git_scan_root(root),
    };
    // Keep package directories intact unless a depth is explicitly requested.
    let package_modules = matches!(language, Language::Go | Language::Java) && args.depth.is_none();
    let config = Config {
        depth: args.depth.unwrap_or(if package_modules { 0 } else { 2 }),
        granularity: if GRANULARITY.load(Ordering::Relaxed) == 2 {
            Granularity::File
        } else {
            Granularity::Folder
        },
        package_modules,
        source_root,
        excludes,
        include_tests: args.include_tests
            || args.no_default_excludes
            || raw.files.iter().any(|p| collect::excludes::is_test(p)),
        include_generated: args.include_generated || args.no_default_excludes,
        includes,
        excluded_files,
        explicit_excludes: Some(args.exclude.clone()),
        all_kinds: ALL_KINDS.load(Ordering::Relaxed),
        no_default_excludes: args.no_default_excludes,
        rules: loaded_rules,
        codeowners: codeowners::load(root)?,
        collection: matches!(language, Language::Python | Language::Rust).then_some(
            archgraph::model::CollectionOptions {
                python_package,
                python_paths: args
                    .python_path
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                rust_features: (language == Language::Rust).then_some(args.rust_features),
                rust_build_scripts: language == Language::Rust && args.rust_build_scripts,
            },
        ),
        file_metadata: raw.metadata,
        legacy_module_classes: false,
        import_records: raw.imports,
    };
    if let Some(owners) = &config.codeowners {
        for warning in &owners.warnings {
            eprintln!(
                "warning: {}:{}: {}",
                owners.path, warning.line, warning.message
            );
        }
    }
    let cochange = args
        .cochange
        .map(|n| cochange::collect(root, n, &raw.files))
        .transpose()?;
    let entries = match language {
        Language::Typescript => depcruise::package_entries(root, &raw.files),
        Language::Python | Language::Go | Language::Rust | Language::Java => Vec::new(),
    };
    let mut doc = archgraph::analyze(source, config, raw.files, raw.edges, entries, cochange)?;
    if args.symbols {
        let symbols = archgraph::symbols::collect(
            root,
            &doc,
            archgraph::symbols::Options {
                depcruise: depcruise.as_deref(),
                python: &args.python,
                go: &args.go,
            },
        )?;
        archgraph::surface::attach(&mut doc, symbols, archgraph::surface::DEFAULT_MAX_SYMBOLS)?;
    }
    if let Some(since) = &args.hotspots {
        let report = archgraph::hotspots::collect(
            root,
            &doc,
            &archgraph::hotspots::Options {
                since: since.clone(),
                max_files: args.hotspot_max_files,
                authors: args.hotspot_authors,
            },
        )?;
        archgraph::apply_hotspots(&mut doc, report);
    }
    Ok(doc)
}

fn report(args: Report) -> Result<ExitCode> {
    let root = args.root.as_path();
    let rules = args.opts.rules.clone().or_else(|| {
        let found = root.join("layout.yaml");
        found.is_file().then_some(found)
    });
    let mut doc = scan_doc(root, &args.opts, rules.as_deref())?;
    if !args.no_provenance && args.opts.collector_input.is_none() && doc.source.git.is_some() {
        provenance_cli::enrich(root, &args.opts, &mut doc)?;
    }
    let out_dir = args.out_dir.unwrap_or_else(|| root.join(".archgraph"));
    let (json, html) = (
        out_dir.join("archgraph.json"),
        out_dir.join("archgraph.html"),
    );
    // The default folder lives in the scanned repository, which may ship it (or the files in it)
    // as links that would make this write land outside it.
    // ponytail: checked, not opened with O_NOFOLLOW; a link swapped in during the scan still wins.
    for path in [&out_dir, &json, &html] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "{} is a symbolic link; refusing to write through it (pass --out-dir)",
                path.display()
            );
        }
    }
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    write(Some(&json), &doc)?;
    fs::write(&html, render::render(&doc)?)
        .with_context(|| format!("writing {}", html.display()))?;

    let bold = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let heading = |text: &str| {
        if bold {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    let mut out = format!(
        "{}\n",
        heading(&format!("archgraph report: {}", root.display()))
    );
    match &rules {
        Some(path) => out += &format!("rules: {}\n", path.display()),
        None => out += "rules: none (pass --rules, or add layout.yaml to the root)\n",
    }
    if doc.source.collector.type_marking.as_deref() == Some("unavailable") {
        out += "note: type-only imports could not be told apart; some cycles may include imports TypeScript removes\n";
    }
    out += &format!(
        "\n{}\n  {}, {}, {}\n",
        heading("Graph"),
        count(doc.files.len(), "file"),
        count(doc.modules.len(), "module"),
        count(doc.edges.len(), "import")
    );
    if let Some(resolution) = &doc.import_resolution {
        let percent = resolution
            .resolved_percent
            .map(|p| format!("{p:.1}%"))
            .unwrap_or_else(|| "n/a".into());
        out += &format!(
            "  {} imports, {} resolved ({percent}), {} unresolved — run archgraph unresolved for the list\n",
            resolution.imports, resolution.resolved, resolution.unresolved
        );
    } else {
        out += "  import resolution accounting unavailable in this recording; rescan with the live collector\n";
    }
    if !doc.external_nodes.is_empty() {
        out += &format!(
            "  {} known external packages (outside internal metrics)\n",
            doc.external_nodes.len()
        );
    }
    if let Some(coverage) = &doc.test_coverage {
        out += &format!(
            "  tests import {} of {} production modules; {} without importing tests\n",
            coverage.covered,
            coverage.modules,
            coverage.without_importing_tests.len()
        );
        for module in &coverage.without_importing_tests {
            out += &format!("    {module}\n");
        }
    }
    out += &format!("  {} excluded files\n", doc.config.excluded_files.len());
    out += &format!("\n{}\n", heading("External exposure"));
    if let Some(exposure) = &doc.external_exposure {
        out += &format!(
            "  advisory threshold >{}% of import occurrences; known packages include standard libraries\n",
            exposure.threshold_percent
        );
        for row in &exposure.modules {
            let percent = row
                .external_percent
                .map(|p| format!("{p:.1}%"))
                .unwrap_or_else(|| "n/a".into());
            let weighted_percent = row
                .weighted_external_percent
                .map(|p| format!("{p:.1}%"))
                .unwrap_or_else(|| "n/a".into());
            out += &format!(
                "  {}: {} packages, {}/{} imports ({percent}), weight {:.4}/{:.4} ({weighted_percent}){}\n",
                row.module,
                row.external_packages,
                row.external_imports,
                row.imports,
                row.external_weight,
                row.total_weight,
                if row.exceeds_threshold {
                    " — high exposure; review wrapper or boundary"
                } else {
                    ""
                }
            );
            if row.unresolved_imports > 0 {
                out += &format!(
                    "    {} unresolved; known external share is a lower bound\n",
                    row.unresolved_imports
                );
            }
        }
    } else {
        out += "  unavailable in this recording; rescan with import accounting\n";
    }
    if let Some(owners) = &doc.config.codeowners {
        let owned = doc.files.iter().filter(|f| !f.owners.is_empty()).count();
        let mut counts = BTreeMap::new();
        for file in &doc.files {
            for owner in &file.owners {
                *counts.entry(owner.as_str()).or_insert(0usize) += 1;
            }
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        out += &format!(
            "\n{} ({})\n  {owned} of {} files owned; shared files count for each owner\n",
            heading("Ownership"),
            owners.path,
            doc.files.len()
        );
        for (owner, n) in counts.iter().take(10) {
            out += &format!("  {owner}: {}\n", count(*n, "file"));
        }
    }
    if rules.is_some() {
        out += &format!("\n{}\n", heading("Rules"));
        for (verdict, label) in [
            (Verdict::Forbidden, "forbidden"),
            (Verdict::Unplanned, "unplanned"),
        ] {
            let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
            for e in doc
                .edges
                .iter()
                .chain(&doc.external_edges)
                .filter(|e| e.verdict == Some(verdict))
            {
                *by_rule
                    .entry(e.rule.as_deref().unwrap_or_default())
                    .or_default() += 1;
            }
            let total: usize = by_rule.values().sum();
            let mut by_rule: Vec<_> = by_rule.into_iter().collect();
            by_rule.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let rules: Vec<String> = by_rule.iter().map(|(r, n)| format!("{r} {n}")).collect();
            out += &match total {
                0 => format!("  0 {label}\n"),
                n => format!("  {n} {label}: {}\n", rules.join(", ")),
            };
        }
    }
    out += &format!("\n{}\n", heading("Cycles"));
    if !doc.type_only_cycles.is_empty() {
        out += &format!(
            "  {} type-only/deferred cycles (type, lazy or structural/mixed; excluded from runtime findings)\n",
            doc.type_only_cycles.len()
        );
    }
    for grabbag in &doc.grabbags {
        out += &format!(
            "  cycle via grab-bag {}: {} ({} genuine residual cycles)\n",
            grabbag
                .candidates
                .iter()
                .map(|c| c.module.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            grabbag.members.join(", "),
            grabbag.remaining_cycles.len()
        );
        for candidate in &grabbag.candidates {
            out += &format!(
                "    {}: cohesion {:.4}, fan-in {:.4}, internal edges/file {:.4}; split folder; involved files: {}\n",
                candidate.module,
                candidate.cohesion,
                candidate.fan_in,
                candidate.internal_edges_per_file,
                candidate.files.join(", ")
            );
        }
    }
    let module_cycles: Vec<_> = doc
        .cycles
        .iter()
        .filter(|c| c.level == CycleLevel::Module)
        .collect();
    match module_cycles.iter().map(|c| c.members.len()).max() {
        None => out += "  no module cycles\n",
        Some(largest) => {
            out += &format!(
                "  {}, largest {}\n",
                count(module_cycles.len(), "module cycle"),
                count(largest, "module")
            );
            // A registrar move leads when it beats the cycle's single-import cut and the cycle
            // shrinks; the others follow the cut.
            let single_cut = |c: &&archgraph::model::Cycle| {
                c.two_way.iter().flatten().any(|t| {
                    t.imports == 1 && !t.registry && !t.entry && !t.even && !t.resolved_by_move
                })
            };
            let mut trailing = String::new();
            for c in &module_cycles {
                for r in &c.registrar_moves {
                    let (Some(to), Some(removed)) = (&r.to, r.pairs_removed) else {
                        continue;
                    };
                    if removed <= 0 {
                        continue;
                    }
                    let out = if r.leads(single_cut(c)) {
                        &mut out
                    } else {
                        &mut trailing
                    };
                    let cycle = match (r.cycle_before, r.cycle_after) {
                        (Some(b), Some(0)) => format!("the {b}-module cycle goes"),
                        (Some(b), Some(a)) if a < b => {
                            format!("the cycle drops from {b} to {a} modules")
                        }
                        (Some(b), Some(a)) if a == b => format!("the cycle stays at {b} modules"),
                        (Some(b), Some(a)) => format!("the cycle goes from {b} to {a} modules"),
                        _ => String::new(),
                    };
                    *out += &format!(
                        "  move {} to {to}: removes {}; {cycle}; no new cycle\n",
                        r.file,
                        count(removed as usize, "two-way pair")
                    );
                }
            }
            let lightest = module_cycles
                .iter()
                .flat_map(|c| c.two_way.iter().flatten())
                .filter(|t| !t.registry && !t.entry && !t.even && !t.resolved_by_move)
                .min_by_key(|t| (t.imports, t.reverse_imports));
            let moved = module_cycles.iter().any(|c| {
                c.registrar_moves
                    .iter()
                    .any(|r| r.pairs_removed.is_some_and(|n| n > 0))
            });
            let pairs = module_cycles
                .iter()
                .any(|c| c.two_way.iter().flatten().next().is_some());
            out += &match lightest {
                Some(t) => format!(
                    "  lightest two-way cut: {} -> {}, {} ({} the other way)\n",
                    t.from,
                    t.to,
                    count(t.imports, "import"),
                    t.reverse_imports
                ),
                // The move is the step: no other cut remains.
                None if moved => String::new(),
                None if pairs => {
                    "  no lighter side to cut: the two-way pairs are even or wiring; see the cycles' break sets\n".into()
                }
                None => "  no two modules import each other; see the cycles' break sets\n".into(),
            };
            out += &trailing;
        }
    }
    out += &format!("\n{}\n", heading("Findings"));
    let mut by_kind: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for f in &doc.findings {
        let kind = serde_json::to_value(f.kind)?
            .as_str()
            .unwrap_or_default()
            .to_owned();
        by_kind.entry(kind).or_default().push(&f.subject);
    }
    let mut by_kind: Vec<_> = by_kind.into_iter().collect();
    by_kind.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    if by_kind.is_empty() {
        out += "  none\n";
    }
    for (kind, subjects) in &by_kind {
        out += &format!("  {kind} ({})\n", subjects.len());
        for subject in subjects.iter().take(3) {
            // Whole-tree findings have subject `.`: name what they are about.
            let subject = match (kind.as_str(), *subject) {
                ("seam", ".") => "clusters across folders",
                ("small-repo", ".") => "the whole source tree",
                (_, s) => s,
            };
            out += &format!("    {subject}\n");
        }
        if subjects.len() > 3 {
            out += &format!("    and {} more\n", subjects.len() - 3);
        }
    }
    out += &format!(
        "\n{}\n  {}\n  {}\n",
        heading("Wrote"),
        json.display(),
        html.display()
    );
    print!("{out}");
    Ok(if doc.summary.forbidden > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `1 file`, `2 files`.
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

fn policy_target(doc: &archgraph::model::Document, path: &str) -> String {
    doc.external_nodes
        .iter()
        .find(|node| node.id == path)
        .map(|node| format!("pkg:{}", node.package))
        .unwrap_or_else(|| path.to_owned())
}

fn check(args: Check) -> Result<ExitCode> {
    let doc = recompute(&args.graph, args.rules.as_deref())?;
    if doc.config.rules.is_none() && doc.surface.is_none() {
        bail!("no rules: pass --rules, or scan with --rules");
    }
    let (mut forbidden, mut unplanned) = (0, 0);
    for e in doc.edges.iter().chain(&doc.external_edges) {
        let label = match e.verdict {
            Some(Verdict::Forbidden) => {
                forbidden += 1;
                "forbidden"
            }
            Some(Verdict::Unplanned) => {
                unplanned += 1;
                "unplanned"
            }
            _ => continue,
        };
        let via = match &e.reach {
            Some(path) => format!(
                " via {}",
                path.iter()
                    .map(|p| policy_target(&doc, p))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            None => String::new(),
        };
        let rule = e.rule.as_deref().unwrap_or_default();
        let written = e.original_to.as_deref().unwrap_or(&e.to);
        let resolved = if written != e.to {
            format!(" [resolves to {}]", policy_target(&doc, &e.to))
        } else {
            String::new()
        };
        println!(
            "{label} {} -> {} ({rule}){via}{resolved}",
            e.from,
            policy_target(&doc, written)
        );
    }
    println!("{forbidden} forbidden, {unplanned} unplanned");
    if let Some(surface) = &doc.surface {
        surface_cli::print_findings(surface);
    }
    if let Some(path) = &args.output {
        write(Some(path), &doc)?;
    }
    Ok(if forbidden > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn simulate(args: Simulate) -> Result<()> {
    let [file, to] = args.moves.as_slice() else {
        bail!("--move takes one file and one destination");
    };
    let input = read_scan(&args.graph)?;
    let mut config = input.config;
    if let Some(path) = &args.rules {
        config.rules = Some(rules::load(path)?);
    }
    archgraph::resolution::require_policy_accounting(&config)?;
    let rules = rules::compile(&config)?;
    let owners = config
        .codeowners
        .as_ref()
        .map(codeowners::compile)
        .transpose()?;
    let files = input.files.into_iter().map(|f| f.path).collect();
    let doc = archgraph::build(
        input.source,
        config,
        files,
        input.edges,
        input.package_entries,
        rules.as_ref(),
        owners.as_ref(),
    );
    archgraph::exposure::validate(&doc)?;
    write(None, &simulate::simulate(&doc, rules.as_ref(), file, to)?)
}

fn render(args: Render) -> Result<()> {
    let html = render::render(&recompute(&args.graph, args.rules.as_deref())?)?;
    match &args.output {
        Some(path) => fs::write(path, html).with_context(|| format!("writing {}", path.display())),
        None => {
            print!("{html}");
            Ok(())
        }
    }
}

fn diff(args: DiffArgs) -> Result<ExitCode> {
    let (old, new) = (recompute(&args.old, None)?, recompute(&args.new, None)?);
    let d = diff::diff(&old, &new);
    let failed: Vec<&str> = args
        .fail_on
        .iter()
        .filter_map(|f| match f {
            FailOn::Forbidden => (!d.new_forbidden.is_empty()).then_some("new-forbidden"),
            FailOn::Cycles => (!d.new_cycles.is_empty()).then_some("new-cycles"),
            FailOn::Unplanned => (!d.new_unplanned.is_empty()).then_some("new-unplanned"),
        })
        .collect();
    match args.format {
        Format::Json => write(None, &d)?,
        Format::Markdown => print!("{}", diff::markdown(&d, &failed)),
    }
    Ok(if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// A scan document recomputed from its files, edges and config, with `rules` replacing the
/// embedded ones.
fn recompute(path: &Path, rules: Option<&Path>) -> Result<Document> {
    let mut input = read_scan(path)?;
    if let Some(path) = rules {
        input.config.rules = Some(rules::load(path)?);
        let compiled = rules::compile(&input.config)?;
        for file in &mut input.files {
            file.module = modules::assigned(
                &file.path,
                &input.config,
                input.source.language,
                compiled.as_ref(),
            );
            if file.class == Some(archgraph::model::ModuleClass::Tests) {
                file.module = format!("tests::{}", file.module);
            }
        }
        // Scores and introductions were judged under the old module/rule configuration.
        input.findings.clear();
        input.hotspots = None;
    }
    archgraph::restore_scan(input)
}

fn read_scan(path: &Path) -> Result<ScanInput> {
    let text = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut input: ScanInput = serde_json::from_slice(&text)
        .with_context(|| format!("reading scan document {}", path.display()))?;
    if !archgraph::model::supported_schema(&input.schema) || input.kind != "scan" {
        bail!(
            "{} is `{}` `{}`, expected a `{SCHEMA}` scan document",
            path.display(),
            input.schema,
            input.kind
        );
    }
    input.config.all_kinds |= ALL_KINDS.load(Ordering::Relaxed);
    input.config.import_records = input.imports.clone();
    input.config.file_metadata = input
        .files
        .iter()
        .map(|f| {
            (
                f.path.clone(),
                archgraph::model::NodeMetadata {
                    barrel: f.barrel,
                    test: f.class == Some(archgraph::model::ModuleClass::Tests),
                },
            )
        })
        .collect();
    archgraph::validate_scan(&input)
        .with_context(|| format!("inconsistent scan document {}", path.display()))?;
    if GRANULARITY.load(Ordering::Relaxed) != 0 {
        input.config.granularity = if GRANULARITY.load(Ordering::Relaxed) == 2 {
            Granularity::File
        } else {
            Granularity::Folder
        };
        let compiled = rules::compile(&input.config)?;
        for file in &mut input.files {
            file.module = modules::assigned(
                &file.path,
                &input.config,
                input.source.language,
                compiled.as_ref(),
            );
            if file.class == Some(archgraph::model::ModuleClass::Tests) {
                file.module = format!("tests::{}", file.module);
            }
        }
        input.findings.clear();
        input.hotspots = None;
    }

    Ok(input)
}

/// Pretty JSON to `output`, or stdout.
fn write(output: Option<&Path>, value: &impl serde::Serialize) -> Result<()> {
    let mut value = serde_json::to_value(value)?;
    if LEGACY_V1.load(Ordering::Relaxed) {
        archgraph::compat::v1(&mut value)?;
    }
    let json = serde_json::to_string_pretty(&value)? + "\n";
    match output {
        Some(path) => fs::write(path, json).with_context(|| format!("writing {}", path.display())),
        None => {
            print!("{json}");
            Ok(())
        }
    }
}

fn git_scan_root(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.fsmonitor=false", "rev-parse", "--show-prefix"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8(output.stdout).ok()?;
    let prefix = prefix.trim_end_matches('\n').trim_end_matches('/');
    Some(if prefix.is_empty() { "." } else { prefix }.to_owned())
}

fn git(root: &Path) -> Option<Git> {
    let run = |args: &[&str]| {
        // A repository's core.fsmonitor names a program `git status` would run.
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    Some(Git {
        commit: run(&["rev-parse", "HEAD"])?,
        dirty: !run(&["status", "--porcelain"])?.is_empty(),
    })
}

/// RFC 3339 UTC, whole seconds; the civil date is Howard Hinnant's civil_from_days.
fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
