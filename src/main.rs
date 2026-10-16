use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use archgraph::collect::{
    self, DEFAULT_EXCLUDES, GO_EXCLUDES, GO_TEST_EXCLUDES, TEST_EXCLUDES, depcruise, go, grimp,
};
use archgraph::model::{
    Config, CycleLevel, Document, Git, Language, SCHEMA, ScanInput, Source, Verdict,
};
use archgraph::{cochange, diff, modules, render, rules, simulate};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(version, about = "Import-graph evidence for architecture decisions")]
struct Cli {
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
}

/// What `scan` and `report` share.
#[derive(Args)]
struct ScanOpts {
    /// Directory segments below the source root that form a module [default: 2; Go: each
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
    /// Go toolchain (1.21 or later) that runs the Go helper
    #[arg(long, default_value = "go")]
    go: String,
    /// layout.yaml with modules, layers, public entries and forbid/allow rules
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Read the last N commits (default 300) for files that change together
    #[arg(long, value_name = "N", num_args = 0..=1, require_equals = true, default_missing_value = "300",
          value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    cochange: Option<usize>,
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
    #[arg(long = "move", num_args = 2, value_names = ["FILE", "DEST"], required = true)]
    moves: Vec<String>,
    /// Rules replacing the ones embedded in the document
    #[arg(long)]
    rules: Option<PathBuf>,
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Cmd::Scan(args) => scan(args).map(|()| ExitCode::SUCCESS),
        Cmd::Report(args) => report(args),
        Cmd::Check(args) => check(args),
        Cmd::Render(args) => render(args).map(|()| ExitCode::SUCCESS),
        Cmd::Simulate(args) => simulate(args).map(|()| ExitCode::SUCCESS),
        Cmd::Diff(args) => diff(args),
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
    let language = match args.language {
        Some(language) => language,
        None => collect::detect_language(root, source_dir)?,
    };
    let mut excludes: Vec<String> = Vec::new();
    if !args.no_default_excludes {
        let (generated, tests): (&[&str], &[&str]) = if language == Language::Go {
            (GO_EXCLUDES, GO_TEST_EXCLUDES)
        } else {
            (&[], &[])
        };
        excludes.extend(
            DEFAULT_EXCLUDES
                .iter()
                .chain(generated)
                .map(|g| g.to_string()),
        );
        if !args.include_tests {
            excludes.extend(TEST_EXCLUDES.iter().chain(tests).map(|g| g.to_string()));
        }
    }
    excludes.extend(args.exclude.iter().cloned());

    let input = args
        .collector_input
        .as_ref()
        .map(|p| fs::read(p).with_context(|| format!("reading {}", p.display())))
        .transpose()?;
    let excludes_set = collect::globset(&excludes)?;
    // Go: files that cannot be split across packages, in pairs.
    let mut bound = Vec::new();
    let (raw, source_root) = match language {
        Language::Typescript => {
            let (json, type_marking) = match input {
                Some(json) => (json, None),
                None => depcruise::run(root, source_dir, depcruise.as_deref())?,
            };
            let mut raw = depcruise::parse(&json)?;
            raw.collector.type_marking = type_marking.map(str::to_owned);
            (raw, source_dir.to_owned())
        }
        Language::Python => {
            let package = grimp::package(root, source_dir, args.package.as_deref(), &excludes_set)?;
            let json = match input {
                Some(json) => json,
                None => grimp::run(root, source_dir, &package, &args.python)?,
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
            let keep_tests = args.include_tests || args.no_default_excludes;
            let parsed = go::parse(&json, args.no_default_excludes, keep_tests)?;
            if !parsed.left_out.is_empty() {
                eprintln!(
                    "warning: left out as test support (--include-tests keeps them): {}",
                    parsed.left_out.join(", ")
                );
            }
            bound = parsed.bound;
            (parsed.raw, ".".to_owned())
        }
    };
    let mut raw = raw.exclude(&excludes_set);
    if !args.no_default_excludes {
        raw = raw.exclude_ignored_output(root)?;
    }
    raw.collector.bound_files = go::bound_files(&bound, &raw.files);
    if let Some(n) = raw.collector.unresolved.filter(|&n| n > 0) {
        eprintln!(
            "warning: {n} unresolved import(s) reported by {}; their edges are missing",
            raw.collector.name
        );
    }

    let source = Source {
        language,
        collector: raw.collector,
        git: git(root),
        generated_at: now_utc(),
    };
    // Go packages are the language's own modules; merging them by folder invents cycles Go
    // forbids.
    let package_modules = language == Language::Go && args.depth.is_none();
    let config = Config {
        depth: args.depth.unwrap_or(if package_modules { 0 } else { 2 }),
        package_modules,
        source_root,
        excludes,
        include_tests: args.include_tests || args.no_default_excludes,
        rules: rules.map(rules::load).transpose()?,
    };
    let cochange = args
        .cochange
        .map(|n| cochange::collect(root, n, &raw.files))
        .transpose()?;
    let entries = match language {
        Language::Typescript => depcruise::package_entries(root, &raw.files),
        Language::Python | Language::Go => Vec::new(),
    };
    archgraph::analyze(source, config, raw.files, raw.edges, entries, cochange)
}

fn report(args: Report) -> Result<ExitCode> {
    let root = args.root.as_path();
    let rules = args.opts.rules.clone().or_else(|| {
        let found = root.join("layout.yaml");
        found.is_file().then_some(found)
    });
    let doc = scan_doc(root, &args.opts, rules.as_deref())?;
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
    if rules.is_some() {
        out += &format!("\n{}\n", heading("Rules"));
        for (verdict, label) in [
            (Verdict::Forbidden, "forbidden"),
            (Verdict::Unplanned, "unplanned"),
        ] {
            let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
            for e in doc.edges.iter().filter(|e| e.verdict == Some(verdict)) {
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

fn check(args: Check) -> Result<ExitCode> {
    let doc = recompute(&args.graph, args.rules.as_deref())?;
    if doc.config.rules.is_none() {
        bail!("no rules: pass --rules, or scan with --rules");
    }
    let (mut forbidden, mut unplanned) = (0, 0);
    for e in &doc.edges {
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
            Some(path) => format!(" via {}", path.join(" -> ")),
            None => String::new(),
        };
        let rule = e.rule.as_deref().unwrap_or_default();
        println!("{label} {} -> {} ({rule}){via}", e.from, e.to);
    }
    println!("{forbidden} forbidden, {unplanned} unplanned");
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
    let rules = rules::compile(&config)?;
    let files = input.files.into_iter().map(|f| f.path).collect();
    let doc = archgraph::build(
        input.source,
        config,
        files,
        input.edges,
        input.package_entries,
        rules.as_ref(),
    );
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
    let input = read_scan(path)?;
    let mut config = input.config;
    if let Some(path) = rules {
        config.rules = Some(rules::load(path)?);
    }
    let files = input.files.into_iter().map(|f| f.path).collect();
    archgraph::analyze(
        input.source,
        config,
        files,
        input.edges,
        input.package_entries,
        input.cochange,
    )
}

fn read_scan(path: &Path) -> Result<ScanInput> {
    let text = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let input: ScanInput = serde_json::from_slice(&text)
        .with_context(|| format!("reading scan document {}", path.display()))?;
    if input.schema != SCHEMA || input.kind != "scan" {
        bail!(
            "{} is `{}` `{}`, expected a `{SCHEMA}` scan document",
            path.display(),
            input.schema,
            input.kind
        );
    }
    consistent(&input).with_context(|| format!("inconsistent scan document {}", path.display()))?;
    Ok(input)
}

/// Every file listed once, with the module its embedded config gives it, and every edge between
/// two listed files: what the recomputation relies on.
fn consistent(input: &ScanInput) -> Result<()> {
    let rules = rules::compile(&input.config)?;
    let mut paths = HashSet::new();
    for f in &input.files {
        if !paths.insert(f.path.as_str()) {
            bail!("{} is listed twice", f.path);
        }
        let module = rules
            .as_ref()
            .and_then(|r| r.module_of(&f.path))
            .unwrap_or_else(|| modules::module_of(&f.path, &input.config, input.source.language));
        if f.module != module {
            bail!(
                "{} has module `{}`, its config gives `{module}`",
                f.path,
                f.module
            );
        }
    }
    for entry in &input.package_entries {
        if !paths.contains(entry.as_str()) {
            bail!("package entry {entry} is not in files");
        }
    }
    for e in &input.edges {
        for end in [&e.from, &e.to] {
            if !paths.contains(end.as_str()) {
                bail!(
                    "edge {} -> {} names {end}, which is not in files",
                    e.from,
                    e.to
                );
            }
        }
    }
    Ok(())
}

/// Pretty JSON to `output`, or stdout.
fn write(output: Option<&Path>, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)? + "\n";
    match output {
        Some(path) => fs::write(path, json).with_context(|| format!("writing {}", path.display())),
        None => {
            print!("{json}");
            Ok(())
        }
    }
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
