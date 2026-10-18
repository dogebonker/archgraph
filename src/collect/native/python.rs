//! Python syntax and import evidence without importing or executing the scanned project.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::collect::barrels::{BarrelModule, ExportRoute, ImportRoute};
use crate::collect::{RawGraph, excludes::Policy};
use crate::model::{Collector, Edge, EdgeKind, ImportReason, ImportRecord};
use crate::surface::{SymbolExport, SymbolImport, SymbolTarget, Symbols};

use super::{named_children, self_export, text, unresolved_count};

#[derive(Clone)]
struct Import {
    module: String,
    names: Vec<(String, String)>,
    from: bool,
    wildcard: bool,
    kind: EdgeKind,
    line: usize,
    dynamic: bool,
    at_module: bool,
}
#[derive(Clone, Default)]
struct Parsed {
    imports: Vec<Import>,
    declarations: BTreeSet<String>,
    declaration_lines: BTreeMap<String, usize>,
    all: Option<BTreeSet<String>>,
    pure: bool,
    limitations: Vec<String>,
}

pub fn collect(
    root: &Path,
    source_dir: &str,
    package: Option<&str>,
    python_paths: &[PathBuf],
    policy: &Policy,
) -> Result<super::Output> {
    collect_cached(
        root,
        source_dir,
        package,
        python_paths,
        policy,
        &mut Cache::default(),
    )
}

#[derive(Default)]
pub(crate) struct Cache {
    parsed: super::ParseCache<Parsed>,
    external_roots: Option<BTreeSet<String>>,
    metadata_files: BTreeSet<PathBuf>,
}
impl Cache {
    pub(crate) fn invalidate(&mut self, files: &[String]) {
        self.parsed.invalidate(files);
    }
    pub(crate) fn metadata_files(&self) -> Vec<PathBuf> {
        self.metadata_files.iter().cloned().collect()
    }
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.parsed.parsed, self.parsed.reused)
    }
}
pub(crate) fn collect_cached(
    root: &Path,
    source_dir: &str,
    package: Option<&str>,
    python_paths: &[PathBuf],
    policy: &Policy,
    cache: &mut Cache,
) -> Result<super::Output> {
    let root = root
        .canonicalize()
        .context("resolving native Python root")?;
    let source = contained_directory(&root, &root.join(source_dir))?;
    let mut search = vec![source.clone()];
    for path in python_paths {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        let path = contained_directory(&root, &path).context("resolving --python-path")?;
        if !search.contains(&path) {
            search.push(path);
        }
    }
    for path in [root.join("src"), root.clone()] {
        if path.is_dir() && !search.contains(&path) {
            search.push(path);
        }
    }
    if cache.external_roots.is_none() {
        cache.external_roots = Some(installed_roots(&root, &search, &mut cache.metadata_files)?);
    }
    let external_roots = cache.external_roots.as_ref().unwrap();
    let all_files = super::source_files(&root, &["py"])?;
    let mut modules = BTreeMap::new();
    for base in search.iter().rev() {
        for file in &all_files {
            let absolute = root.join(file);
            let Ok(relative) = absolute.strip_prefix(base) else {
                continue;
            };
            let mut parts: Vec<_> = relative
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let name = parts.pop().unwrap_or_default();
            let stem = name.trim_end_matches(".py");
            if stem != "__init__" {
                parts.push(stem.into());
            }
            if !parts.is_empty() {
                modules.insert(parts.join("."), file.clone());
            }
        }
    }
    let selected: BTreeSet<_> = policy.select(&all_files)?.into_iter().collect();
    let package = crate::collect::grimp::package(
        &root,
        source_dir,
        package,
        &crate::collect::globset(policy.patterns())?,
    )?;
    let prefix = package.replace('.', "/");
    let mut files: Vec<_> = all_files
        .iter()
        .filter(|file| {
            let absolute = root.join(file);
            if let Ok(relative) = absolute.strip_prefix(&source) {
                let relative = relative.to_string_lossy();
                relative == format!("{prefix}.py") || relative.starts_with(&format!("{prefix}/"))
            } else {
                python_paths.iter().any(|p| {
                    absolute.starts_with(if p.is_absolute() {
                        p.clone()
                    } else {
                        root.join(p)
                    })
                })
            }
        })
        .cloned()
        .collect();
    if !files.iter().any(|file| selected.contains(file)) {
        bail!("no Python source files selected in {}", source.display());
    }
    let language = tree_sitter_python::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let query = Query::new(&language, include_str!("queries/python.scm"))?;
    let mut parsed = BTreeMap::new();
    let mut discovery_limits = Vec::new();
    let mut module_of: BTreeMap<String, String> = BTreeMap::new();
    for (name, file) in &modules {
        let current = module_of
            .entry(file.clone())
            .or_insert_with(|| name.clone());
        if name.split('.').count() < current.split('.').count() {
            *current = name.clone();
        }
    }
    // Match reference package discovery before filtering: excluded tests can reveal production modules.
    let mut file_set: BTreeSet<_> = files.iter().cloned().collect();
    let mut index = 0;
    while index < files.len() {
        let file = files[index].clone();
        index += 1;
        if !parsed.contains_key(&file) {
            match cache
                .parsed
                .get(&file, || parse_file(&root, &file, &mut parser, &query))
            {
                Ok(info) => {
                    parsed.insert(file.clone(), info);
                }
                Err(_) if !selected.contains(&file) => {
                    discovery_limits.push(format!(
                        "{file}: excluded source unavailable during import discovery"
                    ));
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        let imports = parsed[&file].imports.clone();
        for import in &imports {
            if import.dynamic {
                continue;
            }
            let Some(absolute) = absolute_module(
                &file,
                module_of.get(&file).map(String::as_str),
                &import.module,
            ) else {
                continue;
            };
            if let Some(base) = modules.get(&absolute)
                && selected.contains(base)
                && !parsed.contains_key(base)
            {
                parsed.insert(
                    base.clone(),
                    cache
                        .parsed
                        .get(base, || parse_file(&root, base, &mut parser, &query))?,
                );
            }
            let targets: Vec<_> = if import.from && !import.wildcard {
                import
                    .names
                    .iter()
                    .filter_map(|(name, _)| {
                        let base = modules.get(&absolute);
                        if has_attribute(base, &file, name, import.line, &parsed) {
                            base.cloned()
                        } else {
                            modules.get(&format!("{absolute}.{name}")).or(base).cloned()
                        }
                    })
                    .collect()
            } else {
                modules.get(&absolute).cloned().into_iter().collect()
            };
            for target in targets {
                if file_set.insert(target.clone()) {
                    files.push(target);
                }
            }
        }
    }
    files.sort();
    let inventory = files;
    let files: Vec<_> = inventory
        .iter()
        .filter(|file| selected.contains(*file))
        .cloned()
        .collect();
    let file_set: BTreeSet<_> = files.iter().cloned().collect();
    let mut graph = RawGraph {
        files: inventory,
        edges: Vec::new(),
        metadata: Default::default(),
        imports: Some(Vec::new()),
        collector: Collector {
            name: "tree-sitter-python".into(),
            version: Some("0.25.0".into()),
            cruised: files.len(),
            ..Default::default()
        },
    };
    let mut symbols = Symbols {
        complete: true,
        limitations: discovery_limits,
        ..Symbols::default()
    };
    let mut barrels = Vec::new();
    for file in &files {
        let info = &parsed[file];
        symbols
            .limitations
            .extend(info.limitations.iter().map(|s| format!("{file}: {s}")));
        let mut barrel = BarrelModule {
            file: file.clone(),
            pure: info.pure,
            declared: info
                .declarations
                .iter()
                .filter(|name| exported(info, name))
                .cloned()
                .collect(),
            exports: Vec::new(),
            imports: Vec::new(),
        };
        for name in &info.declarations {
            if exported(info, name) {
                symbols.exports.push(self_export(file, name));
            }
        }
        for import in &info.imports {
            let absolute = absolute_module(
                file,
                module_of.get(file).map(String::as_str),
                &import.module,
            );
            let mut record = ImportRecord {
                from: file.clone(),
                specifier: import.module.clone(),
                line: Some(import.line),
                kind: import.kind,
                ..ImportRecord::default()
            };
            if import.dynamic {
                record.unresolved = Some(ImportReason::Dynamic);
            } else if let Some(absolute) = absolute {
                let mut routes = Vec::new();
                if import.from {
                    for (name, alias) in &import.names {
                        let base = modules.get(&absolute);
                        let attribute = has_attribute(base, file, name, import.line, &parsed);
                        let submodule = modules.get(&format!("{absolute}.{name}"));
                        let target = if attribute { base } else { submodule.or(base) };
                        if let Some(target) = target {
                            let name = if submodule == Some(target) && !attribute {
                                "*"
                            } else {
                                name.as_str()
                            };
                            routes.push((
                                target.clone(),
                                name.to_owned(),
                                alias.clone(),
                                name == "*",
                            ));
                        }
                    }
                    if import.wildcard
                        && let Some(target) = modules.get(&absolute)
                    {
                        routes.push((target.clone(), "*".into(), "*".into(), true));
                    }
                } else if let Some(target) = modules.get(&absolute) {
                    routes.push((
                        target.clone(),
                        "*".into(),
                        import
                            .names
                            .first()
                            .map(|(_, a)| a.clone())
                            .unwrap_or_default(),
                        true,
                    ));
                }
                for (target, name, alias, wildcard) in routes {
                    record.targets.push(target.clone());
                    if !file_set.contains(&target) {
                        continue;
                    }
                    graph.edges.push(Edge {
                        from: file.clone(),
                        to: target.clone(),
                        kind: import.kind,
                        line: Some(import.line),
                        specifier: Some(import.module.clone()),
                        names: vec![name.clone()],
                        ..Edge::default()
                    });
                    barrel.imports.push(ImportRoute {
                        to: target.clone(),
                        names: vec![name.clone()],
                        wildcard,
                        kind: import.kind,
                        specifier: Some(import.module.clone()),
                        line: import.line,
                        side_effect: false,
                    });
                    if info.pure && import.at_module && (exported(info, &alias) || import.wildcard)
                    {
                        if import.wildcard {
                            barrel.exports.push(ExportRoute {
                                exported: "*".into(),
                                imported: "*".into(),
                                to: target.clone(),
                            });
                        } else {
                            barrel.exports.push(ExportRoute {
                                exported: alias.clone(),
                                imported: name.clone(),
                                to: target.clone(),
                            });
                        }
                    }
                    symbols.imports.push(SymbolImport {
                        from: file.clone(),
                        to: target.clone(),
                        name: name.clone(),
                        target: (!wildcard).then(|| SymbolTarget {
                            file: target.clone(),
                            name: name.clone(),
                        }),
                        wildcard,
                    });
                    if !wildcard
                        && import.at_module
                        && exported(info, &alias)
                        && import.kind != EdgeKind::Dynamic
                    {
                        symbols.exports.push(SymbolExport {
                            file: file.clone(),
                            name: alias,
                            target: SymbolTarget { file: target, name },
                        });
                    }
                }
                record.targets.sort();
                record.targets.dedup();
                if record.targets.is_empty() {
                    let namespace = if import.from && !import.wildcard {
                        import.names.iter().all(|(name, _)| {
                            namespace_exists(&format!("{absolute}.{name}"), &modules)
                        })
                    } else {
                        namespace_exists(&absolute, &modules)
                    };
                    if namespace {
                        record.namespace = Some(absolute);
                    } else if !import.module.starts_with('.')
                        && (stdlib(&absolute)
                            || external_roots
                                .contains(absolute.split('.').next().unwrap_or(&absolute)))
                    {
                        record.external =
                            Some(absolute.split('.').next().unwrap_or(&absolute).to_owned());
                    } else {
                        record.unresolved = Some(ImportReason::Missing);
                    }
                }
            } else {
                record.unresolved = Some(ImportReason::Missing);
            }
            graph.imports.as_mut().unwrap().push(record);
        }
        barrels.push(barrel);
    }
    symbols
        .limitations
        .extend(crate::collect::barrels::resolve(&mut graph, &barrels));
    // Resolve symbol targets with the same bounded forwarding evidence as the file graph.
    let exports: BTreeMap<_, _> = symbols
        .exports
        .iter()
        .map(|e| ((e.file.clone(), e.name.clone()), e.target.clone()))
        .collect();
    for export in &mut symbols.exports {
        export.target = defining(&exports, &export.target);
    }
    for import in &mut symbols.imports {
        if let Some(target) = &mut import.target {
            *target = defining(&exports, target);
        }
    }
    symbols.complete = symbols.limitations.is_empty();
    symbols.exports.sort();
    symbols.exports.dedup();
    graph.collector.unresolved = Some(unresolved_count(graph.imports.as_ref().unwrap()));
    Ok(super::Output {
        raw: graph,
        symbols,
        source_root: crate::modules::join(source_dir, &prefix),
    })
}

fn parse_file(root: &Path, file: &str, parser: &mut Parser, query: &Query) -> Result<Parsed> {
    let source = fs::read_to_string(root.join(file))
        .with_context(|| format!("reading Python source {file}"))?;
    let tree = parser
        .parse(&source, None)
        .context("parsing Python source")?;
    Ok(parse(&source, tree.root_node(), query))
}
fn has_attribute(
    base: Option<&String>,
    file: &str,
    name: &str,
    line: usize,
    parsed: &BTreeMap<String, Parsed>,
) -> bool {
    base.and_then(|base| parsed.get(base)).is_some_and(|p| {
        p.declarations.contains(name)
            && (base.map(String::as_str) != Some(file)
                || p.declaration_lines
                    .get(name)
                    .is_some_and(|declared| *declared < line))
            || p.imports.iter().any(|import| {
                import.at_module
                    && (base.map(String::as_str) != Some(file) || import.line < line)
                    && import.names.iter().any(|(_, alias)| alias == name)
            })
    })
}

fn contained_directory(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    if !path.starts_with(root) || !path.is_dir() {
        bail!(
            "Python search path must be a directory inside the scan root: {}",
            path.display()
        );
    }
    Ok(path)
}
fn import_name(node: Node<'_>, source: &str) -> (String, String) {
    if node.kind() == "aliased_import" {
        let name = node
            .child_by_field_name("name")
            .map(|n| text(n, source))
            .unwrap_or("");
        let alias = node
            .child_by_field_name("alias")
            .map(|n| text(n, source))
            .unwrap_or(name);
        (name.into(), alias.into())
    } else {
        let name = text(node, source);
        (name.into(), name.split('.').next().unwrap_or(name).into())
    }
}
fn parse(source: &str, root: Node<'_>, query: &Query) -> Parsed {
    let mut result = Parsed {
        pure: true,
        ..Parsed::default()
    };
    if root.has_error() {
        result
            .limitations
            .push("syntax errors: valid tree-sitter captures retained".into());
    }
    for node in named_children(root) {
        let pure = matches!(
            node.kind(),
            "import_statement" | "import_from_statement" | "future_import_statement" | "comment"
        ) || node.kind() == "expression_statement"
            && named_children(node).iter().all(|n| {
                n.kind() == "string"
                    || n.kind() == "assignment"
                        && n.child_by_field_name("left")
                            .is_some_and(|l| text(l, source) == "__all__")
            });
        result.pure &= pure;
    }
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, root, source.as_bytes());
    while let Some((capture_match, index)) = captures.next() {
        let capture = capture_match.captures()[*index];
        let node = capture.node;
        match query.capture_names()[capture.index as usize] {
            "import" | "from" | "future" => {
                let from = node.kind() != "import_statement";
                let module_node = node.child_by_field_name("module_name");
                let names: Vec<_> = named_children(node)
                    .into_iter()
                    .filter(|n| {
                        Some(*n) != module_node
                            && matches!(n.kind(), "dotted_name" | "aliased_import")
                    })
                    .map(|n| import_name(n, source))
                    .collect();
                let wildcard = named_children(node)
                    .iter()
                    .any(|n| n.kind() == "wildcard_import");
                let kind = import_kind(node, source, &result.imports);
                if wildcard {
                    result.limitations.push(
                        "star-import symbol exports cannot all be enumerated statically".into(),
                    );
                }
                if from {
                    result.imports.push(Import {
                        module: if node.kind() == "future_import_statement" {
                            "__future__".into()
                        } else {
                            module_node.map(|n| text(n, source)).unwrap_or("").into()
                        },
                        names,
                        from,
                        wildcard,
                        kind,
                        line: node.start_position().row + 1,
                        dynamic: false,
                        at_module: top_level(node),
                    });
                } else {
                    for (name, alias) in names {
                        result.imports.push(Import {
                            module: name.clone(),
                            names: vec![(name, alias)],
                            from,
                            wildcard: false,
                            kind,
                            line: node.start_position().row + 1,
                            dynamic: false,
                            at_module: top_level(node),
                        });
                    }
                }
            }
            "declaration" if node.parent().is_some_and(top_level) => {
                result.declarations.insert(text(node, source).into());
                result
                    .declaration_lines
                    .insert(text(node, source).into(), node.start_position().row + 1);
                if text(node, source) == "__getattr__" {
                    result
                        .limitations
                        .push("module __getattr__ may expose dynamic exports".into());
                }
            }
            "assignment" => {
                if !top_level(node) {
                    continue;
                }
                if let Some(left) = node.child_by_field_name("left") {
                    if text(left, source) == "__all__" {
                        let right = node.child_by_field_name("right");
                        let values = right.and_then(|n| string_list(n, source));
                        if values.is_none() {
                            result
                                .limitations
                                .push("dynamic __all__ cannot be enumerated".into());
                        }
                        if node.kind() == "augmented_assignment" {
                            if let (Some(existing), Some(values)) = (&mut result.all, values) {
                                existing.extend(values);
                            } else {
                                result.all = None;
                            }
                        } else {
                            result.all = values;
                        }
                    } else if left.kind() == "identifier" {
                        result.declarations.insert(text(left, source).into());
                        result
                            .declaration_lines
                            .insert(text(left, source).into(), node.start_position().row + 1);
                    }
                }
            }
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    continue;
                };
                let function = text(function, source);
                if function.starts_with("__all__.") {
                    result.all = None;
                    result
                        .limitations
                        .push("mutated __all__ cannot be enumerated".into());
                    continue;
                }
                if !import_call(node, root, source, &result.imports) {
                    continue;
                }
                let args = node
                    .child_by_field_name("arguments")
                    .map(named_children)
                    .unwrap_or_default();
                let module = args.first().and_then(|n| literal(*n, source));
                result.imports.push(Import {
                    module: module.clone().unwrap_or_else(|| "<dynamic>".into()),
                    names: Vec::new(),
                    from: false,
                    wildcard: false,
                    kind: import_kind(node, source, &result.imports),
                    line: node.start_position().row + 1,
                    dynamic: module.is_none(),
                    at_module: top_level(node),
                });
                result.limitations.push(
                    "dynamic import calls may be rebound; runtime execution is not modeled".into(),
                );
            }
            _ => {}
        }
    }
    result
}
/// Trust only stable lexical bindings to importlib/builtins; aliases may be shadowed.
fn import_call(call: Node<'_>, root: Node<'_>, source: &str, imports: &[Import]) -> bool {
    let Some(function) = call.child_by_field_name("function") else {
        return false;
    };
    let (name, module_call) = if function.kind() == "identifier" {
        (text(function, source), false)
    } else if function.kind() == "attribute"
        && function
            .child_by_field_name("attribute")
            .is_some_and(|n| text(n, source) == "import_module")
    {
        let Some(object) = function.child_by_field_name("object") else {
            return false;
        };
        if object.kind() != "identifier" {
            return false;
        }
        (text(object, source), true)
    } else {
        return false;
    };
    let trusted = imports.iter().any(|import| {
        import.line <= call.start_position().row + 1
            && (!import.from
                && module_call
                && import.module == "importlib"
                && import.names.iter().any(|(_, alias)| alias == name)
                || import.from
                    && !module_call
                    && import.names.iter().any(|(original, alias)| {
                        alias == name
                            && (import.module == "importlib" && original == "import_module"
                                || import.module == "builtins" && original == "__import__")
                    }))
    });
    if !trusted && (module_call || name != "__import__") {
        return false;
    }
    let mut scopes = BTreeSet::new();
    let mut ancestor = Some(call);
    while let Some(node) = ancestor {
        scopes.insert(node.id());
        ancestor = node.parent();
    }
    !binding_conflict(root, source, name, module_call, &scopes)
}
fn has_identifier(node: Node<'_>, source: &str, name: &str) -> bool {
    node.kind() == "identifier" && text(node, source) == name
        || named_children(node)
            .into_iter()
            .any(|n| has_identifier(n, source, name))
}
fn binding_conflict(
    node: Node<'_>,
    source: &str,
    name: &str,
    module_call: bool,
    scopes: &BTreeSet<usize>,
) -> bool {
    if matches!(
        node.kind(),
        "function_definition" | "class_definition" | "lambda"
    ) {
        if node
            .child_by_field_name("name")
            .is_some_and(|n| text(n, source) == name)
        {
            return true;
        }
        if !scopes.contains(&node.id()) {
            return false;
        }
        if node
            .child_by_field_name("parameters")
            .is_some_and(|n| has_identifier(n, source, name))
        {
            return true;
        }
    }
    if matches!(
        node.kind(),
        "assignment" | "augmented_assignment" | "for_statement" | "named_expression"
    ) && node
        .child_by_field_name("left")
        .or_else(|| node.child_by_field_name("name"))
        .is_some_and(|n| has_identifier(n, source, name))
    {
        return true;
    }
    if matches!(
        node.kind(),
        "global_statement" | "nonlocal_statement" | "delete_statement"
    ) && has_identifier(node, source, name)
    {
        return true;
    }
    if matches!(node.kind(), "import_statement" | "import_from_statement") {
        if named_children(node)
            .iter()
            .any(|n| n.kind() == "wildcard_import")
        {
            return true;
        }
        let module = node.child_by_field_name("module_name");
        for imported in named_children(node)
            .into_iter()
            .filter(|n| Some(*n) != module && matches!(n.kind(), "dotted_name" | "aliased_import"))
        {
            let (original, alias) = import_name(imported, source);
            if alias != name {
                continue;
            }
            let trusted = if node.kind() == "import_statement" {
                module_call && original == "importlib"
            } else {
                !module_call
                    && module.is_some_and(|n| {
                        matches!(
                            (text(n, source), original.as_str()),
                            ("importlib", "import_module") | ("builtins", "__import__")
                        )
                    })
            };
            if !trusted {
                return true;
            }
        }
    }
    named_children(node)
        .into_iter()
        .any(|child| binding_conflict(child, source, name, module_call, scopes))
}
fn top_level(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "function_definition" | "class_definition" | "lambda"
        ) {
            return false;
        }
        node = parent;
    }
    true
}
fn import_kind(mut node: Node<'_>, source: &str, imports: &[Import]) -> EdgeKind {
    let mut lazy = false;
    while let Some(parent) = node.parent() {
        if parent.kind() == "if_statement"
            && parent.child_by_field_name("consequence") == Some(node)
            && parent
                .child_by_field_name("condition")
                .is_some_and(|condition| {
                    matches!(
                        text(condition, source),
                        "TYPE_CHECKING" | "typing.TYPE_CHECKING"
                    ) || imports.iter().any(|import| {
                        import.module == "typing"
                            && import.names.iter().any(|(name, alias)| {
                                if import.from {
                                    name == "TYPE_CHECKING" && text(condition, source) == alias
                                } else {
                                    text(condition, source) == format!("{alias}.TYPE_CHECKING")
                                }
                            })
                    })
                })
        {
            return EdgeKind::Type;
        }
        lazy |= matches!(parent.kind(), "function_definition" | "lambda");
        node = parent;
    }
    if lazy {
        EdgeKind::Dynamic
    } else {
        EdgeKind::Static
    }
}
fn literal(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let children = named_children(node);
    if children
        .iter()
        .any(|n| n.kind() == "interpolation" || n.kind() == "escape_sequence")
    {
        return None;
    }
    children
        .iter()
        .find(|n| n.kind() == "string_content")
        .map(|n| text(*n, source).into())
        .or_else(|| Some(String::new()))
}
fn string_list(node: Node<'_>, source: &str) -> Option<BTreeSet<String>> {
    if !matches!(node.kind(), "list" | "tuple" | "set") {
        return None;
    }
    named_children(node)
        .into_iter()
        .map(|n| literal(n, source))
        .collect()
}
fn absolute_module(file: &str, module: Option<&str>, written: &str) -> Option<String> {
    let dots = written.chars().take_while(|c| *c == '.').count();
    if dots == 0 {
        return Some(written.into());
    }
    let mut base: Vec<_> = module?.split('.').collect();
    if !file.ends_with("/__init__.py") {
        base.pop();
    }
    if dots > base.len() {
        return None;
    }
    for _ in 1..dots {
        base.pop();
    }
    let suffix = &written[dots..];
    if !suffix.is_empty() {
        base.push(suffix);
    }
    Some(base.join("."))
}
fn namespace_exists(name: &str, modules: &BTreeMap<String, String>) -> bool {
    modules.keys().any(|m| m.starts_with(&format!("{name}.")))
}
fn exported(parsed: &Parsed, name: &str) -> bool {
    parsed
        .all
        .as_ref()
        .map_or(!name.starts_with('_'), |all| all.contains(name))
}
fn defining(
    exports: &BTreeMap<(String, String), SymbolTarget>,
    target: &SymbolTarget,
) -> SymbolTarget {
    let mut target = target.clone();
    let mut seen = BTreeSet::new();
    for _ in 0..10 {
        if !seen.insert(target.clone()) {
            break;
        }
        let Some(next) = exports.get(&(target.file.clone(), target.name.clone())) else {
            break;
        };
        if next == &target {
            break;
        }
        target = next.clone();
    }
    target
}

/// Installed metadata is evidence of import roots; distribution names alone are not.
fn installed_roots(
    root: &Path,
    search: &[PathBuf],
    metadata: &mut BTreeSet<PathBuf>,
) -> Result<BTreeSet<String>> {
    let mut paths = search.to_vec();
    for environment in [root.join(".venv"), root.join("venv")] {
        let lib = environment.join("lib");
        metadata.extend([environment, lib.clone()]);
        if !lib.is_dir() {
            continue;
        }
        let canonical = lib.canonicalize()?;
        if !canonical.starts_with(root) {
            continue;
        }
        for entry in fs::read_dir(canonical)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                paths.push(entry.path().join("site-packages"));
            }
        }
    }
    let mut found = BTreeSet::new();
    for path in paths {
        metadata.insert(path.clone());
        if !path.is_dir() {
            continue;
        }
        let path = path.canonicalize()?;
        if !path.starts_with(root) {
            continue;
        }
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir()
                || !entry.file_name().to_string_lossy().ends_with(".dist-info")
            {
                continue;
            }
            metadata.insert(entry.path());
            for filename in ["top_level.txt", "RECORD"] {
                let file = entry.path().join(filename);
                metadata.insert(file.clone());
                if !file.is_file() || fs::symlink_metadata(&file)?.file_type().is_symlink() {
                    continue;
                }
                let contents = fs::read_to_string(&file).with_context(|| {
                    format!("reading installed Python metadata {}", file.display())
                })?;
                if filename == "top_level.txt" {
                    found.extend(
                        contents
                            .lines()
                            .filter(|name| import_identifier(name))
                            .map(str::to_owned),
                    );
                } else if let Some(rows) = record_rows(&contents) {
                    for row in rows {
                        if row.len() == 3
                            && let Some(name) = installed_record_root(&path, &row[0], metadata)?
                        {
                            found.insert(name);
                        }
                    }
                }
            }
        }
    }
    Ok(found)
}

fn import_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn installed_record_root(
    site: &Path,
    written: &str,
    metadata: &mut BTreeSet<PathBuf>,
) -> Result<Option<String>> {
    // RECORD paths are slash-separated; installation scripts and .data payloads are not imports.
    let parts: Vec<_> = written.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains('\\'))
    {
        return Ok(None);
    }
    let leaf = parts.last().copied().unwrap_or_default();
    let Some(stem) = leaf.strip_suffix(".py").or_else(|| {
        leaf.strip_suffix(".so")
            .or_else(|| leaf.strip_suffix(".pyd"))
            .map(|stem| stem.split('.').next().unwrap_or(stem))
    }) else {
        return Ok(None);
    };
    if !import_identifier(stem)
        || parts[..parts.len() - 1]
            .iter()
            .any(|part| !import_identifier(part))
    {
        return Ok(None);
    }
    let file = site.join(written);
    metadata.insert(file.clone());
    if !file.is_file() {
        return Ok(None);
    }
    let actual = file.canonicalize()?;
    if !actual.starts_with(site) {
        return Ok(None);
    }
    metadata.insert(actual);
    Ok(Some(
        if parts.len() == 1 { stem } else { parts[0] }.to_owned(),
    ))
}

/// Strict CSV, including escaped quotes and quoted newlines; malformed metadata proves nothing.
fn record_rows(source: &str) -> Option<Vec<Vec<String>>> {
    let mut chars = source.chars().peekable();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() && !closed => quoted = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                    closed = false;
                }
                '\n' | '\r' => {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    closed = false;
                }
                '"' => return None,
                _ if closed => return None,
                _ => field.push(c),
            }
        }
    }
    if quoted {
        return None;
    }
    if closed || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Some(rows)
}

fn stdlib(module: &str) -> bool {
    // sys.stdlib_module_names from CPython 3.10, 3.12 and 3.13, plus modules removed since 3.8.
    let root = module.split('.').next().unwrap_or(module);
    include_str!("python_stdlib.txt")
        .lines()
        .any(|name| name == root)
}

#[cfg(test)]
mod tests {
    use super::record_rows;

    #[test]
    fn wheel_csv_quotes_crlf_newlines_and_invalid_rows() {
        assert_eq!(
            record_rows("\"a,b.py\",\"sha256=a\"\"b\",1\r\n\"pkg/module.py\",\"line\none\",2"),
            Some(vec![
                vec!["a,b.py".into(), "sha256=a\"b".into(), "1".into()],
                vec!["pkg/module.py".into(), "line\none".into(), "2".into()]
            ])
        );
        for invalid in ["\"unterminated,,", "\"closed\"x,,", "un\"quoted,,"] {
            assert!(record_rows(invalid).is_none(), "{invalid}");
        }
        assert_eq!(record_rows(""), Some(Vec::new()));
    }
}
