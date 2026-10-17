use super::*;
use archgraph::impact::{self, Impact};
use std::collections::BTreeSet;

#[derive(Args)]
pub(super) struct ImpactArgs {
    /// Two-dot Git range, for example origin/main..HEAD
    range: String,
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Reuse a scan of the head commit (include tests when collecting it)
    #[arg(long)]
    graph: Option<PathBuf>,
    /// Also traverse a scan of the base commit, including removed imports
    #[arg(long, requires = "graph")]
    base_graph: Option<PathBuf>,
    /// Maximum reverse-import distance; re-export hops do not consume depth
    #[arg(long, default_value_t = 3)]
    max_depth: usize,
    /// Additional test file glob (repeatable; conventions are always included)
    #[arg(long)]
    tests: Vec<String>,
    /// Export selected test paths, one per line
    #[arg(long)]
    tests_output: Option<PathBuf>,
    /// Export selected Go package directories, one per line
    #[arg(long)]
    go_packages_output: Option<PathBuf>,
    /// Exit 1 if any affected module is deeper than N (use --max-depth > N)
    #[arg(long)]
    fail_on_depth: Option<usize>,
    #[arg(long, value_enum, default_value_t = Format::Markdown)]
    format: Format,
    /// Write JSON independently of the displayed format
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write the step summary and post a new PR comment using gh
    #[arg(long)]
    github_actions: bool,
    /// PR number for --github-actions (default: pull_request.number in GITHUB_EVENT_PATH)
    #[arg(long, requires = "github_actions", value_parser = clap::value_parser!(u64).range(1..))]
    pr: Option<u64>,
    #[command(flatten)]
    opts: ScanOpts,
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .output()
        .context("running Git for impact")?;
    if !output.status.success() {
        bail!(
            "impact Git: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn resolve(root: &Path, rev: &str) -> Result<String> {
    Ok(String::from_utf8(git_output(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )?)?
    .trim()
    .into())
}

fn check_graph(doc: &Document, commit: &str) -> Result<()> {
    let git = doc
        .source
        .git
        .as_ref()
        .context("impact graph must record its Git commit")?;
    if git.commit != commit || git.dirty {
        bail!("impact graph must be a clean scan of {commit}; rescan the requested endpoint");
    }
    for file in &doc.files {
        if Path::new(&file.path).is_absolute()
            || Path::new(&file.path)
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            bail!(
                "impact graph paths must be relative to the scanned root: {}",
                file.path
            );
        }
    }
    Ok(())
}

fn enrich(root: &Path, doc: &mut Document, opts: &ScanOpts, changed: &[String]) -> Vec<String> {
    let mut limitations = Vec::new();
    if doc.hotspots.is_none() {
        match archgraph::hotspots::collect(
            root,
            doc,
            &archgraph::hotspots::Options {
                since: opts
                    .hotspots
                    .clone()
                    .unwrap_or_else(|| "12 months ago".into()),
                max_files: opts.hotspot_max_files,
                authors: opts.hotspot_authors,
            },
        ) {
            Ok(report) => archgraph::apply_hotspots(doc, report),
            Err(error) => limitations.push(format!("Hotspot history unavailable: {error:#}")),
        }
    }
    if doc.source.language == Language::Typescript {
        match super::impact_typescript::resolve_barrels(root, opts.depcruise.as_deref(), doc, changed) {
            Ok(count) if count > 0 => limitations.push(format!("Resolved {count} barrel import edges to their source declarations using TypeScript.")),
            Ok(_) => {},
            Err(error) => limitations.push(format!("Barrel symbol resolution unavailable; retaining conservative imports: {error:#}")),
        }
    }
    limitations
}

pub(super) fn run(mut args: ImpactArgs) -> Result<ExitCode> {
    let (base, head) = args
        .range
        .split_once("..")
        .context("impact expects BASE..HEAD")?;
    if base.is_empty() || head.is_empty() || head.starts_with('.') || head.contains("..") {
        bail!("impact expects two explicit endpoints: BASE..HEAD (not a three-dot range)");
    }
    let root = fs::canonicalize(&args.root)?;
    let base = resolve(&root, base)?;
    let head = resolve(&root, head)?;
    let changed: Vec<String> = std::str::from_utf8(&git_output(
        &root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "--no-renames",
            "-z",
            "--relative",
            &base,
            &head,
            "--",
            ".",
        ],
    )?)?
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_owned)
    .collect();

    args.opts.include_tests = true;
    args.opts.rules = args
        .opts
        .rules
        .as_deref()
        .map(fs::canonicalize)
        .transpose()?;
    args.opts.collector_input = args
        .opts
        .collector_input
        .as_deref()
        .map(fs::canonicalize)
        .transpose()?;
    args.opts.depcruise = args
        .opts
        .depcruise
        .as_deref()
        .map(fs::canonicalize)
        .transpose()?;
    args.opts.rust_analyzer = args
        .opts
        .rust_analyzer
        .as_deref()
        .map(fs::canonicalize)
        .transpose()?;
    for program in [
        &mut args.opts.python,
        &mut args.opts.go,
        &mut args.opts.java,
    ] {
        if program.contains(['/', '\\']) {
            *program = fs::canonicalize(&*program)?.to_string_lossy().into_owned();
        }
    }
    if args.opts.depcruise.is_none() {
        args.opts.depcruise = root
            .ancestors()
            .map(|p| p.join("node_modules/.bin/depcruise"))
            .find(|p| p.is_file());
    }
    let checkout = history::Checkout::new(&root)?;
    let head_root = checkout.at(&head)?;
    let mut doc = match &args.graph {
        Some(path) => recompute(path, None)?,
        None => {
            let local_rules = head_root.join("layout.yaml");
            scan_doc(
                &head_root,
                &args.opts,
                args.opts
                    .rules
                    .as_deref()
                    .or_else(|| local_rules.is_file().then_some(local_rules.as_path())),
            )?
        }
    };
    check_graph(&doc, &head)?;
    let context_limitations = enrich(&head_root, &mut doc, &args.opts, &changed);
    let mut result = impact::analyze(&doc, changed.clone(), args.max_depth, &args.tests)?;
    if !doc.config.include_tests {
        result.limitations.push(
            "The supplied graph excludes tests; rescan with --include-tests to select them.".into(),
        );
    }
    let mut go_limitations = Vec::new();
    if doc.source.language == Language::Go {
        match super::impact_context::go_tests(
            &head_root,
            &args.opts.go,
            &changed,
            &doc,
            &args.tests,
            &mut result,
        ) {
            Ok(()) => {}
            Err(error) => go_limitations.push(format!(
                "Go package test selection unavailable; graph selection only: {error:#}"
            )),
        }
    }
    // Removed files and imports are visible only in the base snapshot.
    let need_base = args.base_graph.is_some()
        || args.graph.is_none() && args.opts.collector_input.is_none()
        || !result.unscanned_files.is_empty();
    if need_base {
        let base_result = (|| -> Result<(Document, Impact)> {
            let base_root = checkout.at(&base)?;
            let mut base_doc = if let Some(path) = &args.base_graph {
                let graph = recompute(path, None)?;
                check_graph(&graph, &base)?;
                graph
            } else if args.graph.is_none() && args.opts.collector_input.is_none() {
                provenance_cli::probe(&base_root, &doc, &args.opts)?
            } else {
                bail!(
                    "supply --base-graph for removed files when reusing --graph or --collector-input"
                );
            };
            let limitations = enrich(&base_root, &mut base_doc, &args.opts, &changed);
            let mut base_result =
                impact::analyze(&base_doc, changed.clone(), args.max_depth, &args.tests)?;
            base_result.limitations.extend(limitations);
            if base_doc.source.language == Language::Go {
                match super::impact_context::go_tests(
                    &base_root,
                    &args.opts.go,
                    &changed,
                    &base_doc,
                    &args.tests,
                    &mut base_result,
                ) {
                    Ok(()) => {}
                    Err(error) => base_result.limitations.push(format!(
                        "Base Go package test selection unavailable: {error:#}"
                    )),
                }
            }
            Ok((base_doc, base_result))
        })();
        match base_result {
            Ok((base_doc, before)) => {
                result = impact::merge(result, before);
                // Retain removed files' ownership and module information for reviewers.
                let known: BTreeSet<_> = doc.files.iter().map(|f| f.path.clone()).collect();
                doc.files.extend(
                    base_doc
                        .files
                        .into_iter()
                        .filter(|f| !known.contains(&f.path)),
                );
            }
            Err(error) if args.base_graph.is_some() => return Err(error),
            Err(error) => result.limitations.push(format!(
                "Base impact unavailable; deleted-file dependents may be missing: {error:#}"
            )),
        }
    }
    result.limitations.extend(go_limitations);
    result.limitations.extend(context_limitations);
    if let Err(error) = super::impact_context::reviewers(&root, &head, &doc, &mut result) {
        result
            .limitations
            .push(format!("Git reviewer history unavailable: {error:#}"));
    }
    // Select only tests that still exist at the endpoint; deleted tests cannot be run.
    let head_root = checkout.at(&head)?;
    result
        .selected_tests
        .retain(|path| head_root.join(path).is_file());
    result
        .go_test_packages
        .retain(|path| head_root.join(path).is_dir());
    result.go_test_packages.sort();
    result.go_test_packages.dedup();
    result.limitations.sort();
    result.limitations.dedup();
    if let Some(path) = &args.tests_output {
        export_lines(path, &result.selected_tests)?;
    }
    if let Some(path) = &args.go_packages_output {
        export_lines(path, &result.go_test_packages)?;
    }
    let mut markdown = markdown(&result);
    if args.github_actions {
        use std::io::Write;
        let number = args.pr.or_else(|| {
            let event = fs::read(std::env::var_os("GITHUB_EVENT_PATH")?).ok()?;
            serde_json::from_slice::<serde_json::Value>(&event).ok()?["pull_request"]["number"]
                .as_u64()
        });
        let post = (|| -> Result<()> {
            let number = number.context("no PR number; pass --pr or set GITHUB_EVENT_PATH")?;
            let body = std::env::temp_dir().join(format!(
                "archgraph-impact-{}-{}.md",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
            ));
            // Create outside the checkout so repository symlinks cannot redirect this write.
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&body)?;
            file.write_all(markdown.as_bytes())?;
            let output = Command::new("gh")
                .current_dir(&root)
                .args(["pr", "comment", &number.to_string(), "--body-file"])
                .arg(&body)
                .output();
            if let Err(error) = fs::remove_file(&body) {
                eprintln!("note: PR comment body cleanup failed: {error}");
            }
            let output = output.context("running gh pr comment")?;
            if !output.status.success() {
                bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
            }
            Ok(())
        })();
        if let Err(error) = post {
            result
                .limitations
                .push(format!("PR comment not posted: {error:#}"));
            markdown = self::markdown(&result);
        }
        if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
            fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)?
                .write_all(markdown.as_bytes())?;
        }
    }
    if let Some(path) = &args.output {
        write(Some(path), &result)?;
    }
    match args.format {
        Format::Json => write(None, &result)?,
        Format::Markdown => print!("{markdown}"),
    }
    Ok(
        if args
            .fail_on_depth
            .is_some_and(|limit| result.affected_modules.iter().any(|m| m.depth > limit))
        {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        },
    )
}

fn export_lines(path: &Path, lines: &[String]) -> Result<()> {
    if lines.iter().any(|line| line.contains(['\n', '\r'])) {
        bail!("cannot export newline-containing test paths as a line list; use JSON");
    }
    fs::write(
        path,
        if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n") + "\n"
        },
    )
    .with_context(|| format!("writing {}", path.display()))
}

fn code(text: &str) -> String {
    format!("`{}`", text.replace(['`', '\n', '\r'], " "))
}

fn markdown(report: &Impact) -> String {
    use std::fmt::Write;
    let mut text = format!(
        "## Architecture impact\n\nAt least these: {} changed files, {} affected modules, {} selected test files. Risk: **{}**.\n",
        report.changed_files.len(),
        report.affected_modules.len(),
        report.selected_tests.len(),
        if report.risk {
            "cycle or hotspot"
        } else {
            "none detected"
        }
    );
    text.push_str("\nChanged files:\n\n");
    for path in report.changed_files.iter().take(100) {
        writeln!(text, "- {}", code(path)).unwrap();
    }
    text.push_str("\nAffected modules (shortest depth, decaying weight):\n\n");
    for module in report.affected_modules.iter().take(100) {
        writeln!(
            text,
            "- {} — depth {}, weight {:.2}{}{}",
            code(&module.id),
            module.depth,
            module.weight,
            if module.in_cycle { ", cycle" } else { "" },
            if module.hotspot { ", hotspot" } else { "" }
        )
        .unwrap();
    }
    text.push_str("\nTests to run:\n\n");
    for path in report.selected_tests.iter().take(100) {
        writeln!(text, "- {}", code(path)).unwrap();
    }
    if !report.go_test_packages.is_empty() {
        text.push_str("\nRun Go tests by package:\n\n");
        for package in report.go_test_packages.iter().take(100) {
            writeln!(text, "- {}", code(package)).unwrap();
        }
    }
    text.push_str("\nSuggested reviewers:\n\n");
    for reviewer in report.reviewers.iter().take(100) {
        writeln!(
            text,
            "- {} ({}, {} commits) — {}",
            code(&reviewer.identity),
            code(&reviewer.source),
            reviewer.commits,
            reviewer
                .modules
                .iter()
                .map(|m| code(m))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    if report
        .changed_files
        .len()
        .max(report.affected_modules.len())
        .max(report.selected_tests.len())
        .max(report.reviewers.len())
        .max(report.go_test_packages.len())
        > 100
    {
        text.push_str(
            "\nEach list is capped at 100; JSON and exported lists contain all results.\n",
        );
    }
    for limitation in &report.limitations {
        writeln!(text, "\nnote: {}", code(limitation)).unwrap();
    }
    text
}
