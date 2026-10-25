//! Declared service calls: independent of source-import edge semantics and collectors.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::SCHEMA;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: usize,
    pub services: Vec<String>,
    pub edges: Vec<Call>,
    #[serde(default)]
    pub forbid: Vec<Rule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Grpc,
    Queue,
}

impl Protocol {
    fn label(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Grpc => "grpc",
            Self::Queue => "queue",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    #[default]
    Low,
    Medium,
    High,
}

impl Confidence {
    fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub from: String,
    pub to: String,
    pub kind: Protocol,
    #[serde(default)]
    pub confidence: Confidence,
    pub source: SourceLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<Protocol>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: String,
    pub kind: String,
    pub source: Source,
    pub services: Vec<String>,
    pub edges: Vec<Edge>,
    pub forbid: Vec<Rule>,
    pub modules: Vec<Module>,
    pub cycles: Vec<Cycle>,
    pub hubs: Vec<Hub>,
    pub forbidden_calls: Vec<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Resource>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forbidden_resources: Vec<usize>,
    pub summary: Summary,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    pub service: String,
    pub kind: ResourceKind,
    pub package: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    pub confidence: Confidence,
    pub source: SourceLocation,
    pub verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    Database,
    Cache,
    Queue,
    Http,
}

impl ResourceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Cache => "cache",
            Self::Queue => "queue",
            Self::Http => "http",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub collector: String,
    pub path: String,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    #[serde(flatten)]
    pub call: Call,
    pub verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Allowed,
    Forbidden,
}

#[derive(Debug, Clone, Serialize)]
pub struct Module {
    pub id: String,
    pub fan_in: usize,
    pub fan_out: usize,
    pub instability: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cycle {
    pub members: Vec<String>,
    pub edge_indices: Vec<usize>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hub {
    pub service: String,
    pub fan_in: usize,
    pub callers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub services: usize,
    pub calls: usize,
    pub cycles: usize,
    pub largest_cycle: usize,
    pub hubs: usize,
    pub forbidden_calls: usize,
    pub low_confidence_calls: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub resources: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub forbidden_resources: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// YAML and JSON share the same strict manifest shape.
pub fn parse(text: &str) -> Result<Manifest> {
    let manifest: Manifest = serde_yaml_ng::from_str(text).context("parse service manifest")?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(manifest: &Manifest) -> Result<()> {
    if manifest.version != 1 {
        bail!(
            "unsupported services manifest version {}; expected 1",
            manifest.version
        );
    }
    let mut services = BTreeSet::new();
    for id in &manifest.services {
        if !crate::collect::is_relative_path(id, false)
            || id.chars().any(char::is_whitespace)
            || id.contains(['*', '?', '[', ']', '{', '}', '<', '>'])
        {
            bail!(
                "invalid service ID {id:?}: use a relative name without whitespace or glob characters"
            );
        }
        if !services.insert(id) {
            bail!("duplicate service ID {id:?}");
        }
    }
    let mut calls = BTreeSet::new();
    for (index, edge) in manifest.edges.iter().enumerate() {
        if !services.contains(&edge.from) || !services.contains(&edge.to) {
            bail!(
                "call {index} references an unknown service: {:?} -> {:?}",
                edge.from,
                edge.to
            );
        }
        if !crate::collect::is_relative_path(&edge.source.file, false) || edge.source.line == 0 {
            bail!("call {index} requires a relative source file and a positive source line");
        }
        if edge
            .operation
            .as_ref()
            .is_some_and(|operation| operation.chars().any(char::is_control))
        {
            bail!("call {index} operation must contain no control characters");
        }
        if !calls.insert(edge) {
            bail!("duplicate call at index {index}");
        }
    }
    let mut ids = BTreeSet::new();
    for rule in &manifest.forbid {
        if rule.id.trim().is_empty() || rule.id.chars().any(char::is_control) {
            bail!("forbid rule IDs must be nonempty and contain no control characters");
        }
        if !ids.insert(&rule.id) {
            bail!("duplicate forbid rule ID {:?}", rule.id);
        }
        if rule.kinds.as_ref().is_some_and(Vec::is_empty) {
            bail!(
                "forbid rule {:?} has empty kinds; omit kinds to match all protocols",
                rule.id
            );
        }
        if rule.from.chars().any(char::is_control)
            || rule.to.chars().any(char::is_control)
            || rule
                .reason
                .as_ref()
                .is_some_and(|reason| reason.chars().any(char::is_control))
        {
            bail!(
                "forbid rule {:?} must contain no control characters",
                rule.id
            );
        }
        if rule.from.trim().is_empty() || rule.to.trim().is_empty() {
            bail!(
                "forbid rule {:?} requires nonempty from and to globs",
                rule.id
            );
        }
        if rule.from.contains(['<', '>']) || rule.to.contains(['<', '>']) {
            bail!(
                "forbid rule {:?} does not support capture patterns; use service globs",
                rule.id
            );
        }
        crate::collect::globset(&[rule.from.clone(), rule.to.clone()])
            .with_context(|| format!("forbid rule {:?}", rule.id))?;
    }
    Ok(())
}

/// Analyze exactly the declared calls; no code, configuration or evidence files are executed.
pub fn analyze(manifest: Manifest, path: &str) -> Result<Report> {
    validate(&manifest)?;
    if path.is_empty() || path.chars().any(char::is_control) {
        bail!("manifest path must be nonempty and contain no control characters");
    }
    let mut services = manifest.services;
    services.sort();
    let indices: BTreeMap<_, _> = services
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let matchers = manifest
        .forbid
        .iter()
        .map(|rule| {
            Ok((
                crate::collect::globset(std::slice::from_ref(&rule.from))?,
                crate::collect::globset(std::slice::from_ref(&rule.to))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut incoming = vec![BTreeSet::new(); services.len()];
    let mut outgoing = vec![BTreeSet::new(); services.len()];
    let mut forbidden_calls = Vec::new();
    let edges: Vec<_> = manifest
        .edges
        .into_iter()
        .enumerate()
        .map(|(index, call)| {
            let from = indices[call.from.as_str()];
            let to = indices[call.to.as_str()];
            if from != to {
                incoming[to].insert(from);
                outgoing[from].insert(to);
            }
            let rule = manifest
                .forbid
                .iter()
                .zip(&matchers)
                .find(|(rule, (from, to))| {
                    from.is_match(&call.from)
                        && to.is_match(&call.to)
                        && rule
                            .kinds
                            .as_ref()
                            .is_none_or(|kinds| kinds.contains(&call.kind))
                })
                .map(|(rule, _)| rule.id.clone());
            let verdict = if rule.is_some() {
                forbidden_calls.push(index);
                Verdict::Forbidden
            } else {
                Verdict::Allowed
            };
            Edge {
                call,
                verdict,
                rule,
            }
        })
        .collect();
    let adjacency: Vec<Vec<_>> = outgoing
        .iter()
        .map(|neighbors| neighbors.iter().copied().collect())
        .collect();
    let mut cycles: Vec<_> = crate::graph::sccs(&adjacency)
        .into_iter()
        .filter(|members| members.len() > 1)
        .map(|members| {
            let members: BTreeSet<_> = members.into_iter().map(|i| services[i].clone()).collect();
            let edge_indices: Vec<_> = edges
                .iter()
                .enumerate()
                .filter(|(_, edge)| {
                    edge.call.from != edge.call.to
                        && members.contains(&edge.call.from)
                        && members.contains(&edge.call.to)
                })
                .map(|(index, _)| index)
                .collect();
            let confidence = edge_indices
                .iter()
                .map(|&i| edges[i].call.confidence)
                .min()
                .unwrap();
            Cycle {
                members: members.into_iter().collect(),
                edge_indices,
                confidence,
            }
        })
        .collect();
    cycles.sort_by(|a, b| a.members.cmp(&b.members));
    let modules = services
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let fan_in = incoming[i].len();
            let fan_out = outgoing[i].len();
            let total = fan_in + fan_out;
            Module {
                id: id.clone(),
                fan_in,
                fan_out,
                instability: (total > 0)
                    .then(|| crate::quality::round4(fan_out as f64 / total as f64)),
            }
        })
        .collect();
    let mut hubs: Vec<_> = services
        .iter()
        .enumerate()
        .filter(|(i, _)| incoming[*i].len() >= 2)
        .map(|(i, service)| Hub {
            service: service.clone(),
            fan_in: incoming[i].len(),
            callers: incoming[i].iter().map(|&j| services[j].clone()).collect(),
        })
        .collect();
    hubs.sort_by(|a, b| {
        b.fan_in
            .cmp(&a.fan_in)
            .then_with(|| a.service.cmp(&b.service))
    });
    let summary = Summary {
        services: services.len(),
        calls: edges.len(),
        cycles: cycles.len(),
        largest_cycle: cycles
            .iter()
            .map(|cycle| cycle.members.len())
            .max()
            .unwrap_or(0),
        hubs: hubs.len(),
        forbidden_calls: forbidden_calls.len(),
        low_confidence_calls: edges
            .iter()
            .filter(|edge| edge.call.confidence == Confidence::Low)
            .count(),
        resources: 0,
        forbidden_resources: 0,
    };
    Ok(Report {
        schema: SCHEMA.to_owned(), kind: "services".to_owned(),
        source: Source { collector: "declarative".to_owned(), path: path.to_owned(), complete: false },
        services, edges, forbid: manifest.forbid, modules, cycles, hubs, forbidden_calls, summary,
        resources: Vec::new(), forbidden_resources: Vec::new(),
        caveats: vec![
            "Only declared calls are analyzed; this graph does not claim completeness.".to_owned(),
            "Source locations and confidence are supplied by the manifest and are not verified.".to_owned(),
            "A service cycle is a review signal, not proof of deadlock or deploy-order failure; queue calls may be asynchronous.".to_owned(),
        ],
    })
}

fn escape(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '#' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn attach_resources(report: &mut Report, mut resources: Vec<Resource>) -> Result<()> {
    let rules = report
        .forbid
        .iter()
        .filter_map(|rule| rule.to.strip_prefix("pkg:").map(|package| (rule, package)))
        .map(|(rule, package)| {
            Ok((
                rule,
                crate::collect::globset(std::slice::from_ref(&rule.from))?,
                crate::collect::globset(&[package.to_owned()])?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    for (index, resource) in resources.iter_mut().enumerate() {
        resource.rule = rules
            .iter()
            .find(|(rule, from, package)| {
                from.is_match(&resource.service)
                    && package.is_match(&resource.package)
                    && rule.kinds.as_ref().is_none_or(|kinds| {
                        let protocol = match resource.kind {
                            ResourceKind::Http => Some(Protocol::Http),
                            ResourceKind::Queue => Some(Protocol::Queue),
                            ResourceKind::Database | ResourceKind::Cache => None,
                        };
                        protocol.is_some_and(|protocol| kinds.contains(&protocol))
                    })
            })
            .map(|(rule, _, _)| rule.id.clone());
        if resource.rule.is_some() {
            resource.verdict = Verdict::Forbidden;
            report.forbidden_resources.push(index);
        }
    }
    report.summary.resources = resources.len();
    report.summary.forbidden_resources = report.forbidden_resources.len();
    report.resources = resources;
    Ok(())
}

pub fn markdown(report: &Report) -> String {
    let mut out = String::new();
    let inferred = report.source.collector != "declarative";
    let mode = if inferred { "Inferred" } else { "Declared" };
    writeln!(out, "# {mode} service architecture\n").unwrap();
    let count = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
    writeln!(
        out,
        "{}, {}, {} (largest: {} services), {}, {}.\n",
        count(report.summary.services, "service"),
        count(
            report.summary.calls,
            if inferred { "call" } else { "declared call" }
        ),
        count(report.summary.cycles, "cycle"),
        report.summary.largest_cycle,
        count(report.summary.hubs, "hub"),
        count(report.summary.forbidden_calls, "forbidden call")
    )
    .unwrap();
    for caveat in &report.caveats {
        writeln!(out, "> {}\n", escape(caveat)).unwrap();
    }
    if inferred {
        writeln!(out, "## Infrastructure\n").unwrap();
        if report.resources.is_empty() {
            writeln!(out, "No infrastructure clients inferred.\n").unwrap();
        }
        for resource in &report.resources {
            writeln!(
                out,
                "- {}: {} via {}{}; {} confidence; {}:{}; {}.",
                escape(&resource.service),
                resource.kind.label(),
                escape(&resource.package),
                resource
                    .operation
                    .as_ref()
                    .map(|op| format!(" ({})", escape(op)))
                    .unwrap_or_default(),
                resource.confidence.label(),
                escape(&resource.source.file),
                resource.source.line,
                resource
                    .rule
                    .as_ref()
                    .map(|rule| format!("forbidden by {}", escape(rule)))
                    .unwrap_or_else(|| "allowed".into())
            )
            .unwrap();
        }
        writeln!(
            out,
            "\n{} resources, {} forbidden resources.\n",
            report.summary.resources, report.summary.forbidden_resources
        )
        .unwrap();
    }
    writeln!(out, "## Cycles\n").unwrap();
    if report.cycles.is_empty() {
        writeln!(
            out,
            "No cross-service cycles among {}calls.\n",
            if inferred { "" } else { "declared " }
        )
        .unwrap();
    }
    for cycle in &report.cycles {
        writeln!(
            out,
            "- {} — {} confidence; calls {}.",
            cycle
                .members
                .iter()
                .map(|id| escape(id))
                .collect::<Vec<_>>()
                .join(", "),
            cycle.confidence.label(),
            cycle
                .edge_indices
                .iter()
                .map(|i| format!("#{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    writeln!(out, "\n## Hubs\n").unwrap();
    if report.hubs.is_empty() {
        writeln!(out, "No service has two or more distinct other callers.\n").unwrap();
    }
    for hub in &report.hubs {
        writeln!(
            out,
            "- {}: called by {} services — {}.",
            escape(&hub.service),
            hub.fan_in,
            hub.callers
                .iter()
                .map(|id| escape(id))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
    }
    writeln!(out, "\n## Services\n\n| Service | Fan-in | Fan-out | Instability |\n| --- | ---: | ---: | ---: |").unwrap();
    for module in &report.modules {
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            escape(&module.id),
            module.fan_in,
            module.fan_out,
            module
                .instability
                .map(|v| format!("{v:.4}"))
                .unwrap_or_else(|| "n/a".to_owned())
        )
        .unwrap();
    }
    writeln!(out, "\n## {mode} calls\n\nCall numbers below are one-based; JSON edge indices are zero-based. Allowed means no declared forbid rule matched.\n").unwrap();
    if report.edges.is_empty() {
        writeln!(
            out,
            "No calls {}.\n",
            if inferred { "inferred" } else { "declared" }
        )
        .unwrap();
    }
    for (index, edge) in report.edges.iter().enumerate() {
        let call = &edge.call;
        let policy = match &edge.rule {
            Some(id) => {
                let reason = report
                    .forbid
                    .iter()
                    .find(|rule| &rule.id == id)
                    .and_then(|rule| rule.reason.as_ref());
                format!(
                    "forbidden by {}{}",
                    escape(id),
                    reason
                        .map(|r| format!(" ({})", escape(r)))
                        .unwrap_or_default()
                )
            }
            None => "allowed".to_owned(),
        };
        writeln!(
            out,
            "- #{} {} → {} — {}; {} confidence; {}:{}{}; {}.",
            index + 1,
            escape(&call.from),
            escape(&call.to),
            call.kind.label(),
            call.confidence.label(),
            escape(&call.source.file),
            call.source.line,
            call.operation
                .as_ref()
                .map(|op| format!("; {}", escape(op)))
                .unwrap_or_default(),
            policy
        )
        .unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_calls_remain_evidence_but_are_not_cross_service_cycles() {
        let manifest = parse(r#"{"version":1,"services":["a"],"edges":[{"from":"a","to":"a","kind":"queue","source":{"file":"config/topics.yaml","line":2}}]}"#).unwrap();
        let report = analyze(manifest, "services.yaml").unwrap();
        assert_eq!(report.edges.len(), 1);
        assert!(report.cycles.is_empty());
        assert_eq!(report.modules[0].fan_in, 0);
        assert_eq!(report.modules[0].fan_out, 0);
        assert_eq!(report.modules[0].instability, None);
        assert!(markdown(&report).contains("1 service, 1 declared call, 0 cycles"));
        let empty = analyze(
            parse(r#"{"version":1,"services":[],"edges":[]}"#).unwrap(),
            "services.yaml",
        )
        .unwrap();
        assert!(empty.modules.is_empty());
        assert_eq!(empty.summary.largest_cycle, 0);
    }

    #[test]
    fn service_globs_keep_single_stars_within_namespace_segments() {
        let manifest = parse(r#"{
            "version":1,"services":["frontend","team/api","team/deep/api"],
            "edges":[
                {"from":"frontend","to":"team/api","kind":"http","source":{"file":"calls.ts","line":1}},
                {"from":"frontend","to":"team/deep/api","kind":"http","source":{"file":"calls.ts","line":2}}
            ],"forbid":[{"id":"one-segment","from":"frontend","to":"team/*"}]
        }"#).unwrap();
        let report = analyze(manifest, "services.yaml").unwrap();
        assert_eq!(report.forbidden_calls, [0]);
        assert_eq!(report.edges[1].verdict, Verdict::Allowed);
    }

    #[test]
    fn direct_analysis_validates_and_markdown_escapes_evidence() {
        let mut manifest = parse(r#"{"version":1,"services":["a"],"edges":[]}"#).unwrap();
        manifest.version = 2;
        assert!(analyze(manifest, "services.yaml").is_err());
        assert_eq!(escape("<script>\n[x]|`"), "&lt;script&gt; \\[x\\]\\|\\`");
    }
}
