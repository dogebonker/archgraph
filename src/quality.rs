//! Kind-aware metrics and accounting around the unchanged graph algorithms.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph;
use crate::model::{Cycle, CycleLevel, DeferredCycle, Edge, EdgeKind, Module};

/// Preserve raw import counts while weighting actual dependencies rather than barrel wiring.
pub fn metrics(
    modules: &mut [Module],
    module_of: &BTreeMap<String, String>,
    edges: &[Edge],
    barrels: &BTreeSet<String>,
) {
    let mut targets: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in edges.iter().filter(|e| barrels.contains(&e.from)) {
        targets.entry(&edge.from).or_default().push(&edge.to);
    }
    let mut pairs: BTreeMap<(&str, &str), f64> = BTreeMap::new();
    for edge in edges.iter().filter(|e| !barrels.contains(&e.from)) {
        let weight = edge
            .weight
            .unwrap_or_else(|| crate::rules::EdgeWeights::default().weight(edge.kind));
        let mut stack = vec![edge.to.as_str()];
        let mut seen = BTreeSet::new();
        while let Some(file) = stack.pop() {
            if !seen.insert(file) {
                continue;
            }
            match targets.get(file) {
                Some(next) => stack.extend(next),
                None if file != edge.from => {
                    let pair = pairs.entry((&edge.from, file)).or_default();
                    *pair = pair.max(weight);
                }
                None => {}
            }
        }
    }
    let mut internal: BTreeMap<&str, f64> = BTreeMap::new();
    let mut outgoing: BTreeMap<&str, BTreeMap<String, f64>> = BTreeMap::new();
    let mut neighbours: BTreeMap<(&str, &str), f64> = BTreeMap::new();
    for ((from, to), weight) in pairs {
        let (Some(from), Some(to)) = (module_of.get(from), module_of.get(to)) else {
            continue;
        };
        if from == to {
            *internal.entry(from).or_default() += weight;
        } else {
            *outgoing
                .entry(from)
                .or_default()
                .entry(to.clone())
                .or_default() += weight;
            let neighbour = neighbours.entry((from, to)).or_default();
            *neighbour = neighbour.max(weight);
        }
    }
    let mut ca: BTreeMap<&str, f64> = BTreeMap::new();
    let mut ce: BTreeMap<&str, f64> = BTreeMap::new();
    for ((from, to), weight) in neighbours {
        *ca.entry(to).or_default() += weight;
        *ce.entry(from).or_default() += weight;
    }
    for module in modules {
        module.weighted_ca = ca.get(module.id.as_str()).copied().unwrap_or_default();
        module.weighted_ce = ce.get(module.id.as_str()).copied().unwrap_or_default();
        module.weighted_internal_edges = internal
            .get(module.id.as_str())
            .copied()
            .unwrap_or_default();
        module.weighted_outgoing_to = outgoing.remove(module.id.as_str()).unwrap_or_default();
        module.weighted_outgoing_edges = module.weighted_outgoing_to.values().sum();
        module.instability = ratio(module.weighted_ce, module.weighted_ca + module.weighted_ce);
        module.cohesion = ratio(
            module.weighted_internal_edges,
            module.weighted_internal_edges + module.weighted_outgoing_edges,
        );
        module.weighted_ca = rounded(module.weighted_ca);
        module.weighted_ce = rounded(module.weighted_ce);
        module.weighted_internal_edges = rounded(module.weighted_internal_edges);
        module.weighted_outgoing_edges = rounded(module.weighted_outgoing_edges);
        for weight in module.weighted_outgoing_to.values_mut() {
            *weight = rounded(*weight);
        }
    }
}

fn rounded(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        (value * 10_000.0).round() / 10_000.0
    }
}

fn ratio(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| (part / whole * 10_000.0).round() / 10_000.0)
}

/// Use ordinary graph findings for the selected kinds; account for deferred SCCs cheaply.
pub fn cycles(
    module_of: &BTreeMap<String, String>,
    edges: &[Edge],
    entries: &BTreeSet<String>,
    barrels: &BTreeSet<String>,
    directory_units: bool,
    all_kinds: bool,
) -> (Vec<Cycle>, Vec<DeferredCycle>) {
    let runtime = graph::cycles(module_of, edges, entries, barrels, directory_units);
    let mut deferred = Vec::new();
    for level in [CycleLevel::Module, CycleLevel::File] {
        let pairs: Vec<_> = edges
            .iter()
            .filter_map(|edge| {
                let from = module_of.get(&edge.from)?;
                let to = module_of.get(&edge.to)?;
                match level {
                    CycleLevel::Module if from != to => Some((from.as_str(), to.as_str(), edge)),
                    CycleLevel::File
                        if from == to
                            && edge.from != edge.to
                            && (!directory_units
                                || directory(&edge.from) != directory(&edge.to)) =>
                    {
                        Some((edge.from.as_str(), edge.to.as_str(), edge))
                    }
                    _ => None,
                }
            })
            .collect();
        for members in components(&pairs) {
            if runtime.iter().any(|cycle| {
                cycle.level == level
                    && cycle
                        .members
                        .iter()
                        .map(String::as_str)
                        .eq(members.iter().copied())
            }) {
                continue;
            }
            let kinds: BTreeSet<_> = pairs
                .iter()
                .filter(|(from, to, _)| members.contains(from) && members.contains(to))
                .map(|(_, _, edge)| {
                    match edge.kind {
                        EdgeKind::Static => "runtime",
                        EdgeKind::Dynamic => "lazy",
                        EdgeKind::Type | EdgeKind::TypeReexport => "type",
                        EdgeKind::Reexport => "reexport",
                    }
                    .to_owned()
                })
                .collect();
            let contains_runtime_cycle = runtime.iter().any(|cycle| {
                cycle.level == level
                    && cycle
                        .members
                        .iter()
                        .all(|member| members.contains(member.as_str()))
            });
            deferred.push(DeferredCycle {
                level: level.clone(),
                members: members.into_iter().map(str::to_owned).collect(),
                kinds: kinds.into_iter().collect(),
                contains_runtime_cycle,
            });
        }
    }
    if all_kinds {
        let all: Vec<_> = edges
            .iter()
            .cloned()
            .map(|mut edge| {
                edge.is_reexport |=
                    matches!(edge.kind, EdgeKind::Reexport | EdgeKind::TypeReexport);
                edge.kind = EdgeKind::Static;
                edge
            })
            .collect();
        (
            graph::cycles(module_of, &all, entries, barrels, directory_units),
            deferred,
        )
    } else {
        (runtime, deferred)
    }
}

fn directory(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(directory, _)| directory)
}

pub(crate) fn components<'a>(pairs: &[(&'a str, &'a str, &Edge)]) -> Vec<BTreeSet<&'a str>> {
    let files: BTreeSet<_> = pairs
        .iter()
        .flat_map(|(from, to, _)| [*from, *to])
        .collect();
    let indexes: BTreeMap<_, _> = files
        .iter()
        .enumerate()
        .map(|(i, file)| (*file, i))
        .collect();
    let files: Vec<_> = files.into_iter().collect();
    let mut adjacency = vec![Vec::new(); files.len()];
    for (from, to, _) in pairs {
        adjacency[indexes[from]].push(indexes[to]);
    }
    graph::sccs(&adjacency)
        .into_iter()
        .filter(|component| {
            component.len() > 1
                || component
                    .first()
                    .is_some_and(|&i| pairs.iter().any(|(a, b, _)| a == b && *a == files[i]))
        })
        .map(|component| component.into_iter().map(|i| files[i]).collect())
        .collect()
}
