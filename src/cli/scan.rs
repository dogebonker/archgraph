use super::*;
use archgraph::cochange;
use archgraph::collect::{depcruise, elixir, go, grimp, java};
use archgraph::model::Config;

#[derive(Args)]
pub(super) struct Scan {
    /// Repository root
    root: PathBuf,
    #[command(flatten)]
    pub(super) opts: ScanOpts,
    #[command(flatten)]
    hotspots: HotspotOpts,
    #[command(flatten)]
    pub(super) collector: CollectorOpts,
    /// Write the document here instead of stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(super) fn run(args: Scan) -> Result<()> {
    let outputs: Vec<&Path> = args.output.as_deref().into_iter().collect();
    let doc = scan_doc(
        &args.root,
        &args.opts,
        &args.hotspots,
        &args.collector,
        args.opts.rules.as_deref(),
        &outputs,
    )?;
    write(args.output.as_deref(), &doc)
}

/// Runs the collector under `root` and analyzes its output with `rules`. `outputs` are the
/// files this run writes, which do not make the checkout dirty.
pub(super) fn scan_doc(
    root: &Path,
    args: &ScanOpts,
    hotspots: &HotspotOpts,
    collector: &CollectorOpts,
    rules: Option<&Path>,
    outputs: &[&Path],
) -> Result<Document> {
    // Resolved here: a relative program path is not reliably resolved against the child's
    // working directory.
    let depcruise = (args.engine != collect::Engine::Native)
        .then_some(collector.depcruise.as_ref())
        .flatten()
        .map(|p| collect::resolve_executable("depcruise", p))
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
    if args.engine == collect::Engine::Depcruise && language != Language::Typescript {
        bail!(
            "--engine depcruise supports TypeScript/JavaScript; use --engine reference for {language:?}"
        );
    }
    if args.engine == collect::Engine::Native && input.is_some() {
        bail!("--engine native reads source; --collector-input belongs to the reference engine");
    }
    let mut native_symbols = None;
    let excludes_set = collect::globset(&discovery_excludes)?;
    // Go: files that cannot be split across packages, in pairs.
    let mut bound = Vec::new();
    let mut generated = Vec::new();
    let mut test_support = Vec::new();
    let mut python_package = None;
    let (raw, source_root) = if args.engine == collect::Engine::Native {
        let mut output = match language {
            Language::Typescript => collect::native::typescript::collect(
                root,
                source_dir,
                &policy,
                args.include_tests
                    || args.include_generated
                    || args.no_default_excludes
                    || !includes.is_empty(),
            )?,
            Language::Python => {
                let output = collect::native::python::collect(
                    root,
                    source_dir,
                    args.package.as_deref(),
                    &args.python_path,
                    &policy,
                )?;
                python_package = Some(
                    output
                        .source_root
                        .strip_prefix(&format!("{source_dir}/"))
                        .unwrap_or(&output.source_root)
                        .replace('/', "."),
                );
                output
            }
            Language::Go => collect::native::go::collect(root, &collector.go, &policy)?,
            Language::Rust => collect::native::rust::collect(root, &policy)?,
            Language::Java => collect::native::java::collect(root, &policy)?,
            Language::Elixir => collect::native::elixir::collect(root, &policy)?,
        };
        if output
            .raw
            .imports
            .as_ref()
            .is_some_and(|imports| imports.iter().any(|import| import.unresolved.is_some()))
        {
            output.symbols.complete = false;
            output.symbols.limitations.push("Unresolved requests can hide local symbol consumers; unused-export suggestions are withheld.".into());
        }
        output.symbols.limitations.sort();
        output.symbols.limitations.dedup();
        output
            .raw
            .collector
            .limitations
            .clone_from(&output.symbols.limitations);
        for limit in output.raw.collector.limitations.iter().take(10) {
            eprintln!("warning: {limit}");
        }
        native_symbols = Some(output.symbols);
        (output.raw, output.source_root)
    } else {
        match language {
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
                let mut sources = collect::imports::Sources::new(root)?;
                let (json, imports) = collect::typescript_resolution::enrich(&json, &mut sources)?;
                let mut raw = depcruise::parse_at(&json, &mut sources)?;
                raw.imports = imports;
                for warning in collect::barrels::typescript(&json, &mut raw, &mut sources)? {
                    eprintln!("warning: {warning}");
                }
                raw.collector.type_marking = type_marking.map(str::to_owned);
                (raw, source_dir.to_owned())
            }
            Language::Python => {
                let package = grimp::package(
                    root,
                    source_dir,
                    args.package.as_deref(),
                    &collect::globset(&excludes)?,
                )?;
                python_package = Some(package.clone());
                let json = match input {
                    Some(json) => json,
                    None => grimp::run(
                        root,
                        source_dir,
                        &package,
                        &collector.python,
                        args.include_tests || args.no_default_excludes || !includes.is_empty(),
                        &args.python_path,
                    )?,
                };
                (
                    grimp::parse(&json, root, source_dir)?,
                    modules::join(source_dir, &package),
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
                        go::run(root, &collector.go, &dirs)?
                    }
                };
                let parsed = go::parse(&json)?;
                if !args.include_tests
                    && !args.no_default_excludes
                    && includes.is_empty()
                    && !parsed.left_out.is_empty()
                {
                    eprintln!(
                        "warning: left out as test support (--include-tests keeps them): {}",
                        parsed.left_out.join(", ")
                    );
                }
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
                            rust_analyzer: collector.rust_analyzer.as_deref(),
                            features: args.rust_features,
                            build_scripts: collector.rust_build_scripts,
                        },
                    )?,
                };
                (rust::parse(&json)?, ".".to_owned())
            }
            Language::Elixir => {
                let json = match input {
                    Some(json) => json,
                    None => elixir::run(root, &collector.elixir, &policy)?,
                };
                (elixir::parse(&json)?, ".".to_owned())
            }
            Language::Java => {
                let json = match input {
                    Some(json) => json,
                    None => java::run(root, &collector.java, &excludes_set)?,
                };
                (java::parse(&json)?, ".".to_owned())
            }
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
    if args.engine != collect::Engine::Native {
        raw.collector.bound_files = go::bound_files(&bound, &raw.files);
    }
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
        git: git(root, outputs),
        generated_at: now_utc(),
        scan_root: git_scan_root(root),
    };
    // Keep package directories intact unless a depth is explicitly requested.
    let package_modules = matches!(language, Language::Go | Language::Java) && args.depth.is_none();
    let config = Config {
        depth: args.depth.unwrap_or(if package_modules { 0 } else { 2 }),
        granularity: globals().granularity.unwrap_or_default(),
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
        all_kinds: globals().all_kinds,
        no_default_excludes: args.no_default_excludes,
        rules: loaded_rules,
        codeowners: codeowners::load(root)?,
        collection: (args.engine == collect::Engine::Native
            || matches!(language, Language::Python | Language::Rust))
        .then_some(archgraph::model::CollectionOptions {
            engine: args.engine,
            python_package,
            python_paths: args
                .python_path
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            rust_features: (language == Language::Rust && args.engine != collect::Engine::Native)
                .then_some(args.rust_features),
            rust_build_scripts: language == Language::Rust
                && args.engine != collect::Engine::Native
                && collector.rust_build_scripts,
        }),
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
        Language::Python | Language::Go | Language::Rust | Language::Java | Language::Elixir => {
            Vec::new()
        }
    };
    let mut doc = archgraph::analyze(source, config, raw.files, raw.edges, entries, cochange)?;
    if args.symbols {
        let symbols = if let Some(mut symbols) = native_symbols {
            let kept: std::collections::BTreeSet<_> =
                doc.files.iter().map(|file| file.path.as_str()).collect();
            symbols.exports.retain(|export| {
                kept.contains(export.file.as_str()) && kept.contains(export.target.file.as_str())
            });
            symbols.imports.retain(|import| {
                kept.contains(import.from.as_str())
                    && kept.contains(import.to.as_str())
                    && import
                        .target
                        .as_ref()
                        .is_none_or(|target| kept.contains(target.file.as_str()))
            });
            symbols
        } else {
            archgraph::symbols::collect(
                root,
                &doc,
                archgraph::symbols::Options {
                    depcruise: depcruise.as_deref(),
                    python: &collector.python,
                    go: &collector.go,
                },
            )?
        };
        archgraph::surface::attach(&mut doc, symbols, archgraph::surface::DEFAULT_MAX_SYMBOLS)?;
    }
    if let Some(since) = &hotspots.hotspots {
        let report = archgraph::hotspots::collect(
            root,
            &doc,
            &archgraph::hotspots::Options {
                since: since.clone(),
                max_files: hotspots.hotspot_max_files,
                authors: hotspots.hotspot_authors,
            },
        )?;
        archgraph::apply_hotspots(&mut doc, report);
    }
    Ok(doc)
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
