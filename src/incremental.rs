//! Cached graph analysis over closed dirty regions; collection and policy remain caller-owned.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};

use crate::{
    graph,
    model::{Cycle, DeferredCycle, Edge, Module},
    modules, quality,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub entries: BTreeSet<String>,
    pub barrels: BTreeSet<String>,
    pub tests: BTreeSet<String>,
    pub directory_units: bool,
    pub all_kinds: bool,
}

#[derive(Debug, Default)]
pub struct Work {
    /// Modules whose SCCs and metrics were recomputed, including neighbours needed for closure.
    pub recomputed_modules: usize,
    pub total_modules: usize,
    pub changed_files: usize,
    pub fallback: Option<String>,
}

#[derive(Default)]
pub struct Analysis {
    module_of: BTreeMap<String, String>,
    edges: BTreeMap<String, Vec<Edge>>,
    neighbours: BTreeMap<String, BTreeSet<String>>,
    options: Options,
    modules: Vec<Module>,
    cycles: Vec<Cycle>,
    deferred: Vec<DeferredCycle>,
    components: Vec<BTreeSet<String>>,
    component_of: BTreeMap<String, usize>,
    condensation: Vec<BTreeSet<usize>>,
    initialized: bool,
}

impl Analysis {
    pub fn new(
        module_of: BTreeMap<String, String>,
        edges: Vec<Edge>,
        options: Options,
    ) -> Result<Self> {
        let mut state = Self::default();
        state.update(module_of, edges, options)?;
        Ok(state)
    }

    pub fn modules(&self) -> &[Module] {
        &self.modules
    }
    pub fn cycles(&self) -> &[Cycle] {
        &self.cycles
    }
    pub fn deferred_cycles(&self) -> &[DeferredCycle] {
        &self.deferred
    }
    pub fn components(&self) -> &[BTreeSet<String>] {
        &self.components
    }
    pub fn component_of(&self) -> &BTreeMap<String, usize> {
        &self.component_of
    }
    pub fn condensation(&self) -> &[BTreeSet<usize>] {
        &self.condensation
    }

    /// Inputs must be the same canonical internal edges and module assignment used by `build`.
    /// Weak closure in both snapshots handles insertions joining SCCs and deletions splitting
    /// them, and keeps barrel attribution and incoming metrics entirely within each region.
    pub fn update(
        &mut self,
        module_of: BTreeMap<String, String>,
        edges: Vec<Edge>,
        options: Options,
    ) -> Result<Work> {
        ensure!(
            edges
                .iter()
                .all(|e| module_of.contains_key(&e.from) && module_of.contains_key(&e.to)),
            "incremental analysis needs internal edge endpoints in module_of; resolve or exclude missing endpoints before updating"
        );
        let mut edges = edges;
        edges.sort_by(|a, b| {
            (&a.from, &a.to, &a.original_to, a.kind).cmp(&(&b.from, &b.to, &b.original_to, b.kind))
        });
        let mut by_file: BTreeMap<String, Vec<Edge>> = BTreeMap::new();
        let mut neighbours: BTreeMap<String, BTreeSet<String>> = module_of
            .values()
            .map(|m| (m.clone(), BTreeSet::new()))
            .collect();
        for edge in edges {
            let from = &module_of[&edge.from];
            let to = &module_of[&edge.to];
            neighbours.get_mut(from).unwrap().insert(to.clone());
            neighbours.get_mut(to).unwrap().insert(from.clone());
            by_file.entry(edge.from.clone()).or_default().push(edge);
        }
        let changed: BTreeSet<_> = self
            .module_of
            .keys()
            .chain(module_of.keys())
            .chain(self.edges.keys())
            .chain(by_file.keys())
            .filter(|file| {
                self.module_of.get(*file) != module_of.get(*file)
                    || self.edges.get(*file) != by_file.get(*file)
            })
            .cloned()
            .collect();
        let mut dirty: BTreeSet<String> = changed
            .iter()
            .flat_map(|f| self.module_of.get(f).into_iter().chain(module_of.get(f)))
            .cloned()
            .collect();
        let mut fallback = if !self.initialized {
            Some("initial graph".to_owned())
        } else if self.options != options {
            Some("analysis options or barrel/test classification changed".to_owned())
        } else {
            None
        };
        if fallback.is_some() {
            dirty.extend(self.neighbours.keys().chain(neighbours.keys()).cloned());
        }
        let production_counts = |files: &BTreeMap<String, Vec<Edge>>, opts: &Options| {
            let mut counts = (0, 0);
            for e in files
                .values()
                .flatten()
                .filter(|e| !opts.tests.contains(&e.from) && !opts.tests.contains(&e.to))
            {
                counts.0 += usize::from(e.kind.is_runtime());
                counts.1 += 1;
            }
            counts
        };
        let counts = production_counts(&by_file, &options);
        if counts != production_counts(&self.edges, &self.options) {
            // Public break-set weights use the total graph edge count, even across components.
            dirty.extend(
                options
                    .entries
                    .iter()
                    .filter_map(|f| module_of.get(f))
                    .cloned(),
            );
        }
        let mut queue: Vec<_> = dirty.iter().cloned().collect();
        while let Some(module) = queue.pop() {
            for next in self
                .neighbours
                .get(&module)
                .into_iter()
                .chain(neighbours.get(&module))
                .flatten()
            {
                if dirty.insert(next.clone()) {
                    queue.push(next.clone());
                }
            }
        }
        let total_modules = neighbours.len();
        let recomputed_modules = dirty.iter().filter(|m| neighbours.contains_key(*m)).count();
        if fallback.is_none() && recomputed_modules == total_modules && !dirty.is_empty() {
            fallback = Some("dirty dependency region spans the full graph".to_owned());
        }
        let local_of: BTreeMap<_, _> = module_of
            .iter()
            .filter(|(_, m)| dirty.contains(*m))
            .map(|(f, m)| (f.clone(), m.clone()))
            .collect();
        let local_edges: Vec<_> = by_file
            .iter()
            .filter(|(f, _)| local_of.contains_key(*f))
            .flat_map(|(_, es)| es.iter())
            .collect();
        let pairs = modules::weighted(local_edges.iter().copied(), &options.barrels);
        let mut metrics = graph::modules(&local_of, pairs.keys());
        quality::metrics(&mut metrics, &local_of, &pairs);
        self.modules.retain(|m| !dirty.contains(&m.id));
        self.modules.extend(metrics);
        self.modules.sort_by(|a, b| a.id.cmp(&b.id));
        let production: Vec<_> = local_edges
            .into_iter()
            .filter(|e| !options.tests.contains(&e.from) && !options.tests.contains(&e.to))
            .collect();
        let (cycles, deferred) = quality::cycles_with_budget(
            &local_of,
            production.iter().copied(),
            &options.entries,
            &options.barrels,
            options.directory_units,
            options.all_kinds,
            Some(counts),
        );
        let touches = |level: &crate::model::CycleLevel, members: &[String]| {
            members.iter().any(|m| match level {
                crate::model::CycleLevel::Module => dirty.contains(m),
                crate::model::CycleLevel::File => {
                    self.module_of.get(m).is_some_and(|id| dirty.contains(id))
                }
            })
        };
        self.cycles.retain(|c| !touches(&c.level, &c.members));
        self.cycles.extend(cycles);
        self.cycles.sort_by(|a, b| a.members.cmp(&b.members));
        self.deferred.retain(|c| !touches(&c.level, &c.members));
        self.deferred.extend(deferred);
        // quality::cycles groups deferred module cycles before file cycles.
        self.deferred.sort_by(|a, b| {
            let level = |c: &DeferredCycle| usize::from(c.level == crate::model::CycleLevel::File);
            (level(a), &a.members).cmp(&(level(b), &b.members))
        });
        self.components.retain(|c| c.is_disjoint(&dirty));
        let names: Vec<_> = local_of
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let ids: BTreeMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let mut adjacency = vec![Vec::new(); names.len()];
        for e in production
            .iter()
            .filter(|e| options.all_kinds || e.kind.is_runtime())
        {
            let (from, to) = (&local_of[&e.from], &local_of[&e.to]);
            if from != to {
                adjacency[ids[from.as_str()]].push(ids[to.as_str()]);
            }
        }
        self.components
            .extend(graph::sccs(&adjacency).into_iter().map(|c| {
                c.into_iter()
                    .map(|i| names[i].clone())
                    .collect::<BTreeSet<_>>()
            }));
        self.components.sort();
        self.component_of = self
            .components
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.iter().map(move |m| (m.clone(), i)))
            .collect();
        self.condensation = vec![BTreeSet::new(); self.components.len()];
        for e in by_file.values().flatten().filter(|e| {
            !options.tests.contains(&e.from)
                && !options.tests.contains(&e.to)
                && (options.all_kinds || e.kind.is_runtime())
        }) {
            let (from, to) = (
                self.component_of[&module_of[&e.from]],
                self.component_of[&module_of[&e.to]],
            );
            if from != to {
                self.condensation[from].insert(to);
            }
        }
        self.module_of = module_of;
        self.edges = by_file;
        self.neighbours = neighbours;
        self.options = options;
        self.initialized = true;
        Ok(Work {
            recomputed_modules,
            total_modules,
            changed_files: changed.len(),
            fallback,
        })
    }
}
