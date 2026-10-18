//! Physical folder extraction candidates, backed by import boundaries rather than build claims.

pub mod workspaces;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{Confidence, Document, Edge, EdgeKind, File, ImportRecord, ModuleClass, SCHEMA};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceInventory {
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosing_package: Option<String>,
    pub packages: Vec<ExistingPackage>,
    pub projects: Vec<ExistingProject>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingPackage {
    pub root: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingProject {
    pub root: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub min_cohesion: f64,
    pub max_cuts: usize,
    pub limit: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_cohesion: 0.8,
            max_cuts: 3,
            limit: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub kind: String,
    pub options: Options,
    pub workspace: WorkspaceInventory,
    pub candidates: Vec<Candidate>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub path: String,
    pub modules: Vec<String>,
    pub files: Vec<String>,
    pub classification: Classification,
    pub readiness: Readiness,
    pub fan_in: usize,
    pub size: usize,
    pub score: usize,
    pub cohesion: f64,
    pub internal_edges: usize,
    pub cut_list: Vec<Cut>,
    pub package_dependencies: Vec<PackageDependency>,
    pub external_packages: Vec<String>,
    pub uncertainties: Vec<String>,
    pub simulation: Simulation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    Free,
    Cheap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Readiness {
    ImportLevelReady,
    CutsRequired,
    NeedsVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cut {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub line: Option<usize>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDependency {
    pub from: String,
    pub to: String,
    pub package: Option<String>,
    pub root: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Simulation {
    pub cycles_before: usize,
    pub cycles_after: usize,
    pub boundary_cycle: Vec<String>,
    pub scope: String,
}

fn inside(path: &str, root: &str) -> bool {
    root == "." || path == root || path.strip_prefix(root).is_some_and(|p| p.starts_with('/'))
}

/// Every folder that `inside` places `path` in, other than ".".
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/')
        .map(|(i, _)| &path[..i])
        .chain([path])
}

fn owner<'a>(path: &str, workspace: &'a WorkspaceInventory) -> Option<&'a ExistingPackage> {
    workspace
        .packages
        .iter()
        .filter(|p| inside(path, &p.root))
        .max_by_key(|p| p.root.len())
}

#[derive(Clone)]
struct Dependency {
    cut: Cut,
    weight: f64,
}

/// The evidence one candidate folder reads, gathered once for all folders.
#[derive(Default)]
struct Within<'a> {
    files: Vec<&'a File>,
    internal_weights: Vec<f64>,
    dependencies: Vec<&'a Dependency>,
    imports: Vec<&'a ImportRecord>,
    low_confidence: Vec<&'a Edge>,
}

fn dependencies(doc: &Document) -> Vec<Dependency> {
    let weights = doc
        .config
        .rules
        .as_ref()
        .and_then(|r| r.doc.edge_weights)
        .unwrap_or_default();
    let mut pairs: BTreeMap<(String, String), Dependency> = BTreeMap::new();
    let mut insert = |from: &str, to: &str, kind: EdgeKind, line: Option<usize>, weight: f64| {
        if from == to {
            return;
        }
        let pair = pairs
            .entry((from.to_owned(), to.to_owned()))
            .or_insert_with(|| Dependency {
                cut: Cut {
                    from: from.to_owned(),
                    to: to.to_owned(),
                    kind,
                    line,
                    reason: "imports another unextracted folder".into(),
                },
                weight,
            });
        pair.weight = pair.weight.max(weight);
        if kind > pair.cut.kind {
            pair.cut.kind = kind;
        }
        if pair.cut.line.is_none() {
            pair.cut.line = line;
        }
    };
    for edge in &doc.edges {
        let weight = edge.weight.unwrap_or_else(|| weights.weight(edge.kind));
        insert(&edge.from, &edge.to, edge.kind, edge.line, weight);
        if let Some(original) = &edge.original_to {
            insert(&edge.from, original, edge.kind, edge.line, weight);
        }
    }
    for import in doc.imports.iter().flatten() {
        for target in import.targets.iter().chain(import.namespace.iter()) {
            insert(
                &import.from,
                target,
                import.kind,
                import.line,
                weights.weight(import.kind),
            );
        }
    }
    pairs.into_values().collect()
}

/// All selected files form the extraction boundary, including included tests and facades.
pub fn analyze(
    doc: &Document,
    options: Options,
    mut workspace: WorkspaceInventory,
) -> Result<Report> {
    if !options.min_cohesion.is_finite() || !(0.0..=1.0).contains(&options.min_cohesion) {
        bail!("minimum cohesion must be finite and between 0 and 1");
    }
    if options.limit == 0 {
        bail!("candidate limit must be greater than zero");
    }
    for root in &doc.source.collector.go_modules {
        if !workspace
            .packages
            .iter()
            .any(|package| package.root == *root)
        {
            workspace.packages.push(ExistingPackage {
                root: root.clone(),
                name: None,
            });
        }
    }
    for root in workspace
        .packages
        .iter()
        .map(|p| &p.root)
        .chain(workspace.projects.iter().map(|p| &p.root))
    {
        if !crate::collect::is_relative_path(root, true) {
            bail!("workspace root must be relative to the scan root: {root:?}");
        }
    }
    let files: BTreeMap<_, _> = doc.files.iter().map(|f| (f.path.as_str(), f)).collect();
    for path in files.keys() {
        if !crate::collect::is_relative_path(path, false) {
            bail!("package splits require relative source paths: {path:?}");
        }
    }
    let dependencies = dependencies(doc);
    for dependency in &dependencies {
        if !crate::collect::is_relative_path(&dependency.cut.from, false)
            || !crate::collect::is_relative_path(&dependency.cut.to, false)
        {
            bail!("package splits require relative import destinations");
        }
    }
    let directories: BTreeSet<_> = files
        .keys()
        .flat_map(|path| path.match_indices('/').map(|(i, _)| path[..i].to_owned()))
        .collect();
    let production = |path: &str| {
        files
            .get(path)
            .is_some_and(|f| f.class != Some(ModuleClass::Tests))
    };
    // Facade and occurrence evidence constrains extraction, but must not inflate cohesion.
    let weights = doc
        .config
        .rules
        .as_ref()
        .and_then(|r| r.doc.edge_weights)
        .unwrap_or_default();
    let mut metric_pairs: BTreeMap<(&str, &str), f64> = BTreeMap::new();
    for edge in &doc.edges {
        if edge.from == edge.to
            || !production(&edge.from)
            || !production(&edge.to)
            || files[edge.from.as_str()].barrel
        {
            continue;
        }
        let weight = edge.weight.unwrap_or_else(|| weights.weight(edge.kind));
        let pair = metric_pairs.entry((&edge.from, &edge.to)).or_default();
        *pair = pair.max(weight);
    }
    let mut within: BTreeMap<&str, Within> = BTreeMap::new();
    for file in &doc.files {
        for folder in ancestors(&file.path) {
            within.entry(folder).or_default().files.push(file);
        }
    }
    for ((from, to), weight) in &metric_pairs {
        for folder in ancestors(from).filter(|folder| inside(to, folder)) {
            within
                .entry(folder)
                .or_default()
                .internal_weights
                .push(*weight);
        }
    }
    for dependency in &dependencies {
        let cut = &dependency.cut;
        let to_only = ancestors(&cut.to).filter(|folder| !inside(&cut.from, folder));
        for folder in ancestors(&cut.from).chain(to_only) {
            within
                .entry(folder)
                .or_default()
                .dependencies
                .push(dependency);
        }
    }
    for import in doc.imports.iter().flatten() {
        for folder in ancestors(&import.from) {
            within.entry(folder).or_default().imports.push(import);
        }
    }
    for edge in doc
        .edges
        .iter()
        .filter(|e| e.confidence == Some(Confidence::Low))
    {
        for folder in ancestors(&edge.from) {
            within.entry(folder).or_default().low_confidence.push(edge);
        }
    }
    let mut candidates = Vec::new();
    for path in directories {
        if workspace
            .packages
            .iter()
            .map(|p| &p.root)
            .chain(workspace.projects.iter().map(|p| &p.root))
            .any(|root| root != "." && inside(root, &path))
        {
            continue;
        }
        let within = within.remove(path.as_str()).unwrap_or_default();
        let selected = within.files;
        let size = selected.iter().filter(|f| production(&f.path)).count();
        if size < 2
            || selected
                .iter()
                .filter(|f| production(&f.path))
                .all(|f| f.barrel)
        {
            continue;
        }
        let internal = within.internal_weights.len();
        let internal_weight: f64 = within.internal_weights.iter().sum();
        let mut outgoing_weight = 0.0;
        let mut cut_list = Vec::new();
        let mut package_dependencies = Vec::new();
        let mut consumers = BTreeSet::new();
        let mut uncertainties = BTreeSet::new();
        for dependency in within.dependencies {
            let cut = &dependency.cut;
            let from_inside = inside(&cut.from, &path);
            let to_inside = inside(&cut.to, &path);
            if from_inside && to_inside {
                if !files.contains_key(cut.to.as_str()) {
                    uncertainties.insert(format!(
                        "unscanned local dependency: {} → {}",
                        cut.from, cut.to
                    ));
                }
            } else if from_inside {
                let package = owner(&cut.to, &workspace).filter(|target| {
                    target.root != "."
                        && owner(&cut.from, &workspace)
                            .is_none_or(|source| source.root != target.root)
                });
                if let Some(package) = package {
                    package_dependencies.push(PackageDependency {
                        from: cut.from.clone(),
                        to: cut.to.clone(),
                        package: package.name.clone(),
                        root: package.root.clone(),
                    });
                } else {
                    if production(&cut.from) {
                        outgoing_weight += dependency.weight;
                    }
                    cut_list.push(cut.clone());
                }
            } else if to_inside && production(&cut.from) {
                consumers.insert(files[cut.from.as_str()].module.clone());
            }
        }
        if internal == 0
            || internal_weight == 0.0
            || consumers.is_empty()
            || cut_list.len() > options.max_cuts
        {
            continue;
        }
        let cohesion = internal_weight / (internal_weight + outgoing_weight);
        if cohesion < options.min_cohesion {
            continue;
        }
        let mut external_packages = BTreeSet::new();
        match &doc.imports {
            None => {
                uncertainties.insert(
                    "explicit import ledger unavailable; omitted imports cannot be checked".into(),
                );
            }
            Some(_) => {
                for import in within.imports {
                    if let Some(package) = &import.external {
                        external_packages.insert(package.clone());
                        let own_package = workspace.enclosing_package.as_ref() == Some(package)
                            || owner(&import.from, &workspace).and_then(|p| p.name.as_ref())
                                == Some(package);
                        let named_package = !import.specifier.starts_with(['.', '/', '\\', '#'])
                            && (import.specifier == *package
                                || import
                                    .specifier
                                    .strip_prefix(package)
                                    .is_some_and(|suffix| {
                                        suffix.starts_with('/')
                                            || suffix.starts_with('.')
                                            || suffix.starts_with("::")
                                    }));
                        if own_package || !named_package {
                            uncertainties.insert(format!("out-of-scan, local-alias or self-package import: {}:{} {} (recorded as external {})", import.from, import.line.map_or_else(|| "?".into(), |n| n.to_string()), import.specifier, package));
                        }
                    }
                    if import.unresolved.is_some() || import.namespace.is_some() {
                        uncertainties.insert(format!(
                            "unresolved or namespace import: {}:{} {}",
                            import.from,
                            import.line.map_or_else(|| "?".into(), |n| n.to_string()),
                            import.specifier
                        ));
                    }
                }
            }
        }
        for edge in within.low_confidence {
            uncertainties.insert(format!(
                "low-confidence import: {} → {}",
                edge.from, edge.to
            ));
        }
        if !workspace.checked {
            uncertainties.insert("workspace metadata was not checked".into());
        }
        let classification = if cut_list.is_empty() {
            Classification::Free
        } else {
            Classification::Cheap
        };
        let readiness = if !uncertainties.is_empty() {
            Readiness::NeedsVerification
        } else if classification == Classification::Free {
            Readiness::ImportLevelReady
        } else {
            Readiness::CutsRequired
        };
        let modules: BTreeSet<_> = selected
            .iter()
            .filter(|f| production(&f.path))
            .map(|f| f.module.clone())
            .collect();
        let fan_in = consumers.len();
        candidates.push(Candidate {
            path,
            modules: modules.into_iter().collect(),
            files: selected.iter().map(|f| f.path.clone()).collect(),
            classification,
            readiness,
            fan_in,
            size,
            score: fan_in
                .checked_mul(size)
                .ok_or_else(|| anyhow::anyhow!("package score overflow"))?,
            cohesion,
            internal_edges: internal,
            cut_list,
            package_dependencies,
            external_packages: external_packages.into_iter().collect(),
            uncertainties: uncertainties.into_iter().collect(),
            simulation: Simulation::default(),
        });
    }
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.cut_list.len().cmp(&b.cut_list.len()))
            .then(a.path.cmp(&b.path))
    });
    candidates.truncate(options.limit);
    for candidate in &mut candidates {
        candidate.simulation = simulate(doc, &dependencies, candidate);
        if !candidate.simulation.boundary_cycle.is_empty() {
            candidate.readiness = Readiness::NeedsVerification;
            candidate
                .uncertainties
                .push("package boundary participates in a cycle after extraction".into());
        }
    }
    Ok(Report {
        schema: SCHEMA.into(),
        kind: "package-splits".into(),
        options,
        workspace,
        candidates,
        limitations: vec![
            "Import-level suggestions only: build configuration, manifests, exports, consumer import paths, shared test utilities and environment assumptions are not validated.".into(),
            "All import kinds constrain boundaries; cycle simulation uses the scan's runtime/all-kinds setting and excludes tests.".into(),
            "External package internals and consumers outside this scan were not traversed. Overlapping folder candidates are alternatives, not simultaneous extraction plans.".into(),
        ],
    })
}

/// Module cycles among `links`, self-imports aside.
fn components(links: &BTreeSet<(String, String)>) -> Vec<BTreeSet<&str>> {
    let pairs: Vec<_> = links
        .iter()
        .filter(|(from, to)| from != to)
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect();
    crate::graph::components(&pairs)
}

fn simulate(doc: &Document, dependencies: &[Dependency], candidate: &Candidate) -> Simulation {
    let module_of: BTreeMap<_, _> = doc
        .files
        .iter()
        .filter(|f| f.class != Some(ModuleClass::Tests))
        .map(|f| (f.path.as_str(), f.module.as_str()))
        .collect();
    let runtime: Vec<_> = dependencies
        .iter()
        .filter(|d| doc.config.all_kinds || d.cut.kind.is_runtime())
        .filter(|d| {
            module_of.contains_key(d.cut.from.as_str()) && module_of.contains_key(d.cut.to.as_str())
        })
        .collect();
    let before_links: BTreeSet<_> = runtime
        .iter()
        .map(|d| {
            (
                module_of[d.cut.from.as_str()].to_owned(),
                module_of[d.cut.to.as_str()].to_owned(),
            )
        })
        .collect();
    let before = components(&before_links);
    let mut nodes: BTreeSet<String> = module_of
        .iter()
        .filter(|(file, _)| !inside(file, &candidate.path))
        .map(|(_, module)| (*module).to_owned())
        .collect();
    let after_links: BTreeSet<_> = runtime
        .iter()
        .filter(|d| !inside(&d.cut.from, &candidate.path) && !inside(&d.cut.to, &candidate.path))
        .map(|d| {
            (
                module_of[d.cut.from.as_str()].to_owned(),
                module_of[d.cut.to.as_str()].to_owned(),
            )
        })
        .collect();
    let after = components(&after_links);
    let mut package_id = "\0extracted-package".to_owned();
    while nodes.contains(&package_id) {
        package_id.push('\0');
    }
    nodes.insert(package_id.clone());
    let cuts: BTreeSet<_> = candidate
        .cut_list
        .iter()
        .map(|c| (c.from.as_str(), c.to.as_str()))
        .collect();
    let package_links: BTreeSet<_> = runtime
        .iter()
        .filter(|d| !cuts.contains(&(d.cut.from.as_str(), d.cut.to.as_str())))
        .map(|d| {
            let node = |file: &str| {
                if inside(file, &candidate.path) {
                    package_id.clone()
                } else {
                    module_of[file].to_owned()
                }
            };
            (node(&d.cut.from), node(&d.cut.to))
        })
        .collect();
    let boundary_cycle = components(&package_links)
        .into_iter()
        .find(|s| s.contains(package_id.as_str()))
        .map(|s| {
            s.into_iter()
                .filter(|m| *m != package_id)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Simulation {
        cycles_before: before.len(),
        cycles_after: after.len(),
        boundary_cycle,
        scope: if doc.config.all_kinds {
            "all-kinds"
        } else {
            "runtime"
        }
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Edge, Granularity, ImportReason, ImportRecord, Source};
    use serde_json::json;

    fn scan(pairs: &[(&str, &str)]) -> Document {
        let config: Config = serde_json::from_value(json!({"depth": 1, "source_root": ".", "excludes": [], "include_tests": false, "rules": null})).unwrap();
        let source: Source = serde_json::from_value(json!({"language": "typescript", "collector": {"name": "fixture", "cruised": 0}, "generated_at": "test"})).unwrap();
        let edges: Vec<_> = pairs
            .iter()
            .map(|(from, to)| Edge {
                from: (*from).into(),
                to: (*to).into(),
                line: Some(1),
                ..Default::default()
            })
            .collect();
        let files = edges
            .iter()
            .flat_map(|e| [e.from.clone(), e.to.clone()])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut doc = crate::analyze(source, config, files, edges, Vec::new(), None).unwrap();
        doc.imports = Some(
            doc.edges
                .iter()
                .map(|e| ImportRecord {
                    from: e.from.clone(),
                    specifier: e.to.clone(),
                    targets: vec![e.to.clone()],
                    kind: e.kind,
                    line: e.line,
                    ..Default::default()
                })
                .collect(),
        );
        doc
    }

    fn inventory() -> WorkspaceInventory {
        WorkspaceInventory {
            checked: true,
            ..Default::default()
        }
    }
    fn report(doc: &Document) -> Report {
        analyze(doc, Options::default(), inventory()).unwrap()
    }
    fn base() -> Document {
        scan(&[
            ("app/a.ts", "lib/core/a.ts"),
            ("lib/core/a.ts", "lib/core/b.ts"),
        ])
    }
    fn candidate<'a>(report: &'a Report, path: &str) -> &'a Candidate {
        report.candidates.iter().find(|c| c.path == path).unwrap()
    }

    #[test]
    fn free_candidates_use_physical_subtrees() {
        let doc = base();
        let report = report(&doc);
        let core = candidate(&report, "lib/core");
        assert_eq!(core.classification, Classification::Free);
        assert_eq!(core.readiness, Readiness::ImportLevelReady);
        assert_eq!((core.fan_in, core.size, core.score), (1, 2, 2));
        assert_eq!(core.cohesion, 1.0);
        assert_eq!(core.modules, ["lib"]);
        assert!(report.candidates.iter().any(|c| c.path == "lib"));
    }

    #[test]
    fn cheap_candidates_list_all_cuts_and_respect_thresholds() {
        let mut doc = base();
        doc.edges.push(Edge {
            from: "lib/core/a.ts".into(),
            to: "app/a.ts".into(),
            kind: EdgeKind::Type,
            line: Some(17),
            ..Default::default()
        });
        let result = report(&doc);
        let core = candidate(&result, "lib/core");
        assert_eq!(core.classification, Classification::Cheap);
        assert_eq!(core.readiness, Readiness::CutsRequired);
        assert_eq!(core.cut_list[0].line, Some(17));
        assert_eq!(core.cut_list[0].kind, EdgeKind::Type);
        assert!(core.cohesion > 0.9);
        let options = Options {
            max_cuts: 0,
            ..Default::default()
        };
        assert!(
            analyze(&doc, options, inventory())
                .unwrap()
                .candidates
                .is_empty()
        );
        doc.edges.last_mut().unwrap().kind = EdgeKind::Static;
        assert!(report(&doc).candidates.is_empty());
    }

    #[test]
    fn candidates_need_production_consumers_and_internal_cohesion() {
        let doc = scan(&[("lib/a.ts", "lib/b.ts")]);
        assert!(report(&doc).candidates.is_empty());
        let mut doc = base();
        doc.files
            .iter_mut()
            .find(|f| f.path == "app/a.ts")
            .unwrap()
            .class = Some(ModuleClass::Tests);
        assert!(report(&doc).candidates.is_empty());
        let doc = scan(&[("app/a.ts", "lib/a.ts"), ("app/a.ts", "lib/b.ts")]);
        assert!(report(&doc).candidates.is_empty());
    }

    #[test]
    fn file_granularity_does_not_change_folder_boundaries_or_candidate_score() {
        let mut doc = base();
        doc.config.granularity = Granularity::File;
        for file in &mut doc.files {
            file.module = file.path.clone();
        }
        let result = report(&doc);
        assert_eq!(candidate(&result, "lib/core").modules.len(), 2);
        assert_eq!(candidate(&result, "lib/core").score, 2);
    }

    #[test]
    fn existing_packages_and_projects_are_not_extracted_but_subfolders_are() {
        let doc = base();
        let mut workspace = inventory();
        workspace.packages.push(ExistingPackage {
            root: "lib".into(),
            name: Some("@repo/lib".into()),
        });
        let result = analyze(&doc, Options::default(), workspace.clone()).unwrap();
        assert!(result.candidates.iter().all(|c| c.path != "lib"));
        assert!(result.candidates.iter().any(|c| c.path == "lib/core"));
        workspace.projects.push(ExistingProject {
            root: "lib/core".into(),
            name: None,
        });
        assert!(
            analyze(&doc, Options::default(), workspace)
                .unwrap()
                .candidates
                .is_empty()
        );
    }

    #[test]
    fn recorded_go_module_roots_are_existing_packages() {
        let mut doc = base();
        doc.source.collector.go_modules = vec!["lib".into()];
        let result = report(&doc);
        assert!(
            result
                .candidates
                .iter()
                .all(|candidate| candidate.path != "lib")
        );
        assert!(
            result
                .candidates
                .iter()
                .any(|candidate| candidate.path == "lib/core")
        );
        assert!(
            result
                .workspace
                .packages
                .iter()
                .any(|package| package.root == "lib")
        );
    }

    #[test]
    fn dependency_on_another_workspace_package_is_allowed_but_own_parent_is_not() {
        let doc = scan(&[
            ("app/a.ts", "lib/core/a.ts"),
            ("lib/core/a.ts", "lib/core/b.ts"),
            ("lib/core/a.ts", "other/x.ts"),
        ]);
        let mut workspace = inventory();
        workspace.packages.push(ExistingPackage {
            root: ".".into(),
            name: Some("root".into()),
        });
        assert!(
            analyze(&doc, Options::default(), workspace.clone())
                .unwrap()
                .candidates
                .is_empty()
        );
        workspace.projects.push(ExistingProject {
            root: "other".into(),
            name: None,
        });
        assert!(
            analyze(&doc, Options::default(), workspace.clone())
                .unwrap()
                .candidates
                .is_empty()
        );
        workspace.packages.push(ExistingPackage {
            root: "other".into(),
            name: Some("@repo/other".into()),
        });
        let result = analyze(&doc, Options::default(), workspace).unwrap();
        let core = candidate(&result, "lib/core");
        assert_eq!(core.classification, Classification::Free);
        assert_eq!(
            core.package_dependencies[0].package.as_deref(),
            Some("@repo/other")
        );
    }

    #[test]
    fn written_barrel_and_lazy_edges_remain_boundary_dependencies() {
        let mut doc = base();
        doc.edges
            .iter_mut()
            .find(|e| e.from == "lib/core/a.ts")
            .unwrap()
            .original_to = Some("outside/index.ts".into());
        assert!(report(&doc).candidates.is_empty());
        doc.edges
            .iter_mut()
            .find(|e| e.from == "lib/core/a.ts")
            .unwrap()
            .kind = EdgeKind::Type;
        doc.edges
            .iter_mut()
            .find(|e| e.from == "lib/core/a.ts")
            .unwrap()
            .weight = None;
        // The runtime ledger occurrence wins internally; the weak written route still needs a cut.
        let result = analyze(
            &doc,
            Options {
                min_cohesion: 0.0,
                ..Default::default()
            },
            inventory(),
        )
        .unwrap();
        assert_eq!(
            candidate(&result, "lib/core").cut_list[0].to,
            "outside/index.ts"
        );
    }

    #[test]
    fn ledger_sees_excluded_targets_and_unknown_imports_prevent_ready_claim() {
        let mut doc = base();
        doc.imports.as_mut().unwrap().push(ImportRecord {
            from: "lib/core/a.ts".into(),
            specifier: "./generated".into(),
            targets: vec!["lib/core/generated.ts".into()],
            ..Default::default()
        });
        let result = report(&doc);
        assert_eq!(
            candidate(&result, "lib/core").readiness,
            Readiness::NeedsVerification
        );
        doc.imports.as_mut().unwrap().push(ImportRecord {
            from: "lib/core/a.ts".into(),
            specifier: "variable".into(),
            unresolved: Some(ImportReason::Dynamic),
            ..Default::default()
        });
        assert!(
            candidate(&report(&doc), "lib/core")
                .uncertainties
                .iter()
                .any(|s| s.contains("variable"))
        );
        doc.imports = None;
        assert_eq!(
            candidate(&report(&doc), "lib/core").readiness,
            Readiness::NeedsVerification
        );
        assert_eq!(
            candidate(
                &analyze(&base(), Options::default(), WorkspaceInventory::default()).unwrap(),
                "lib/core"
            )
            .readiness,
            Readiness::NeedsVerification
        );
    }

    #[test]
    fn included_test_imports_and_unscanned_outbound_targets_require_cuts() {
        let mut doc = base();
        doc.files.push(crate::model::File {
            path: "lib/core/test.ts".into(),
            module: "tests::lib".into(),
            barrel: false,
            class: Some(ModuleClass::Tests),
            owners: vec![],
        });
        doc.imports.as_mut().unwrap().push(ImportRecord {
            from: "lib/core/test.ts".into(),
            targets: vec!["tests/support.ts".into()],
            ..Default::default()
        });
        let result = report(&doc);
        let core = candidate(&result, "lib/core");
        assert_eq!(core.size, 2);
        assert_eq!(core.files.len(), 3);
        assert_eq!(core.classification, Classification::Cheap);
        assert_eq!(core.cut_list[0].to, "tests/support.ts");
    }

    #[test]
    fn package_condensation_detects_a_new_boundary_cycle() {
        let mut doc = scan(&[
            ("app/a.ts", "lib/a.ts"),
            ("lib/a.ts", "lib/b.ts"),
            ("lib/b.ts", "other/o.ts"),
            ("other/o.ts", "lib/c.ts"),
            ("lib/c.ts", "lib/d.ts"),
        ]);
        for file in &mut doc.files {
            file.module = file.path.clone();
        }
        let mut workspace = inventory();
        workspace.packages.push(ExistingPackage {
            root: "other".into(),
            name: None,
        });
        let result = analyze(&doc, Options::default(), workspace).unwrap();
        let lib = candidate(&result, "lib");
        assert_eq!(lib.classification, Classification::Free);
        assert_eq!(lib.readiness, Readiness::NeedsVerification);
        assert!(
            lib.simulation
                .boundary_cycle
                .iter()
                .any(|m| m == "other/o.ts")
        );
    }

    #[test]
    fn removing_a_candidate_can_split_an_old_scc_without_creating_cycles() {
        let doc = scan(&[
            ("lib/a.ts", "lib/b.ts"),
            ("lib/b.ts", "a/x.ts"),
            ("lib/b.ts", "c/x.ts"),
            ("a/x.ts", "lib/a.ts"),
            ("c/x.ts", "lib/a.ts"),
            ("a/x.ts", "b/x.ts"),
            ("b/x.ts", "a/x.ts"),
            ("c/x.ts", "d/x.ts"),
            ("d/x.ts", "c/x.ts"),
        ]);
        let result = analyze(
            &doc,
            Options {
                min_cohesion: 0.0,
                ..Default::default()
            },
            inventory(),
        )
        .unwrap();
        let simulation = &candidate(&result, "lib").simulation;
        assert_eq!((simulation.cycles_before, simulation.cycles_after), (1, 2));
    }

    #[test]
    fn every_import_kind_requires_a_cut_and_pairs_are_deduplicated() {
        for kind in [EdgeKind::Type, EdgeKind::Dynamic, EdgeKind::Reexport] {
            let mut doc = base();
            for line in [None, Some(7)] {
                doc.edges.push(Edge {
                    from: "lib/core/a.ts".into(),
                    to: "app/a.ts".into(),
                    kind,
                    line,
                    ..Default::default()
                });
            }
            let result = analyze(
                &doc,
                Options {
                    min_cohesion: 0.0,
                    ..Default::default()
                },
                inventory(),
            )
            .unwrap();
            let core = candidate(&result, "lib/core");
            assert_eq!(core.classification, Classification::Cheap);
            assert_eq!(core.cut_list.len(), 1);
            assert_eq!(core.cut_list[0].line, Some(7));
        }
    }

    #[test]
    fn external_packages_are_reported_and_namespace_and_low_confidence_are_uncertain() {
        let mut doc = base();
        doc.imports.as_mut().unwrap().push(ImportRecord {
            from: "lib/core/a.ts".into(),
            specifier: "react".into(),
            external: Some("react".into()),
            ..Default::default()
        });
        assert_eq!(
            candidate(&report(&doc), "lib/core").external_packages,
            ["react"]
        );
        doc.imports.as_mut().unwrap().push(ImportRecord {
            from: "lib/core/a.ts".into(),
            namespace: Some("lib/core/ns".into()),
            ..Default::default()
        });
        assert_eq!(
            candidate(&report(&doc), "lib/core").readiness,
            Readiness::NeedsVerification
        );
        doc.imports.as_mut().unwrap().pop();
        doc.edges[0].confidence = Some(Confidence::Low);
        // Only uncertain imports originating inside the extraction boundary count.
        assert_eq!(
            candidate(&report(&doc), "lib/core").readiness,
            Readiness::ImportLevelReady
        );
        doc.edges
            .iter_mut()
            .find(|e| e.from == "lib/core/a.ts")
            .unwrap()
            .confidence = Some(Confidence::Low);
        assert_eq!(
            candidate(&report(&doc), "lib/core").readiness,
            Readiness::NeedsVerification
        );
    }

    #[test]
    fn facade_and_ledger_routes_do_not_inflate_internal_cohesion() {
        let mut doc = scan(&[
            ("app/a.ts", "lib/a.ts"),
            ("lib/a.ts", "lib/definition.ts"),
            ("lib/b.ts", "lib/definition.ts"),
            ("lib/c.ts", "lib/definition.ts"),
            ("lib/a.ts", "outside/x.ts"),
        ]);
        doc.files.push(crate::model::File {
            path: "lib/index.ts".into(),
            module: "lib".into(),
            barrel: true,
            class: None,
            owners: vec![],
        });
        for edge in doc.edges.iter_mut().filter(|e| e.to == "lib/definition.ts") {
            edge.original_to = Some("lib/index.ts".into());
        }
        doc.edges.push(Edge {
            from: "lib/index.ts".into(),
            to: "lib/definition.ts".into(),
            kind: EdgeKind::Reexport,
            weight: Some(1.0),
            ..Default::default()
        });
        // The three source→definition imports are three metric pairs, not six facade routes.
        assert!(report(&doc).candidates.iter().all(|c| c.path != "lib"));
        let result = analyze(
            &doc,
            Options {
                min_cohesion: 0.7,
                ..Default::default()
            },
            inventory(),
        )
        .unwrap();
        let lib = candidate(&result, "lib");
        assert_eq!(lib.internal_edges, 3);
        assert_eq!(lib.cohesion, 0.75);
    }

    #[test]
    fn external_proof_does_not_hide_local_or_self_package_imports() {
        for specifier in [
            "../../shared.ts",
            "#shared",
            "@alias/shared",
            "@repo/lib/shared",
        ] {
            let mut doc = base();
            doc.imports.as_mut().unwrap().push(ImportRecord {
                from: "lib/core/a.ts".into(),
                specifier: specifier.into(),
                external: Some("@repo/lib".into()),
                ..Default::default()
            });
            if !specifier.starts_with("@repo/lib") {
                assert_eq!(
                    candidate(&report(&doc), "lib/core").readiness,
                    Readiness::NeedsVerification
                );
            }
            let mut workspace = inventory();
            workspace.enclosing_package = Some("@repo/lib".into());
            let result = analyze(&doc, Options::default(), workspace).unwrap();
            let core = candidate(&result, "lib/core");
            assert_eq!(core.readiness, Readiness::NeedsVerification, "{specifier}");
            assert!(
                core.uncertainties
                    .iter()
                    .any(|s| s.contains("self-package"))
            );
        }
        let mut doc = base();
        doc.imports.as_mut().unwrap().push(ImportRecord {
            from: "lib/core/a.ts".into(),
            specifier: "@repo/lib/shared".into(),
            external: Some("@repo/lib".into()),
            ..Default::default()
        });
        let mut workspace = inventory();
        workspace.packages.push(ExistingPackage {
            root: "lib".into(),
            name: Some("@repo/lib".into()),
        });
        assert_eq!(
            candidate(
                &analyze(&doc, Options::default(), workspace).unwrap(),
                "lib/core"
            )
            .readiness,
            Readiness::NeedsVerification
        );
        for (specifier, package) in [
            ("react", "react"),
            ("react/jsx-runtime", "react"),
            ("os.path", "os"),
            ("std::io", "std"),
        ] {
            let mut doc = base();
            doc.imports.as_mut().unwrap().push(ImportRecord {
                from: "lib/core/a.ts".into(),
                specifier: specifier.into(),
                external: Some(package.into()),
                ..Default::default()
            });
            assert_eq!(
                candidate(&report(&doc), "lib/core").readiness,
                Readiness::ImportLevelReady,
                "{specifier}"
            );
        }
    }

    #[test]
    fn invalid_options_and_paths_are_rejected_and_ranking_is_stable() {
        let doc = base();
        assert!(
            analyze(
                &doc,
                Options {
                    min_cohesion: f64::NAN,
                    ..Default::default()
                },
                inventory()
            )
            .is_err()
        );
        assert!(
            analyze(
                &doc,
                Options {
                    limit: 0,
                    ..Default::default()
                },
                inventory()
            )
            .is_err()
        );
        let mut workspace = inventory();
        workspace.packages.push(ExistingPackage {
            root: "../outside".into(),
            name: None,
        });
        assert!(analyze(&doc, Options::default(), workspace).is_err());
        let report = report(&doc);
        assert_eq!(report.candidates[0].path, "lib");
    }
}
