//! Verify whether a folder-level cycle survives splitting weakly related helper files.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::model::{CycleLevel, Document, Granularity, Link, ModuleClass};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub cohesion_max: f64,
    pub fan_in_min: f64,
    pub internal_edges_per_file_max: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            cohesion_max: 0.2,
            fan_in_min: 5.0,
            internal_edges_per_file_max: 0.5,
        }
    }
}

impl Thresholds {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.cohesion_max.is_finite() && (0.0..=1.0).contains(&self.cohesion_max),
            "grabbag.cohesion_max must be finite and between 0 and 1"
        );
        ensure!(
            self.fan_in_min.is_finite() && self.fan_in_min >= 0.0,
            "grabbag.fan_in_min must be finite and non-negative"
        );
        ensure!(
            self.internal_edges_per_file_max.is_finite() && self.internal_edges_per_file_max >= 0.0,
            "grabbag.internal_edges_per_file_max must be finite and non-negative"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub module: String,
    pub cohesion: f64,
    pub fan_in: f64,
    pub internal_edges_per_file: f64,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub members: Vec<String>,
    pub candidates: Vec<Candidate>,
    pub edges: Vec<Link>,
    pub remaining_cycles: Vec<Vec<String>>,
}

pub fn classify(doc: &mut Document) {
    doc.grabbags.clear();
    if doc.config.granularity == Granularity::File && !doc.config.legacy_module_classes {
        return;
    }
    let thresholds = doc
        .config
        .rules
        .as_ref()
        .and_then(|r| r.doc.grabbag)
        .unwrap_or_default();
    let candidates: BTreeMap<_, _> =
        doc.modules
            .iter()
            .filter_map(|m| {
                let cohesion = m.cohesion?;
                let per_file = m.internal_edges as f64 / m.files.max(1) as f64;
                (m.class != Some(ModuleClass::Tests)
                    && m.files >= 2
                    && cohesion < thresholds.cohesion_max
                    && m.weighted_ca >= thresholds.fan_in_min
                    && per_file < thresholds.internal_edges_per_file_max
                    && doc.files.iter().any(|f| {
                        f.module == m.id && !f.barrel && f.class != Some(ModuleClass::Tests)
                    }))
                .then(|| {
                    (
                        m.id.clone(),
                        Candidate {
                            module: m.id.clone(),
                            cohesion,
                            fan_in: m.weighted_ca,
                            internal_edges_per_file: per_file,
                            files: doc
                                .files
                                .iter()
                                .filter(|f| f.module == m.id)
                                .map(|f| f.path.clone())
                                .collect(),
                        },
                    )
                })
            })
            .collect();
    if candidates.is_empty() {
        return;
    }
    let module_of: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|f| (f.path.clone(), f.module.clone()))
        .collect();
    let tests: BTreeSet<_> = doc
        .files
        .iter()
        .filter(|f| {
            f.class == Some(ModuleClass::Tests) || crate::modules::is_test(&f.path, &doc.config)
        })
        .map(|f| f.path.as_str())
        .collect();
    let barrels = doc
        .files
        .iter()
        .filter(|f| f.barrel)
        .map(|f| f.path.clone())
        .collect();
    let entries = crate::modules::cycle_entries(
        doc.files.iter().map(|f| &f.path),
        &doc.config.source_root,
        &doc.package_entries,
    );
    let directory_units = doc.source.language == crate::model::Language::Go;
    let mut cycles = Vec::new();
    for original in std::mem::take(&mut doc.cycles) {
        let selected: BTreeSet<_> = original
            .members
            .iter()
            .filter(|m| candidates.contains_key(*m))
            .cloned()
            .collect();
        if original.level != CycleLevel::Module || selected.is_empty() {
            cycles.push(original);
            continue;
        }
        let members: BTreeSet<_> = original.members.iter().collect();
        let edges: Vec<_> = doc
            .edges
            .iter()
            .filter(|e| {
                !tests.contains(e.from.as_str())
                    && !tests.contains(e.to.as_str())
                    && members.contains(&module_of[&e.from])
                    && members.contains(&module_of[&e.to])
            })
            .collect();
        let split: BTreeMap<_, _> = module_of
            .iter()
            .map(|(file, module)| {
                // NUL cannot occur in a path or configured module ID, so temporary IDs cannot collide.
                (
                    file.clone(),
                    if selected.contains(module) {
                        format!("\0{file}")
                    } else {
                        module.clone()
                    },
                )
            })
            .collect();
        let split_pairs: Vec<_> = edges
            .iter()
            .filter(|e| doc.config.all_kinds || e.kind.is_runtime())
            .map(|e| (split[&e.from].as_str(), split[&e.to].as_str()))
            .filter(|(from, to)| from != to)
            .collect();
        let split_components = crate::graph::components(&split_pairs);
        let witnesses = edges.iter().copied().filter(|e| {
            (doc.config.all_kinds || e.kind.is_runtime())
                && split_components.iter().any(|members| {
                    members.contains(split[&e.from].as_str())
                        && members.contains(split[&e.to].as_str())
                })
        });
        // Recompute cuts using real module IDs and only edges proven to belong to a split SCC.
        let (remaining, _) = crate::quality::cycles(
            &module_of,
            witnesses,
            &entries,
            &barrels,
            directory_units,
            doc.config.all_kinds,
        );
        let remaining: Vec<_> = remaining
            .into_iter()
            .filter(|c| c.level == CycleLevel::Module)
            .collect();
        if remaining.iter().any(|c| c.members == original.members) {
            cycles.push(original);
            continue;
        }
        let involved: BTreeSet<_> = edges
            .iter()
            .filter(|e| {
                (doc.config.all_kinds || e.kind.is_runtime())
                    && module_of[&e.from] != module_of[&e.to]
            })
            .flat_map(|e| [&e.from, &e.to])
            .collect();
        doc.grabbags.push(Evidence {
            members: original.members,
            candidates: selected
                .iter()
                .map(|id| {
                    let mut candidate = candidates[id].clone();
                    candidate.files.retain(|f| involved.contains(f));
                    candidate
                })
                .collect(),
            edges: edges
                .iter()
                .filter(|e| doc.config.all_kinds || e.kind.is_runtime())
                .map(|e| Link {
                    from: e.from.clone(),
                    to: e.to.clone(),
                })
                .collect(),
            remaining_cycles: remaining.iter().map(|c| c.members.clone()).collect(),
        });
        cycles.extend(remaining);
    }
    cycles.sort_by(|a, b| {
        (a.level == CycleLevel::File, &a.members).cmp(&(b.level == CycleLevel::File, &b.members))
    });
    doc.cycles = cycles;
    let verified_runtime = if doc.config.all_kinds {
        let selected_cycles = std::mem::take(&mut doc.cycles);
        let selected_grabbags = std::mem::take(&mut doc.grabbags);
        let production_edges = doc
            .edges
            .iter()
            .filter(|e| !tests.contains(e.from.as_str()) && !tests.contains(e.to.as_str()));
        doc.cycles = crate::quality::cycles(
            &module_of,
            production_edges,
            &entries,
            &barrels,
            directory_units,
            false,
        )
        .0;
        doc.config.all_kinds = false;
        // One strict-runtime pass keeps deferred flags independent of --all-kinds selection.
        classify(doc);
        let runtime = std::mem::replace(&mut doc.cycles, selected_cycles);
        doc.grabbags = selected_grabbags;
        doc.config.all_kinds = true;
        runtime
    } else {
        doc.cycles.clone()
    };
    for deferred in &mut doc.type_only_cycles {
        deferred.contains_runtime_cycle = verified_runtime.iter().any(|cycle| {
            cycle.level == deferred.level
                && cycle
                    .members
                    .iter()
                    .all(|member| deferred.members.contains(member))
        });
    }
}
