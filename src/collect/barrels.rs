//! Symbol-directed forwarding resolution shared by source collectors.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{RawGraph, imports};
use crate::model::{Confidence, Edge, EdgeKind, NodeMetadata};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarrelModule {
    pub file: String,
    pub pure: bool,
    pub declared: Vec<String>,
    pub exports: Vec<ExportRoute>,
    pub imports: Vec<ImportRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRoute {
    pub exported: String,
    pub imported: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRoute {
    pub to: String,
    pub names: Vec<String>,
    pub wildcard: bool,
    pub kind: EdgeKind,
    pub specifier: Option<String>,
    pub line: usize,
    pub side_effect: bool,
}

pub fn resolve(graph: &mut RawGraph, modules: &[BarrelModule]) -> Vec<String> {
    let modules: BTreeMap<_, _> = modules
        .iter()
        .map(|module| (module.file.as_str(), module))
        .collect();
    for module in modules.values() {
        graph.metadata.insert(
            module.file.clone(),
            NodeMetadata {
                barrel: module.pure,
                ..Default::default()
            },
        );
    }
    let mut warnings = BTreeSet::new();
    let mut output = Vec::new();
    for edge in &graph.edges {
        let routes: Vec<_> = modules
            .get(edge.from.as_str())
            .into_iter()
            .flat_map(|module| &module.imports)
            .filter(|route| route.to == edge.to)
            .collect();
        if routes.is_empty() {
            output.push(edge.clone());
            continue;
        }
        for route in routes {
            let mut updated = edge.clone();
            updated.kind = if matches!(edge.kind, EdgeKind::Type | EdgeKind::TypeReexport)
                && matches!(route.kind, EdgeKind::Static | EdgeKind::Reexport)
            {
                if route.kind == EdgeKind::Reexport {
                    EdgeKind::TypeReexport
                } else {
                    EdgeKind::Type
                }
            } else {
                route.kind
            };
            updated.weight = None;
            updated.is_reexport = matches!(route.kind, EdgeKind::Reexport | EdgeKind::TypeReexport)
                || modules
                    .get(edge.from.as_str())
                    .is_some_and(|module| module.pure)
                    && !route.side_effect;
            let source_is_barrel = modules
                .get(edge.from.as_str())
                .is_some_and(|module| module.pure);
            if source_is_barrel && updated.is_reexport && updated.kind == EdgeKind::Static {
                updated.kind = EdgeKind::Reexport;
            } else if !source_is_barrel && updated.kind == EdgeKind::Reexport {
                // Executable modules still load the modules they re-export.
                updated.kind = EdgeKind::Static;
            }
            updated.specifier = route.specifier.clone();
            updated.line = Some(route.line);
            updated.names = route.names.clone();
            let target_is_barrel = modules
                .get(route.to.as_str())
                .is_some_and(|module| module.pure);
            if !target_is_barrel || route.side_effect {
                if route.side_effect {
                    updated.kind = EdgeKind::Static;
                    updated.is_reexport = false;
                }
                output.push(updated);
                continue;
            }
            let mut targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut failure = None;
            if route.wildcard || route.names.is_empty() {
                failure = Some("namespace or star import cannot be narrowed".to_owned());
            } else {
                for name in &route.names {
                    match target(&modules, &route.to, name, 0, &mut BTreeSet::new()) {
                        Ok(file) => {
                            targets.entry(file).or_default().push(name.clone());
                        }
                        Err(reason) => {
                            failure = Some(reason);
                            break;
                        }
                    }
                }
            }
            if let Some(reason) = failure {
                updated.kind = EdgeKind::Reexport;
                updated.confidence = Some(Confidence::Low);
                updated.resolution_reason = Some(reason.clone());
                warnings.insert(format!(
                    "{}:{} → {}: {reason}",
                    updated.from, route.line, route.to
                ));
                output.push(updated);
            } else {
                for (file, names) in targets {
                    let mut resolved = updated.clone();
                    resolved
                        .original_to
                        .get_or_insert_with(|| resolved.to.clone());
                    resolved.to = file;
                    resolved.names = names;
                    resolved.confidence = Some(Confidence::High);
                    output.push(resolved);
                }
            }
        }
    }
    // Keep the strongest occurrence of a pair; a real runtime occurrence beats structural wiring.
    let mut unique: BTreeMap<(String, String, String), Edge> = BTreeMap::new();
    for edge in output {
        let key = (
            edge.from.clone(),
            edge.to.clone(),
            edge.original_to.clone().unwrap_or_else(|| edge.to.clone()),
        );
        match unique.get_mut(&key) {
            Some(existing) if edge.kind > existing.kind => *existing = edge,
            Some(existing)
                if edge.kind == existing.kind
                    && existing.confidence == Some(Confidence::Low)
                    && edge.confidence != Some(Confidence::Low) =>
            {
                *existing = edge
            }
            None => {
                unique.insert(key, edge);
            }
            _ => {}
        }
    }
    graph.edges = unique.into_values().collect();
    warnings.into_iter().collect()
}

fn target<'a>(
    modules: &BTreeMap<&str, &'a BarrelModule>,
    file: &'a str,
    name: &str,
    depth: usize,
    visiting: &mut BTreeSet<(String, String)>,
) -> std::result::Result<String, String> {
    let Some(module) = modules.get(file) else {
        return Err(format!("missing export metadata for {file}"));
    };
    if !module.pure {
        return Ok(file.to_owned());
    }
    if depth >= 10 {
        return Err("re-export chain exceeds depth 10".into());
    }
    let key = (file.to_owned(), name.to_owned());
    if !visiting.insert(key.clone()) {
        return Err(format!("re-export chain cycle at {file}:{name}"));
    }
    let explicit: Vec<_> = module
        .exports
        .iter()
        .filter(|route| route.exported == name)
        .collect();
    let candidates: Vec<_> = if explicit.is_empty() {
        module
            .exports
            .iter()
            .filter(|route| route.exported == "*" && name != "default")
            .collect()
    } else {
        explicit
    };
    let mut resolved = BTreeSet::new();
    let mut failures = Vec::new();
    for route in candidates {
        if route.imported == "*" && route.exported != "*" {
            failures.push("namespace re-export cannot be narrowed".into());
            continue;
        }
        let imported = if route.imported == "*" {
            name
        } else {
            route.imported.as_str()
        };
        // Star exports require proof that the target exposes this particular symbol.
        if route.exported == "*" && !exposes(modules, &route.to, imported, &mut BTreeSet::new(), 0)
        {
            continue;
        }
        match target(modules, &route.to, imported, depth + 1, visiting) {
            Ok(file) => {
                resolved.insert(file);
            }
            Err(reason) => failures.push(reason),
        }
    }
    visiting.remove(&key);
    if resolved.len() > 1 {
        return Err(format!("ambiguous star export {name} from {file}"));
    }
    if !failures.is_empty() {
        return Err(failures.remove(0));
    }
    resolved.into_iter().next().ok_or_else(|| {
        format!("named export {name} is unproven by {file}; export metadata may be incomplete")
    })
}

fn exposes(
    modules: &BTreeMap<&str, &BarrelModule>,
    file: &str,
    name: &str,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> bool {
    if depth > 10 || !seen.insert(file.to_owned()) {
        return false;
    }
    let Some(module) = modules.get(file) else {
        return false;
    };
    if module.declared.iter().any(|declared| declared == name)
        || module.exports.iter().any(|route| route.exported == name)
    {
        return true;
    }
    module
        .exports
        .iter()
        .filter(|route| route.exported == "*")
        .any(|route| exposes(modules, &route.to, name, &mut seen.clone(), depth + 1))
}

#[derive(Deserialize)]
struct Cruise {
    modules: Vec<CruiseModule>,
}
#[derive(Deserialize)]
struct CruiseModule {
    source: String,
    #[serde(default)]
    dependencies: Vec<CruiseDependency>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CruiseDependency {
    #[serde(default)]
    module: Option<String>,
    resolved: String,
    #[serde(default)]
    could_not_resolve: bool,
    #[serde(default, rename = "_archgraphLine")]
    line: Option<usize>,
    #[serde(default, rename = "_archgraphRequire")]
    require: Option<bool>,
}

/// Reads source only inside the scan root and trusts dependency-cruiser for target resolution.
pub fn typescript(root: &Path, json: &[u8], graph: &mut RawGraph) -> Result<Vec<String>> {
    let root = root.canonicalize()?;
    let cruise: Cruise = serde_json::from_slice(json)?;
    let files: BTreeSet<_> = graph.files.iter().cloned().collect();
    let mut modules = Vec::new();
    for module in cruise.modules {
        if !files.contains(&module.source) {
            continue;
        }
        let Ok(path) = root.join(&module.source).canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let statements = imports::typescript(&source);
        let syntax = imports::module_exports(&source);
        let mut metadata = BarrelModule {
            file: module.source.clone(),
            pure: syntax.pure,
            declared: syntax.declared,
            exports: Vec::new(),
            imports: Vec::new(),
        };
        let mut bindings = BTreeMap::new();
        for statement in statements {
            let Some(specifier) = statement.specifier.as_deref() else {
                continue;
            };
            let Some(dependency) = module.dependencies.iter().find(|dependency| {
                !dependency.could_not_resolve
                    && dependency.module.as_deref() == Some(specifier)
                    && dependency.line.is_none_or(|line| line == statement.line)
                    && dependency
                        .require
                        .is_none_or(|require| require == statement.require)
                    && files.contains(&dependency.resolved)
            }) else {
                continue;
            };
            if statement.reexport {
                for name in &statement.names {
                    metadata.exports.push(ExportRoute {
                        exported: name.local.clone(),
                        imported: name.imported.clone(),
                        to: dependency.resolved.clone(),
                    });
                }
            } else {
                for name in &statement.names {
                    bindings.insert(
                        name.local.clone(),
                        (name.imported.clone(), dependency.resolved.clone()),
                    );
                }
            }
            let mut groups = BTreeMap::new();
            for name in &statement.names {
                groups
                    .entry(name.type_only)
                    .or_insert_with(Vec::new)
                    .push(name.imported.clone());
            }
            if groups.is_empty() {
                groups.insert(false, Vec::new());
            }
            for (type_only, names) in groups {
                metadata.imports.push(ImportRoute {
                    to: dependency.resolved.clone(),
                    names,
                    wildcard: statement.wildcard,
                    kind: if type_only {
                        if statement.reexport {
                            EdgeKind::TypeReexport
                        } else {
                            EdgeKind::Type
                        }
                    } else {
                        statement.kind
                    },
                    specifier: statement.specifier.clone(),
                    line: statement.line,
                    side_effect: statement.side_effect,
                });
            }
        }
        for local in syntax.local {
            if let Some((imported, to)) = bindings.get(&local.imported) {
                metadata.exports.push(ExportRoute {
                    exported: local.local,
                    imported: imported.clone(),
                    to: to.clone(),
                });
            } else {
                metadata.declared.push(local.local);
            }
        }
        modules.push(metadata);
    }
    let warnings = resolve(graph, &modules);
    refine_types(&root, graph);
    Ok(warnings)
}

fn refine_types(root: &Path, graph: &mut RawGraph) {
    let mut sources = BTreeMap::new();
    for file in &graph.files {
        let Ok(path) = root.join(file).canonicalize() else {
            continue;
        };
        if !path.starts_with(root) {
            continue;
        }
        if let Ok(source) = std::fs::read_to_string(path) {
            sources.insert(file.clone(), source);
        }
    }
    let definitions: BTreeMap<_, _> = sources
        .iter()
        .map(|(file, source)| (file.as_str(), imports::type_only_exports(source)))
        .collect();
    for edge in &mut graph.edges {
        if !matches!(edge.kind, EdgeKind::Static | EdgeKind::Reexport) || edge.names.is_empty() {
            continue;
        }
        let Some(types) = definitions.get(edge.to.as_str()) else {
            continue;
        };
        if !edge.names.iter().all(|name| types.contains(name)) {
            continue;
        }
        let Some(source) = sources.get(&edge.from) else {
            continue;
        };
        let statement = imports::typescript(source).into_iter().find(|statement| {
            statement.specifier == edge.specifier && Some(statement.line) == edge.line
        });
        let Some(statement) = statement else {
            continue;
        };
        let names: Vec<_> = statement
            .names
            .iter()
            .filter(|name| edge.names.contains(&name.imported))
            .map(|name| name.local.clone())
            .collect();
        if statement.reexport
            || !names.is_empty() && imports::type_only_uses(source, &names).len() == names.len()
        {
            edge.kind = EdgeKind::Type;
            edge.weight = None;
        }
    }
    if let Some(records) = &mut graph.imports {
        for record in records {
            let edges: Vec<_> = graph
                .edges
                .iter()
                .filter(|edge| {
                    edge.from == record.from
                        && edge.specifier.as_deref() == Some(record.specifier.as_str())
                        && edge.line == record.line
                        && record
                            .targets
                            .iter()
                            .any(|target| target == edge.original_to.as_ref().unwrap_or(&edge.to))
                })
                .collect();
            if edges.is_empty() {
                continue;
            }
            record.targets = edges
                .iter()
                .map(|edge| edge.to.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if let Some(kind) = edges.iter().map(|edge| edge.kind).max() {
                record.kind = kind;
            }
        }
    }
}
