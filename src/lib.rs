macro_rules! eprintln { ($($arg:tt)*) => { $crate::output::stderr(format_args!("{}\n", format_args!($($arg)*))) }; }
// Library half of the CLI and daemon, shared with integration tests.

pub mod adr;
pub mod apply;
pub mod bisect;
pub mod cochange;
pub mod codeowners;
pub mod collect;
pub mod compat;
pub mod context;
pub mod diff;
pub mod early_smells;
pub mod edit_audit;
pub mod edit_gate;
pub mod exposure;
pub mod findings;
pub mod git;
pub mod grabbag;
pub mod graph;
pub mod history;
pub mod hook_agents;
pub mod hotspots;
pub mod impact;
pub mod incremental;
pub mod infer_layout;
pub mod known;
pub mod mcp_tools;
pub mod model;
pub mod modules;
pub mod output;
pub mod package_splits;
pub mod presets;
pub mod quality;
pub mod recipes;
pub mod render;
pub mod resolution;
pub mod rewrite;
pub mod rules;
pub mod sarif;
pub mod services;
pub mod simulate;
pub mod surface;
pub mod symbols;
pub mod sync_rules;
pub mod target;
pub mod tour;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use model::{Cochange, Config, Document, Edge, File, SCHEMA, Source, Summary, Tool};
use rules::Compiled;

/// Rebuild a persisted scan, retaining its optional historical context.
pub fn restore_scan(mut input: model::ScanInput) -> Result<Document> {
    validate_scan(&input)?;
    input.hydrate_config();
    let mut doc = analyze(
        input.source,
        input.config,
        input.files.into_iter().map(|f| f.path).collect(),
        input.edges,
        input.package_entries,
        input.cochange,
    )?;
    let threshold = input
        .surface
        .as_ref()
        .map_or(surface::DEFAULT_MAX_SYMBOLS, |s| s.max_symbols);
    if let Some(symbols) = input.symbols {
        surface::attach(&mut doc, symbols, threshold)?;
    } else if input.surface.is_some() {
        doc.surface = Some(surface::analyze(&doc, threshold)?);
    }
    recipes::attach(&mut doc)?;
    for saved in input
        .findings
        .into_iter()
        .chain(input.diagnostics.into_iter().map(|d| d.finding))
    {
        if let Some(finding) = doc
            .findings
            .iter_mut()
            .chain(doc.diagnostics.iter_mut().map(|d| &mut d.finding))
            .find(|f| f.subject == saved.subject && f.kind.as_str() == saved.kind)
        {
            finding.age_days = saved.age_days;
            finding.introduced_by = saved.introduced_by;
            finding.provenance = saved.provenance;
        }
    }
    if let Some(report) = input.hotspots {
        if report
            .files
            .iter()
            .any(|h| !doc.files.iter().any(|f| f.path == h.path))
        {
            bail!("hotspot report names a file outside the scan");
        }
        apply_hotspots(&mut doc, report);
    }
    if let Some(saved) = input.package_splits {
        if !model::supported_schema(&saved.schema) || saved.kind != "package-splits" {
            bail!("expected a package-splits report in the scan attachment");
        }
        doc.package_splits = Some(package_splits::analyze(
            &doc,
            saved.options,
            saved.workspace,
        )?);
    }
    Ok(doc)
}

pub fn validate_scan(input: &model::ScanInput) -> Result<()> {
    if !model::supported_schema(&input.schema) || input.kind != "scan" {
        bail!("expected an {SCHEMA} scan document");
    }
    let rules = rules::compile(&input.config)?;
    input
        .config
        .codeowners
        .as_ref()
        .map(codeowners::compile)
        .transpose()?;
    let mut config = input.config.clone();
    config.import_records = input.imports.clone();
    config.globals = input.globals.clone();
    // Saved modules follow the document's own granularity, also when exported as legacy v1.
    config.legacy_module_classes = false;
    let mut paths = BTreeSet::new();
    for file in &input.files {
        if !paths.insert(file.path.as_str()) {
            bail!("{} is listed twice", file.path);
        }
        let module = modules::assigned_file(file, &config, input.source.language, rules.as_ref());
        if file.module != module {
            bail!(
                "{} has module `{}`, its config gives `{module}`",
                file.path,
                file.module
            );
        }
    }
    for path in &input.package_entries {
        if !paths.contains(path.as_str()) {
            bail!("package entry {path} is not in files");
        }
    }
    for edge in &input.edges {
        for path in [&edge.from, &edge.to] {
            if !paths.contains(path.as_str()) {
                bail!(
                    "edge {} -> {} names {path}, which is not in files",
                    edge.from,
                    edge.to
                );
            }
        }
        if let Some(path) = &edge.original_to
            && (path.starts_with('/')
                || path
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == ".."))
        {
            bail!("written import destination must be a relative path inside the scan root");
        }
    }
    for pair in &input.source.collector.bound {
        if !paths.contains(pair.from.as_str()) || !paths.contains(pair.to.as_str()) {
            bail!(
                "bound pair {} -> {} names a file outside the scan",
                pair.from,
                pair.to
            );
        }
        if pair.names.iter().any(|n| n.chars().any(char::is_control)) {
            bail!("bound names must not contain control characters");
        }
    }
    resolution::validate(
        input.imports.as_deref(),
        input.globals.as_deref(),
        &input.files,
    )?;
    resolution::validate_external_identities(
        &input.source,
        input.imports.as_deref(),
        &input.files,
    )?;
    resolution::require_policy_accounting(&config)?;
    Ok(())
}

pub fn apply_hotspots(doc: &mut Document, mut report: hotspots::Hotspots) {
    hotspots::attach_findings(&mut report, doc);
    let priorities = hotspots::finding_priorities(&report, doc.findings.len());
    for (finding, priority) in doc.findings.iter_mut().zip(priorities) {
        finding.hotspot_priority = Some(priority);
    }
    recipes::rank(doc, recipes::Rank::Recipe);
    hotspots::attach_findings(&mut report, doc);
    doc.hotspots = Some(report);
}

/// The one pipeline every subcommand calls: module assignment, barrels, verdicts under
/// `config.rules`, metrics, cycles and findings from collected files and edges (and git
/// co-changes, when read). Every edge must join two of `files`.
pub fn analyze(
    source: Source,
    config: Config,
    files: Vec<String>,
    edges: Vec<Edge>,
    package_entries: Vec<String>,
    cochange: Option<Cochange>,
) -> Result<Document> {
    analyze_inner(
        source,
        config,
        files,
        edges,
        package_entries,
        cochange,
        None,
    )
}
/// Shared analysis with cached SCC/metric regions for a daemon generation.
#[allow(clippy::too_many_arguments)]
pub fn analyze_cached(
    source: Source,
    config: Config,
    files: Vec<String>,
    edges: Vec<Edge>,
    package_entries: Vec<String>,
    cochange: Option<Cochange>,
    cache: &mut incremental::Analysis,
) -> Result<Document> {
    analyze_inner(
        source,
        config,
        files,
        edges,
        package_entries,
        cochange,
        Some(cache),
    )
}
#[allow(clippy::too_many_arguments)]
fn analyze_inner(
    source: Source,
    config: Config,
    files: Vec<String>,
    edges: Vec<Edge>,
    package_entries: Vec<String>,
    cochange: Option<Cochange>,
    cache: Option<&mut incremental::Analysis>,
) -> Result<Document> {
    let rules = rules::compile(&config)?;
    resolution::require_policy_accounting(&config)?;
    let owners = config
        .codeowners
        .as_ref()
        .map(codeowners::compile)
        .transpose()?;
    let mut doc = build_inner(
        source,
        config,
        files,
        edges,
        package_entries,
        rules.as_ref(),
        owners.as_ref(),
        cache,
    );
    if doc.modules.iter().any(|m| {
        ![
            m.weighted_ca,
            m.weighted_ce,
            m.weighted_internal_edges,
            m.weighted_outgoing_edges,
            m.weighted_ca + m.weighted_ce,
            m.weighted_internal_edges + m.weighted_outgoing_edges,
        ]
        .into_iter()
        .all(f64::is_finite)
    }) {
        bail!("edge_weights overflow the weighted metrics; use smaller finite weights");
    }
    resolution::validate(doc.imports.as_deref(), doc.globals.as_deref(), &doc.files)?;
    exposure::validate(&doc)?;
    findings::place_registrars(&mut doc, rules.as_ref(), owners.as_ref());
    doc.cochange = cochange;
    doc.findings = findings::find(&doc, rules.as_ref(), owners.as_ref());
    doc.target = rules
        .as_ref()
        .and_then(|rules| target::analyze(&doc, rules));
    resolution::attach(&mut doc);
    recipes::attach(&mut doc)?;
    Ok(doc)
}

/// `analyze` without findings, with the rules already compiled from `config.rules`.
pub fn build(
    source: Source,
    config: Config,
    files: Vec<String>,
    edges: Vec<Edge>,
    package_entries: Vec<String>,
    rules: Option<&Compiled>,
    owners: Option<&codeowners::Compiled>,
) -> Document {
    build_inner(
        source,
        config,
        files,
        edges,
        package_entries,
        rules,
        owners,
        None,
    )
}
#[allow(clippy::too_many_arguments)]
fn build_inner(
    source: Source,
    config: Config,
    files: Vec<String>,
    mut edges: Vec<Edge>,
    mut package_entries: Vec<String>,
    rules: Option<&Compiled>,
    owners: Option<&codeowners::Compiled>,
    cache: Option<&mut incremental::Analysis>,
) -> Document {
    let weights = config
        .rules
        .as_ref()
        .and_then(|r| r.doc.edge_weights)
        .unwrap_or_default();
    for edge in &mut edges {
        if edge.kind == model::EdgeKind::TypeReexport {
            edge.is_reexport = true;
            edge.kind = model::EdgeKind::Type;
        }
        edge.weight = Some(weights.weight(edge.kind));
    }
    package_entries.sort();
    package_entries.dedup();
    edges.sort_by(|a, b| {
        (&a.from, &a.to, &a.original_to, a.kind).cmp(&(&b.from, &b.to, &b.original_to, b.kind))
    });
    let tests: BTreeSet<_> = files
        .iter()
        .filter(|path| modules::is_test(path, &config))
        .cloned()
        .collect();
    let module_of: BTreeMap<String, String> = files
        .into_iter()
        .map(|f| {
            let mut module = modules::assigned(&f, &config, source.language, rules);
            if !config.legacy_module_classes && tests.contains(&f) {
                module = format!("tests::{module}");
            }
            (f, module)
        })
        .collect();
    let (external_nodes, mut external_edges) = resolution::external_graph(&source, &config);
    let internal_count = edges.len();
    edges.append(&mut external_edges);
    match rules {
        Some(rules) => {
            let mut policy_modules: BTreeMap<_, _> = module_of
                .keys()
                .map(|path| {
                    (
                        path.clone(),
                        rules
                            .module_of(path)
                            .unwrap_or_else(|| modules::folder_of(path, &config, source.language)),
                    )
                })
                .collect();
            for path in edges.iter().filter_map(|e| e.original_to.as_ref()) {
                policy_modules.entry(path.clone()).or_insert_with(|| {
                    rules
                        .module_of(path)
                        .unwrap_or_else(|| modules::folder_of(path, &config, source.language))
                });
            }
            rules.decide_with_externals(
                &mut edges,
                &policy_modules,
                &external_nodes
                    .iter()
                    .map(|n| (n.id.clone(), n.package.clone()))
                    .collect(),
            );
        }
        None => {
            for e in &mut edges {
                (e.verdict, e.rule, e.reach) = (None, None, None);
            }
        }
    }
    let external_edges = edges.split_off(internal_count);
    let inferred = modules::barrels(&edges);
    let barrels = module_of
        .keys()
        .filter(|path| {
            config
                .file_metadata
                .get(*path)
                .map_or_else(|| inferred.contains(*path), |m| m.barrel)
        })
        .cloned()
        .collect();
    let pairs = modules::weighted(&edges, &barrels);
    let entries = modules::cycle_entries(module_of.keys(), &config.source_root, &package_entries);
    let module_edges = graph::module_edges(&module_of, &edges);
    let (cycles, type_only_cycles, mut metrics) = if let Some(cache) = cache {
        let work = cache
            .update(
                module_of.clone(),
                edges.clone(),
                incremental::Options {
                    entries: entries.clone(),
                    barrels: barrels.clone(),
                    tests: tests.clone(),
                    directory_units: source.language == model::Language::Go,
                    all_kinds: config.all_kinds,
                },
            )
            .expect("validated analysis endpoints");
        if let Some(reason) = work.fallback {
            eprintln!("daemon: full analysis fallback: {reason}");
        }
        (
            cache.cycles().to_vec(),
            cache.deferred_cycles().to_vec(),
            cache.modules().to_vec(),
        )
    } else {
        let (cycles, deferred) = quality::cycles(
            &module_of,
            edges
                .iter()
                .filter(|e| !tests.contains(&e.from) && !tests.contains(&e.to)),
            &entries,
            &barrels,
            source.language == model::Language::Go,
            config.all_kinds,
        );
        let mut metrics = graph::modules(&module_of, pairs.keys());
        quality::metrics(&mut metrics, &module_of, &pairs);
        (cycles, deferred, metrics)
    };
    for module in &mut metrics {
        if !config.legacy_module_classes
            && module_of
                .iter()
                .any(|(path, id)| id == &module.id && tests.contains(path))
        {
            module.class = Some(model::ModuleClass::Tests);
        }
    }
    let test_coverage = (config.include_tests || !tests.is_empty()).then(|| {
        let production: BTreeSet<_> = module_of
            .iter()
            .filter(|(path, _)| !tests.contains(*path))
            .map(|(_, id)| id.clone())
            .collect();
        let covered: BTreeSet<_> = edges
            .iter()
            .filter(|e| tests.contains(&e.from) && !tests.contains(&e.to))
            .map(|e| module_of[&e.to].clone())
            .collect();
        model::TestCoverage {
            modules: production.len(),
            covered: covered.len(),
            without_importing_tests: production.difference(&covered).cloned().collect(),
        }
    });
    let mut doc = Document {
        schema: SCHEMA,
        kind: "scan",
        tool: Tool {
            name: "archgraph",
            version: env!("CARGO_PKG_VERSION"),
        },
        modules: metrics,
        cycles,
        type_only_cycles,
        test_coverage,
        imports: config.import_records.clone(),
        globals: config.globals.clone(),
        import_resolution: None,
        unresolved: Vec::new(),
        external_nodes,
        external_edges,
        external_exposure: None,
        grabbags: Vec::new(),
        files: module_of
            .iter()
            .map(|(path, module)| File {
                path: path.clone(),
                module: module.clone(),
                barrel: barrels.contains(path),
                class: (!config.legacy_module_classes && tests.contains(path))
                    .then_some(model::ModuleClass::Tests),
                owners: owners.map(|o| o.owners(path)).unwrap_or_default(),
            })
            .collect(),
        source,
        config,
        edges,
        package_entries,
        module_edges,
        summary: Summary::default(),
        findings: Vec::new(),
        diagnostics: Vec::new(),
        cochange: None,
        hotspots: None,
        symbols: None,
        surface: None,
        package_splits: None,
        target: None,
        stale_known: Vec::new(),
    };
    known::mark(&mut doc);
    if owners.is_some() {
        for module in &mut doc.modules {
            let mut ownership = model::ModuleOwnership {
                owned_files: 0,
                owners: BTreeMap::new(),
                primary: Vec::new(),
            };
            for file in doc.files.iter().filter(|f| f.module == module.id) {
                ownership.owned_files += usize::from(!file.owners.is_empty());
                for owner in &file.owners {
                    *ownership.owners.entry(owner.clone()).or_default() += 1;
                }
            }
            ownership.primary = ownership
                .owners
                .iter()
                .filter(|(_, n)| **n > module.files / 2)
                .map(|(owner, _)| owner.clone())
                .collect();
            module.ownership = Some(ownership);
        }
    }
    resolution::attach(&mut doc);
    grabbag::classify(&mut doc);
    doc.summary = simulate::summary(&doc);
    doc
}
