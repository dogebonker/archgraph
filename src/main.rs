macro_rules! print { ($($arg:tt)*) => { archgraph::output::stdout(format_args!($($arg)*)) }; }
macro_rules! println { () => { archgraph::output::stdout(format_args!("\n")) }; ($($arg:tt)*) => { archgraph::output::stdout(format_args!("{}\n", format_args!($($arg)*))) }; }
macro_rules! eprintln { ($($arg:tt)*) => { archgraph::output::stderr(format_args!("{}\n", format_args!($($arg)*))) }; }
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

/// Options belong to the calling thread, including daemon requests.
#[derive(Clone, Default)]
struct Globals {
    no_daemon: bool,
    all_kinds: bool,
    legacy_v1: bool,
    granularity: Option<Granularity>,
    brief: bool,
    tty: bool,
    tool_timeout: u64,
    rank: archgraph::recipes::Rank,
}
thread_local! { static GLOBALS: RefCell<Globals> = RefCell::new(Globals::default()); }
fn globals() -> Globals {
    GLOBALS.with(|g| g.borrow().clone())
}

use anyhow::{Context, Result, bail};
use archgraph::collect::{self, rust};
use archgraph::model::{Document, Git, Granularity, Language, SCHEMA, ScanInput, Source, Verdict};
use archgraph::{codeowners, diff, history, modules, render, rules, simulate};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[path = "cli/brief.rs"]
mod brief_cli;
#[path = "cli/check.rs"]
mod check_cli;
#[path = "cli/context.rs"]
mod context_cli;
#[path = "cli/daemon.rs"]
mod daemon_cli;
#[path = "cli/explain.rs"]
mod explain_cli;
#[path = "cli/history.rs"]
mod history_cli;
#[path = "cli/hooks.rs"]
mod hooks_cli;
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
#[path = "cli/mcp.rs"]
mod mcp_cli;
#[path = "cli/package_splits.rs"]
mod package_splits_cli;
#[path = "cli/provenance.rs"]
mod provenance_cli;
#[path = "cli/recipes.rs"]
mod recipes_cli;
#[path = "cli/report.rs"]
mod report_cli;
#[path = "cli/scan.rs"]
mod scan_cli;
#[path = "cli/services.rs"]
mod services_cli;
#[path = "cli/surface.rs"]
mod surface_cli;
#[path = "cli/tour.rs"]
mod tour_cli;

#[derive(Parser)]
#[command(version, about = "Import-graph evidence for architecture decisions")]
struct Cli {
    /// Bounded deterministic JSON for agent context windows
    #[arg(long, global = true)]
    brief: bool,
    /// Bypass a running daemon and collect independently
    #[arg(long, global = true)]
    no_daemon: bool,
    /// Finding ordering: hotspot × recipe confidence × pairs removed, or legacy
    #[arg(long, global = true, value_enum, default_value_t = archgraph::recipes::Rank::Recipe)]
    rank: archgraph::recipes::Rank,
    /// Include every import kind in cycle detection, for comparison
    #[arg(long, global = true)]
    all_kinds: bool,
    /// Export JSON using legacy v1 kind names and schema discriminator
    #[arg(long, global = true)]
    legacy_v1: bool,
    /// Graph module granularity; file overrides folder/layout grouping
    #[arg(long, global = true, value_enum)]
    granularity: Option<Granularity>,
    /// Seconds an external tool (collector, rust-analyzer, gh) may run before it is killed; 0
    /// waits forever
    #[arg(long, global = true, value_name = "SECONDS", default_value_t = 600)]
    tool_timeout: u64,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install and run architecture checks around agent edits
    Hook(hooks_cli::Hook),
    /// Inspect architecture rule-fire counts
    Fires(hooks_cli::Fires),
    /// Return a typed fix recipe and its verification predicates
    Propose(recipes_cli::Target),
    /// Preview or execute the mechanical steps of a finding's recipe
    Apply(recipes_cli::Apply),
    /// Rescan and verify a previously proposed or applied finding
    Verify(recipes_cli::Verify),
    /// Expose architecture tools to MCP clients over stdio
    Mcp(mcp_cli::Mcp),
    #[command(hide = true)]
    McpCall(mcp_cli::Call),
    /// Keep a repository graph warm over local JSON-RPC
    Serve(daemon_cli::Serve),
    /// Show the local daemon state
    Status(daemon_cli::Status),
    /// Collect the import graph and write a scan document
    Scan(scan_cli::Scan),
    /// Scan, apply rules, write archgraph.json and archgraph.html, and print a summary
    Report(report_cli::Report),
    /// Recompute a scan document under rules and list forbidden and unplanned imports
    Check(check_cli::Check),
    /// Write one self-contained HTML page showing the module graph
    Render(Render),
    /// Compare forbidden imports, cycles and module metrics before and after moving a file
    Simulate(Simulate),
    /// Compare two scan documents: module metrics, cycles and forbidden imports
    Diff(DiffArgs),
    /// Scan recent Git commits and write an architecture timeline
    History(history_cli::HistoryArgs),
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
    /// Explain a file, module or finding from a saved graph
    Explain(Explain),
    /// List imports whose file or external package could not be resolved
    Unresolved(Unresolved),
}

#[derive(Args)]
struct Explain {
    subject: String,
    #[arg(long, default_value = ".archgraph/archgraph.json")]
    graph: PathBuf,
    /// Compact edit context, bounded to 400 UTF-8 bytes
    #[arg(long)]
    for_agent: bool,
}

#[derive(Args)]
struct Unresolved {
    #[arg(default_value = ".archgraph/archgraph.json")]
    graph: PathBuf,
    #[arg(long)]
    json: bool,
}

/// What `scan`, `report`, `history` and `impact` collect: these change the graph itself.
#[derive(Args, Clone, serde::Serialize)]
struct ScanOpts {
    /// Collector engine; native uses tree-sitter and source-level resolution
    #[arg(long, value_enum, default_value_t = collect::Engine::Reference)]
    engine: collect::Engine,
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
    /// Python package to scan (default: the single package under the source root)
    #[arg(long)]
    package: Option<String>,
    /// Additional Python import search root (repeatable)
    #[arg(long)]
    python_path: Vec<PathBuf>,
    /// Cargo features rust-analyzer enables
    #[arg(long, value_enum, default_value_t = rust::Features::All)]
    rust_features: rust::Features,
    /// layout.yaml with modules, layers, public entries and forbid/allow rules
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Read the last N commits (default 300) for files that change together
    #[arg(long, value_name = "N", num_args = 0..=1, require_equals = true, default_missing_value = "300",
          value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    cochange: Option<usize>,
    /// Collect symbol-level public API evidence (TypeScript, Python and Go)
    #[arg(long)]
    symbols: bool,
}

/// Churn evidence read from commit history after collection.
#[derive(Args, Clone, Default, serde::Serialize)]
struct HotspotOpts {
    /// Collect explainable churn factors [default window: 12 months ago]
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "12 months ago")]
    hotspots: Option<String>,
    /// Include the distinct-author coordination factor in hotspot scores
    #[arg(long)]
    hotspot_authors: bool,
    /// Ignore commits touching more than this many repository files
    #[arg(long, default_value_t = 30, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    hotspot_max_files: usize,
}

/// Collector executables and their execution mode: how to collect, not what.
#[derive(Args, Clone, serde::Serialize)]
struct CollectorOpts {
    /// dependency-cruiser executable to run instead of `npx --no-install depcruise`
    #[arg(long)]
    depcruise: Option<PathBuf>,
    /// Python interpreter with grimp 3.17
    #[arg(long, default_value = "python3")]
    python: String,
    /// Go toolchain (1.21 or later) that runs the Go helper
    #[arg(long, default_value = "go")]
    go: String,
    /// Java launcher from a JDK 11 or later
    #[arg(long, default_value = "java")]
    java: String,
    /// Elixir 1.14 or later, used only to parse source
    #[arg(long, default_value = "elixir")]
    elixir: String,
    /// rust-analyzer to run instead of the active rustup toolchain's
    #[arg(long)]
    rust_analyzer: Option<PathBuf>,
    /// Run build scripts and proc macros, as rust-analyzer does in an editor. This compiles
    /// and runs code from the scanned repository with your permissions
    #[arg(long)]
    rust_build_scripts: bool,
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
    GLOBALS.with(|g| {
        *g.borrow_mut() = Globals {
            no_daemon: cli.no_daemon,
            all_kinds: cli.all_kinds,
            legacy_v1: cli.legacy_v1,
            granularity: cli.granularity,
            brief: cli.brief,
            tty: false,
            tool_timeout: cli.tool_timeout,
            rank: cli.rank,
        }
    });
    collect::request_timeout(
        (cli.tool_timeout > 0).then(|| std::time::Duration::from_secs(cli.tool_timeout)),
    );
    if let Cmd::Serve(args) = cli.command {
        return daemon_cli::serve(args).unwrap_or_else(|e| {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        });
    }
    if let Cmd::Mcp(args) = cli.command {
        return mcp_cli::serve(args)
            .map(|()| ExitCode::SUCCESS)
            .unwrap_or_else(|e| {
                eprintln!("error: {e:#}");
                ExitCode::from(2)
            });
    }
    match daemon_cli::forward(&cli) {
        Ok(Some(out)) => {
            print!("{}", out.stdout);
            if !out.stderr.is_empty() {
                archgraph::output::stderr(format_args!("{}", out.stderr));
            }
            return ExitCode::from(out.code);
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("error: {error:#}");
            return ExitCode::from(2);
        }
    }
    execute(cli)
}
fn execute(cli: Cli) -> ExitCode {
    let brief = cli.brief
        && !matches!(&cli.command, Cmd::Hook(_) | Cmd::Fires(_))
        && !matches!(&cli.command, Cmd::Explain(args) if args.for_agent);
    let capture = brief.then(archgraph::output::Capture::begin);
    let code = execute_inner(cli);
    if let Some(capture) = capture {
        let out = capture.finish(if code == ExitCode::SUCCESS {
            0
        } else if code == ExitCode::from(1) {
            1
        } else {
            2
        });
        if !out.stdout.is_empty() {
            let value = serde_json::from_str::<serde_json::Value>(&out.stdout)
                .unwrap_or_else(|_| serde_json::json!({"kind":"message","text":out.stdout}));
            let value = if value["schema"] == "archgraph/brief/v1" {
                value
            } else {
                brief_cli::project(&value)
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("serializing brief output")
            );
        }
        if !out.stderr.is_empty() {
            archgraph::output::stderr(format_args!("{}", out.stderr));
        }
    }
    code
}
fn execute_inner(cli: Cli) -> ExitCode {
    use std::io::IsTerminal;
    GLOBALS.with(|g| {
        *g.borrow_mut() = Globals {
            no_daemon: cli.no_daemon,
            all_kinds: cli.all_kinds,
            legacy_v1: cli.legacy_v1,
            granularity: cli.granularity,
            brief: cli.brief,
            tty: daemon_cli::caller_tty().unwrap_or_else(|| std::io::stdout().is_terminal()),
            tool_timeout: cli.tool_timeout,
            rank: cli.rank,
        }
    });
    collect::request_timeout(
        (cli.tool_timeout > 0).then(|| std::time::Duration::from_secs(cli.tool_timeout)),
    );
    let result = match cli.command {
        Cmd::Hook(args) => hooks_cli::run(args),
        Cmd::Fires(args) => hooks_cli::fires(args),
        Cmd::Propose(args) => recipes_cli::propose(args).map(|()| ExitCode::SUCCESS),
        Cmd::Apply(args) => recipes_cli::apply(args),
        Cmd::Verify(args) => recipes_cli::verify(args),
        Cmd::Serve(_) => unreachable!("serve is dispatched before request execution"),
        Cmd::Mcp(_) => unreachable!("MCP is dispatched before request execution"),
        Cmd::McpCall(args) => mcp_cli::call(args)
            .and_then(|value| write(None, &value))
            .map(|()| ExitCode::SUCCESS),
        Cmd::Status(args) => daemon_cli::status(args).map(|()| ExitCode::SUCCESS),
        Cmd::Explain(args) => recompute(&args.graph, None)
            .and_then(|doc| {
                if args.for_agent {
                    explain_cli::for_agent(&doc, &[args.subject])
                } else {
                    explain_cli::explain(&doc, &args.subject)
                }
            })
            .and_then(|value| {
                if args.for_agent {
                    println!("{}", serde_json::to_string(&value)?);
                    Ok(())
                } else {
                    write(None, &value)
                }
            })
            .map(|()| ExitCode::SUCCESS),
        Cmd::Scan(args) => scan_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Report(args) => report_cli::run(args),
        Cmd::Check(args) => check_cli::run(args),
        Cmd::Render(args) => render(args).map(|()| ExitCode::SUCCESS),
        Cmd::Simulate(args) => simulate(args).map(|()| ExitCode::SUCCESS),
        Cmd::Diff(args) => diff(args),
        Cmd::History(args) => history_cli::run(args).map(|()| ExitCode::SUCCESS),
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
            "{}{} {} — {}",
            serde_json::to_string(&import.from)?,
            import
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default(),
            serde_json::to_string(&import.specifier)?,
            import
                .unresolved
                .map(archgraph::model::ImportReason::as_str)
                .unwrap_or_default()
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
        Language::Elixir,
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

fn simulate(args: Simulate) -> Result<()> {
    let [file, to] = args.moves.as_slice() else {
        bail!("--move takes one file and one destination");
    };
    let mut input = read_scan(&args.graph)?;
    input.hydrate_config();
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
        Some(path) => write_file(path, html.as_bytes()),
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
    if rules.is_none()
        && let Some(mut doc) = daemon_cli::cached_graph(path)
    {
        archgraph::recipes::rank(&mut doc, globals().rank);
        return Ok(doc);
    }
    let mut input = read_scan(path)?;
    if let Some(path) = rules {
        input.config.rules = Some(rules::load(path)?);
        reassign(&mut input)?;
    }
    let mut doc = archgraph::restore_scan(input)?;
    archgraph::recipes::rank(&mut doc, globals().rank);
    Ok(doc)
}

/// Reassigns modules under `input.config`. Scores and introductions were judged under the old
/// module/rule configuration, so they are dropped.
fn reassign(input: &mut ScanInput) -> Result<()> {
    let compiled = rules::compile(&input.config)?;
    for file in &mut input.files {
        file.module = modules::assigned_file(
            file,
            &input.config,
            input.source.language,
            compiled.as_ref(),
        );
    }
    input.findings.clear();
    input.hotspots = None;
    Ok(())
}

fn read_scan(path: &Path) -> Result<ScanInput> {
    if let Some(doc) = daemon_cli::cached_graph(path) {
        return Ok(serde_json::from_value(serde_json::to_value(doc)?)?);
    }
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
    input.config.all_kinds |= globals().all_kinds;
    archgraph::validate_scan(&input)
        .with_context(|| format!("inconsistent scan document {}", path.display()))?;
    if let Some(granularity) = globals().granularity {
        input.config.granularity = granularity;
        reassign(&mut input)?;
    }

    Ok(input)
}

/// Pretty JSON to `output`, or stdout.
fn write(output: Option<&Path>, value: &impl serde::Serialize) -> Result<()> {
    let mut value = serde_json::to_value(value)?;
    if globals().legacy_v1 {
        archgraph::compat::v1(&mut value)?;
    }
    if output.is_none() && globals().brief {
        value = brief_cli::project(&value);
    }
    let json = serde_json::to_string_pretty(&value)? + "\n";
    match output {
        Some(path) => write_file(path, json.as_bytes()),
        None => {
            print!("{json}");
            Ok(())
        }
    }
}

/// Replaces `path` atomically, so an interrupted run never leaves a truncated output.
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    history::write_atomic(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// Whether `root` is the clean checkout and directory `source` was scanned from. Errors when it
/// is at another commit or directory; `false` when the scan recorded neither or a checkout is
/// dirty. `outputs` are the caller's own files in `root`.
fn scanned_checkout(source: &Source, root: &Path, outputs: &[&Path]) -> Result<bool> {
    let Some(expected) = &source.git else {
        return Ok(false);
    };
    let actual = git(root, outputs).context("cannot verify the --root Git revision")?;
    if actual.commit != expected.commit {
        bail!("--root does not match the graph's Git revision; use its scanned checkout or rescan");
    }
    let Some(expected_root) = &source.scan_root else {
        return Ok(false);
    };
    if git_scan_root(root).as_ref() != Some(expected_root) {
        bail!("--root differs from the directory recorded by the scan; use its scanned root");
    }
    Ok(!actual.dirty && !expected.dirty)
}

fn git_scan_root(root: &Path) -> Option<String> {
    let prefix = archgraph::git::text(root, &["rev-parse", "--show-prefix"]).ok()?;
    let prefix = prefix.trim_end_matches('\n').trim_end_matches('/');
    Some(if prefix.is_empty() { "." } else { prefix }.to_owned())
}

/// The checkout's commit, and whether anything other than archgraph's own outputs changed:
/// `.archgraph/` and those of `outputs` inside `root`.
fn git(root: &Path, outputs: &[&Path]) -> Option<Git> {
    let canonical_root = fs::canonicalize(root).ok()?;
    let mut status = vec![
        "status".to_owned(),
        "--porcelain".into(),
        "--".into(),
        ":(exclude,literal).archgraph".into(),
    ];
    status.extend(outputs.iter().filter_map(|output| {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let path = fs::canonicalize(parent).ok()?.join(output.file_name()?);
        let relative = path.strip_prefix(&canonical_root).ok()?.to_str()?;
        Some(format!(":(exclude,literal){relative}"))
    }));
    let status: Vec<_> = status.iter().map(String::as_str).collect();
    Some(Git {
        commit: archgraph::git::text(root, &["rev-parse", "HEAD"])
            .ok()?
            .trim()
            .to_owned(),
        dirty: !archgraph::git::run_with_index(
            root,
            &status,
            daemon_cli::cached_index(root).as_deref(),
        )
        .ok()?
        .trim_ascii()
        .is_empty(),
    })
}
