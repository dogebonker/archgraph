//! Compare supplied service evidence under the later report's policy.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::Serialize;

use crate::model::SCHEMA;
use crate::services::{
    self, Edge, Manifest, Protocol, Resource, ResourceKind, Rule, Source, Verdict,
};

#[derive(Debug, Serialize)]
pub struct Change<T> {
    pub before: Vec<T>,
    pub after: Vec<T>,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub source: Source,
    pub forbid: Vec<Rule>,
    pub caveats: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub kind: &'static str,
    pub policy_basis: &'static str,
    pub policy_changed: bool,
    pub before: Snapshot,
    pub after: Snapshot,
    pub new_forbidden_calls: Vec<Change<Edge>>,
    pub existing_forbidden_calls: Vec<Change<Edge>>,
    pub resolved_forbidden_calls: Vec<Change<Edge>>,
    pub new_forbidden_resources: Vec<Change<Resource>>,
    pub existing_forbidden_resources: Vec<Change<Resource>>,
    pub resolved_forbidden_resources: Vec<Change<Resource>>,
    pub caveats: Vec<String>,
}

impl Report {
    pub fn has_new_forbidden(&self) -> bool {
        !self.new_forbidden_calls.is_empty() || !self.new_forbidden_resources.is_empty()
    }
}

type CallKey = (String, String, Protocol, Option<String>, String);
type ResourceKey = (String, &'static str, String, Option<String>, String);

struct Classified<T> {
    new: Vec<Change<T>>,
    existing: Vec<Change<T>>,
    resolved: Vec<Change<T>>,
}

fn call_key(edge: &Edge) -> CallKey {
    let call = &edge.call;
    (
        call.from.clone(),
        call.to.clone(),
        call.kind,
        call.operation.clone(),
        call.source.file.clone(),
    )
}

fn resource_kind(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Database => "database",
        ResourceKind::Cache => "cache",
        ResourceKind::Queue => "queue",
        ResourceKind::Http => "http",
    }
}

fn resource_key(resource: &Resource) -> ResourceKey {
    (
        resource.service.clone(),
        resource_kind(resource.kind),
        resource.package.clone(),
        resource.operation.clone(),
        resource.source.file.clone(),
    )
}

fn classify<T: Serialize, K: Ord + Clone>(
    before: impl Iterator<Item = T>,
    after: impl Iterator<Item = T>,
    key: fn(&T) -> K,
) -> Classified<T> {
    let group = |values: Vec<T>| {
        let mut groups: BTreeMap<K, Vec<T>> = BTreeMap::new();
        for value in values {
            groups.entry(key(&value)).or_default().push(value);
        }
        for values in groups.values_mut() {
            values.sort_by_cached_key(|value| {
                serde_json::to_string(value).expect("service evidence serializes")
            });
        }
        groups
    };
    let mut before = group(before.collect());
    let mut after = group(after.collect());
    let keys: BTreeSet<K> = before.keys().chain(after.keys()).cloned().collect();
    let (mut new, mut existing, mut resolved) = (Vec::new(), Vec::new(), Vec::new());
    for key in keys {
        let change = Change {
            before: before.remove(&key).unwrap_or_default(),
            after: after.remove(&key).unwrap_or_default(),
        };
        if change.before.is_empty() {
            new.push(change);
        } else if change.after.is_empty() {
            resolved.push(change);
        } else {
            existing.push(change);
        }
    }
    Classified {
        new,
        existing,
        resolved,
    }
}

fn judge(report: &services::Report, forbid: &[Rule]) -> Result<services::Report> {
    let mut judged = services::analyze(
        Manifest {
            version: 1,
            services: report.services.clone(),
            edges: report.edges.iter().map(|edge| edge.call.clone()).collect(),
            forbid: forbid.to_vec(),
        },
        &report.source.path,
    )?;
    services::attach_resources(&mut judged, report.resources.clone())?;
    Ok(judged)
}

/// Inputs come from `services::read_report`; both sides use the after report's ordered rules.
pub fn diff(before: &services::Report, after: &services::Report) -> Result<Report> {
    if before.source.collector != after.source.collector {
        bail!(
            "service reports use incompatible collector modes; compare reports collected in the same mode"
        );
    }
    let policy_changed =
        serde_json::to_value(&before.forbid)? != serde_json::to_value(&after.forbid)?;
    let old = judge(before, &after.forbid)?;
    let new = judge(after, &after.forbid)?;
    let Classified {
        new: new_forbidden_calls,
        existing: existing_forbidden_calls,
        resolved: resolved_forbidden_calls,
    } = classify(
        old.edges
            .into_iter()
            .filter(|edge| edge.verdict == Verdict::Forbidden),
        new.edges
            .into_iter()
            .filter(|edge| edge.verdict == Verdict::Forbidden),
        call_key,
    );
    let Classified {
        new: new_forbidden_resources,
        existing: existing_forbidden_resources,
        resolved: resolved_forbidden_resources,
    } = classify(
        old.resources
            .into_iter()
            .filter(|resource| resource.verdict == Verdict::Forbidden),
        new.resources
            .into_iter()
            .filter(|resource| resource.verdict == Verdict::Forbidden),
        resource_key,
    );
    let snapshot = |report: &services::Report| Snapshot {
        source: report.source.clone(),
        forbid: report.forbid.clone(),
        caveats: report.caveats.clone(),
    };
    let mut caveats = vec![
        "Both evidence sets are evaluated under after.forbid. Policy changes are reported separately.".into(),
        "Service reports do not record analyzer versions, repository revisions, collection scope or boundary-manifest digests. Collection parity is unavailable.".into(),
        "This compares supplied incomplete evidence. It does not prove that a code edit caused a change or establish complete architectural compliance.".into(),
        "Logical relationship identity excludes source lines and confidence. Repeated occurrences in one source file remain one relationship, with all before/after evidence retained.".into(),
        "Resolved means no longer observed in the supplied after evidence, not proof that a relationship was removed at runtime.".into(),
    ];
    if before.source.path != after.source.path {
        caveats.push("Source labels differ. Report labels are not repository or collection-scope identities.".into());
    }
    Ok(Report {
        schema: SCHEMA,
        kind: "services-diff",
        policy_basis: "after",
        policy_changed,
        before: snapshot(before),
        after: snapshot(after),
        new_forbidden_calls,
        existing_forbidden_calls,
        resolved_forbidden_calls,
        new_forbidden_resources,
        existing_forbidden_resources,
        resolved_forbidden_resources,
        caveats,
    })
}

fn escape(text: &str) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '#' => {
                escaped.push('\\');
                escaped.push(c);
            }
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

pub fn markdown(report: &Report, failed: bool) -> String {
    let mut out = format!(
        "# Service evidence diff: {}\n\nPolicy basis: after.forbid. Policy changed: {}.\n",
        if failed { "fail" } else { "pass" },
        report.policy_changed,
    );
    for (title, changes) in [
        ("New forbidden calls", &report.new_forbidden_calls),
        ("Existing forbidden calls", &report.existing_forbidden_calls),
        (
            "Forbidden calls no longer observed",
            &report.resolved_forbidden_calls,
        ),
    ] {
        out += &format!("\n## {title} ({})\n", changes.len());
        for change in changes {
            for (side, edges) in [("Before", &change.before), ("After", &change.after)] {
                for edge in edges {
                    let call = &edge.call;
                    out += &format!(
                        "\n- {side}: {} -> {} ({:?}{}), {}:{}, rule {}.\n",
                        escape(&call.from),
                        escape(&call.to),
                        call.kind,
                        call.operation
                            .as_ref()
                            .map_or(String::new(), |operation| format!(
                                ", {}",
                                escape(operation)
                            )),
                        escape(&call.source.file),
                        call.source.line,
                        escape(edge.rule.as_deref().unwrap_or_default())
                    );
                }
            }
        }
    }
    for (title, changes) in [
        ("New forbidden resources", &report.new_forbidden_resources),
        (
            "Existing forbidden resources",
            &report.existing_forbidden_resources,
        ),
        (
            "Forbidden resources no longer observed",
            &report.resolved_forbidden_resources,
        ),
    ] {
        out += &format!("\n## {title} ({})\n", changes.len());
        for change in changes {
            for (side, resources) in [("Before", &change.before), ("After", &change.after)] {
                for resource in resources {
                    out += &format!(
                        "\n- {side}: {} uses {} ({}), {}:{}, rule {}.\n",
                        escape(&resource.service),
                        escape(&resource.package),
                        resource_kind(resource.kind),
                        escape(&resource.source.file),
                        resource.source.line,
                        escape(resource.rule.as_deref().unwrap_or_default())
                    );
                }
            }
        }
    }
    out += "\n## Policy reasons\n";
    for rule in &report.after.forbid {
        out += &format!(
            "\n- {}: {}\n",
            escape(&rule.id),
            escape(rule.reason.as_deref().unwrap_or("not supplied"))
        );
    }
    out += "\n## Limits\n";
    for caveat in &report.caveats {
        out += &format!("\n- {}\n", escape(caveat));
    }
    out
}
