//! Kind-aware metrics and accounting around the unchanged graph algorithms.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph;
use crate::model::{Cycle, CycleLevel, DeferredCycle, Edge, EdgeKind, Module};

/// Preserve raw import counts while weighting actual dependencies rather than barrel wiring:
/// `pairs` are `modules::weighted`'s.
pub fn metrics(
    modules: &mut [Module],
    module_of: &BTreeMap<String, String>,
    pairs: &BTreeMap<(String, String), f64>,
) {
    let mut internal: BTreeMap<&str, f64> = BTreeMap::new();
    let mut outgoing: BTreeMap<&str, BTreeMap<String, f64>> = BTreeMap::new();
    let mut neighbours: BTreeMap<(&str, &str), f64> = BTreeMap::new();
    for ((from, to), &weight) in pairs {
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
        module.weighted_ca = round4(module.weighted_ca);
        module.weighted_ce = round4(module.weighted_ce);
        module.weighted_internal_edges = round4(module.weighted_internal_edges);
        module.weighted_outgoing_edges = round4(module.weighted_outgoing_edges);
        for weight in module.weighted_outgoing_to.values_mut() {
            *weight = round4(*weight);
        }
    }
}

/// Rounded to four decimals, as every ratio and weight in the output; zero is never `-0.0`
/// (an empty `f64` sum is).
pub(crate) fn round4(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        (value * 10_000.0).round() / 10_000.0
    }
}

pub(crate) fn ratio(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| round4(part / whole))
}

/// Use ordinary graph findings for the selected kinds; account for deferred SCCs cheaply.
pub fn cycles<'a>(
    module_of: &BTreeMap<String, String>,
    edges: impl IntoIterator<Item = &'a Edge>,
    entries: &BTreeSet<String>,
    barrels: &BTreeSet<String>,
    directory_units: bool,
    all_kinds: bool,
) -> (Vec<Cycle>, Vec<DeferredCycle>) {
    cycles_with_budget(
        module_of,
        edges,
        entries,
        barrels,
        directory_units,
        all_kinds,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cycles_with_budget<'a>(
    module_of: &BTreeMap<String, String>,
    edges: impl IntoIterator<Item = &'a Edge>,
    entries: &BTreeSet<String>,
    barrels: &BTreeSet<String>,
    directory_units: bool,
    all_kinds: bool,
    total_edges: Option<(usize, usize)>,
) -> (Vec<Cycle>, Vec<DeferredCycle>) {
    let edges: Vec<&Edge> = edges.into_iter().collect();
    let runtime = graph::cycles_with_budget(
        module_of,
        edges.iter().copied(),
        entries,
        barrels,
        directory_units,
        total_edges.map(|(runtime, _)| runtime),
    );
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
                                || crate::modules::dir_of(&edge.from)
                                    != crate::modules::dir_of(&edge.to)) =>
                    {
                        Some((edge.from.as_str(), edge.to.as_str(), edge))
                    }
                    _ => None,
                }
            })
            .collect();
        let named: Vec<_> = pairs.iter().map(|&(from, to, _)| (from, to)).collect();
        for members in graph::components(&named) {
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
                .map(|(_, _, edge)| edge.kind.as_str().to_owned())
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
    deferred.sort_by(|a, b| {
        let level = |c: &DeferredCycle| usize::from(c.level == CycleLevel::File);
        (level(a), &a.members).cmp(&(level(b), &b.members))
    });
    if all_kinds {
        // Every kind counts as a runtime import; the clones carry the rewritten kinds.
        let all: Vec<_> = edges
            .iter()
            .map(|&edge| {
                let mut edge = edge.clone();
                edge.is_reexport |=
                    matches!(edge.kind, EdgeKind::Reexport | EdgeKind::TypeReexport);
                edge.kind = EdgeKind::Static;
                edge
            })
            .collect();
        (
            graph::cycles_with_budget(
                module_of,
                &all,
                entries,
                barrels,
                directory_units,
                total_edges.map(|(_, all)| all),
            ),
            deferred,
        )
    } else {
        (runtime, deferred)
    }
}
