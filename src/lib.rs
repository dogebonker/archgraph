//! Library half of the `archgraph` binary, shared with the integration tests. Not a published API.

pub mod cochange;
pub mod collect;
pub mod diff;
pub mod findings;
pub mod graph;
pub mod model;
pub mod modules;
pub mod render;
pub mod rules;
pub mod simulate;

use std::collections::BTreeMap;

use anyhow::Result;
use model::{Cochange, Config, Document, Edge, File, SCHEMA, Source, Summary, Tool};
use rules::Compiled;

/// The one pipeline every subcommand calls: module assignment, barrels, verdicts under
/// `config.rules`, metrics, cycles and findings from collected files and edges (and git
/// co-changes, when read). Every edge must join two of `files`.
pub fn analyze(
    source: Source,
    config: Config,
    files: Vec<String>,
    edges: Vec<Edge>,
    cochange: Option<Cochange>,
) -> Result<Document> {
    let rules = rules::compile(&config)?;
    let mut doc = build(source, config, files, edges, rules.as_ref());
    doc.cochange = cochange;
    doc.findings = findings::find(&doc, rules.as_ref());
    Ok(doc)
}

/// `analyze` without findings, with the rules already compiled from `config.rules`.
pub fn build(
    source: Source,
    config: Config,
    files: Vec<String>,
    mut edges: Vec<Edge>,
    rules: Option<&Compiled>,
) -> Document {
    edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    let module_of: BTreeMap<String, String> = files
        .into_iter()
        .map(|f| {
            let module = rules
                .and_then(|r| r.module_of(&f))
                .unwrap_or_else(|| modules::module_of(&f, &config.source_root, config.depth));
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
    let module_edges = graph::module_edges(&module_of, &edges);
    let mut doc = Document {
        schema: SCHEMA,
        kind: "scan",
        tool: Tool {
            name: "archgraph",
            version: env!("CARGO_PKG_VERSION"),
        },
        modules: graph::modules(&module_of, &pairs),
        cycles: graph::cycles(&module_of, &edges, &module_edges),
        files: module_of
            .iter()
            .map(|(path, module)| File {
                path: path.clone(),
                module: module.clone(),
                barrel: barrels.contains(path),
            })
            .collect(),
        source,
        config,
        edges,
        module_edges,
        summary: Summary::default(),
        findings: Vec::new(),
        cochange: None,
    };
    doc.summary = simulate::summary(&doc);
    doc
}
