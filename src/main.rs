use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The global flags, set once in `main`; defaults elsewhere, as in unit tests.
#[derive(Default)]
struct Globals {
    all_kinds: bool,
    legacy_v1: bool,
    granularity: Option<Granularity>,
}
static GLOBALS: OnceLock<Globals> = OnceLock::new();

fn globals() -> &'static Globals {
    GLOBALS.get_or_init(Globals::default)
}

use anyhow::{Context, Result, bail};
use archgraph::collect::{self, depcruise, elixir, go, grimp, java, rust};
use archgraph::diff::count;
use archgraph::model::{
    Config, CycleLevel, Document, Git, Granularity, Language, SCHEMA, ScanInput, Source, Verdict,
};
use archgraph::{cochange, codeowners, diff, history, modules, render, rules, simulate};
use clap::{Args, Parser, Subcommand, ValueEnum};
use sha2::Digest;

#[path = "cli/context.rs"]
mod context_cli;
#[path = "cli/history.rs"]
mod history_cli;
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
    /// Collect the import graph and write a scan document
    Scan(scan_cli::Scan),
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
struct Report {
    /// Repository root
    #[arg(default_value = ".")]
    root: PathBuf,
    #[command(flatten)]
    opts: ScanOpts,
    #[command(flatten)]
    hotspots: HotspotOpts,
    #[command(flatten)]
    collector: CollectorOpts,
    /// Directory for archgraph.json and archgraph.html [default: <root>/.archgraph]
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Skip bounded Git provenance probes
    #[arg(long)]
    no_provenance: bool,
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
    GLOBALS.get_or_init(|| Globals {
        all_kinds: cli.all_kinds,
        legacy_v1: cli.legacy_v1,
        granularity: cli.granularity,
    });
    let _ = collect::TOOL_TIMEOUT
        .set((cli.tool_timeout > 0).then(|| std::time::Duration::from_secs(cli.tool_timeout)));
    let result = match cli.command {
        Cmd::Scan(args) => scan_cli::run(args).map(|()| ExitCode::SUCCESS),
        Cmd::Report(args) => report(args),
        Cmd::Check(args) => check(args),
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

fn report(args: Report) -> Result<ExitCode> {
    let root = args.root.as_path();
    let rules = args.opts.rules.clone().or_else(|| {
        let found = root.join("layout.yaml");
        found.is_file().then_some(found)
    });
    let out_dir = args.out_dir.unwrap_or_else(|| root.join(".archgraph"));
    let mut doc = scan_cli::scan_doc(
        root,
        &args.opts,
        &args.hotspots,
        &args.collector,
        rules.as_deref(),
        &[&out_dir],
    )?;
    if !args.no_provenance && args.opts.collector_input.is_none() && doc.source.git.is_some() {
        provenance_cli::enrich(root, &args.collector, &mut doc)?;
    }
    let (json, html) = (
        out_dir.join("archgraph.json"),
        out_dir.join("archgraph.html"),
    );
    // The default folder lives in the scanned repository, which may ship it (or the files in it)
    // as links that would make this write land outside it.
    // ponytail: checked, not opened with O_NOFOLLOW; a link swapped in during the scan still wins.
    history::refuse_symlink(&out_dir).context("pass --out-dir to write elsewhere")?;
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    write(Some(&json), &doc)?;
    write_file(&html, render::render(&doc)?.as_bytes())?;

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
    if !doc.source.collector.limitations.is_empty() {
        out += &format!(
            "  {} collection limits recorded in source.collector.limitations\n",
            doc.source.collector.limitations.len()
        );
    }
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
    let mut by_kind: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &doc.findings {
        by_kind.entry(f.kind.as_str()).or_default().push(&f.subject);
    }
    let mut by_kind: Vec<_> = by_kind.into_iter().collect();
    by_kind.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    if by_kind.is_empty() {
        out += "  none\n";
    }
    for (kind, subjects) in &by_kind {
        out += &format!("  {kind} ({})\n", subjects.len());
        for subject in subjects.iter().take(3) {
            // Whole-tree findings have subject `.`: name what they are about.
            let subject = match (*kind, *subject) {
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
    let mut input = read_scan(path)?;
    if let Some(path) = rules {
        input.config.rules = Some(rules::load(path)?);
        reassign(&mut input)?;
    }
    archgraph::restore_scan(input)
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
        dirty: !archgraph::git::run(root, &status)
            .ok()?
            .trim_ascii()
            .is_empty(),
    })
}
