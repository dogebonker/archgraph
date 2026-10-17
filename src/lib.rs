//! Library half of the `archgraph` binary, shared with the integration tests. Not a published API.

pub mod bisect;
pub mod cochange;
pub mod codeowners;
pub mod collect;
pub mod diff;
pub mod findings;
pub mod graph;
pub mod history;
pub mod hotspots;
pub mod impact;
pub mod model;
pub mod modules;
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
pub fn restore_scan(input: model::ScanInput) -> Result<Document> {
    validate_scan(&input)?;
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
    if input.schema != SCHEMA || input.kind != "scan" {
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
    package_entries.sort();
    package_entries.dedup();
    edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    let module_of: BTreeMap<String, String> = files
        .into_iter()
        .map(|f| {
            let module = rules
                .and_then(|r| r.module_of(&f))
                .unwrap_or_else(|| modules::module_of(&f, &config, source.language));
            (f, module)
        })
        .collect();
    match rules {
        Some(rules) => rules.decide(&mut edges, &module_of),
        None => {
            for e in &mut edges {
                (e.verdict, e.rule, e.reach) = (None, None, None);
            }
        }
    }
    let barrels = modules::barrels(&edges);
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
    let mut doc = Document {
        schema: SCHEMA,
        kind: "scan",
        tool: Tool {
            name: "archgraph",
            version: env!("CARGO_PKG_VERSION"),
        },
        modules: graph::modules(&module_of, &pairs),
        cycles: graph::cycles(
            &module_of,
            &edges,
            &entries,
            &barrels,
            source.language == model::Language::Go,
        ),
        files: module_of
            .iter()
            .map(|(path, module)| File {
                path: path.clone(),
                module: module.clone(),
                barrel: barrels.contains(path),
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
