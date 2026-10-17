//! Library half of the `archgraph` binary, shared with the integration tests. Not a published API.

pub mod bisect;
pub mod cochange;
pub mod codeowners;
pub mod collect;
pub mod compat;
pub mod diff;
pub mod findings;
pub mod graph;
pub mod history;
pub mod hotspots;
pub mod impact;
pub mod model;
pub mod modules;
pub mod quality;
pub mod render;
pub mod rules;
pub mod simulate;
pub mod surface;
pub mod symbols;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use model::{Cochange, Config, Document, Edge, File, SCHEMA, Source, Summary, Tool};
use rules::Compiled;

/// Rebuild a persisted scan, retaining its optional historical context.
pub fn restore_scan(mut input: model::ScanInput) -> Result<Document> {
    validate_scan(&input)?;
    input.config.file_metadata = input
        .files
        .iter()
        .map(|f| {
            (
                f.path.clone(),
                model::NodeMetadata {
                    barrel: f.barrel,
                    test: f.class == Some(crate::model::ModuleClass::Tests),
                },
            )
        })
        .collect();
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
    for saved in input.findings {
        if let Some(finding) = doc.findings.iter_mut().find(|f| {
            f.subject == saved.subject
                && serde_json::to_value(f.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .as_deref()
                    == Some(saved.kind.as_str())
        }) {
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
    let mut paths = BTreeSet::new();
    for file in &input.files {
        if !paths.insert(file.path.as_str()) {
            bail!("{} is listed twice", file.path);
        }
        let module = rules
            .as_ref()
            .and_then(|r| r.module_of(&file.path))
            .unwrap_or_else(|| {
                modules::module_of(&file.path, &input.config, input.source.language)
            });
        let module = if file.class == Some(model::ModuleClass::Tests) {
            format!("tests::{module}")
        } else {
            module
        };
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
    Ok(())
}

pub fn apply_hotspots(doc: &mut Document, mut report: hotspots::Hotspots) {
    hotspots::attach_findings(&mut report, doc);
    let priorities = hotspots::finding_priorities(&report, doc.findings.len());
    for (finding, priority) in doc.findings.iter_mut().zip(priorities) {
        finding.hotspot_priority = Some(priority);
    }
    doc.findings.sort_by(|a, b| {
        b.hotspot_priority
            .unwrap_or(0.0)
            .total_cmp(&a.hotspot_priority.unwrap_or(0.0))
    });
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
    let rules = rules::compile(&config)?;
    let owners = config
        .codeowners
        .as_ref()
        .map(codeowners::compile)
        .transpose()?;
    let mut doc = build(
        source,
        config,
        files,
        edges,
        package_entries,
        rules.as_ref(),
        owners.as_ref(),
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
    findings::place_registrars(&mut doc, rules.as_ref(), owners.as_ref());
    doc.cochange = cochange;
    doc.findings = findings::find(&doc, rules.as_ref(), owners.as_ref());
    Ok(doc)
}

/// `analyze` without findings, with the rules already compiled from `config.rules`.
pub fn build(
    source: Source,
    config: Config,
    files: Vec<String>,
    mut edges: Vec<Edge>,
    mut package_entries: Vec<String>,
    rules: Option<&Compiled>,
    owners: Option<&codeowners::Compiled>,
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
        .filter(|path| {
            config.file_metadata.get(*path).is_some_and(|m| m.test)
                || collect::excludes::is_test(path)
        })
        .cloned()
        .collect();
    let module_of: BTreeMap<String, String> = files
        .into_iter()
        .map(|f| {
            let mut module = rules
                .and_then(|r| r.module_of(&f))
                .unwrap_or_else(|| modules::module_of(&f, &config, source.language));
            if !config.legacy_module_classes && tests.contains(&f) {
                module = format!("tests::{module}");
            }
            (f, module)
        })
        .collect();
    match rules {
        Some(rules) => {
            let mut policy_modules = module_of.clone();
            for path in edges.iter().filter_map(|e| e.original_to.as_ref()) {
                policy_modules.entry(path.clone()).or_insert_with(|| {
                    rules
                        .module_of(path)
                        .unwrap_or_else(|| modules::module_of(path, &config, source.language))
                });
            }
            rules.decide(&mut edges, &policy_modules);
        }
        None => {
            for e in &mut edges {
                (e.verdict, e.rule, e.reach) = (None, None, None);
            }
        }
    }
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
    let pairs = modules::attribute(&edges, &barrels);
    // Public entries for cycle cuts: what a package.json publishes and the source root's index
    // (`index.ts`, a Python package's `__init__.py`). Other barrels are internal.
    let root_index = |f: &str| {
        let name = match config.source_root.as_str() {
            "." => Some(f),
            root => f.strip_prefix(root).and_then(|r| r.strip_prefix('/')),
        };
        name.is_some_and(|n| n == "__init__.py" || modules::TS_INDEX.contains(&n))
    };
    let entries: BTreeSet<String> = module_of
        .keys()
        .filter(|f| root_index(f))
        .chain(&package_entries)
        .cloned()
        .collect();
    let module_edges = graph::module_edges(&module_of, &edges);
    let production_edges: Vec<_> = edges
        .iter()
        .filter(|e| !tests.contains(&e.from) && !tests.contains(&e.to))
        .cloned()
        .collect();
    let (cycles, type_only_cycles) = quality::cycles(
        &module_of,
        &production_edges,
        &entries,
        &barrels,
        source.language == model::Language::Go,
        config.all_kinds,
    );
    let mut metrics = graph::modules(&module_of, &pairs);
    quality::metrics(&mut metrics, &module_of, &edges, &barrels);
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
        cochange: None,
        hotspots: None,
        symbols: None,
        surface: None,
    };
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
    doc.summary = simulate::summary(&doc);
    doc
}
