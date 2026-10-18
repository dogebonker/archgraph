//! Source-only Rust module evidence. Macros, cfg evaluation and Cargo build scripts never run.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::collect::{
    RawGraph,
    barrels::{self, BarrelModule, ExportRoute, ImportRoute},
    excludes::Policy,
};
use crate::model::{Collector, Edge, EdgeKind, ImportReason, ImportRecord};
use crate::surface::{SymbolImport, SymbolTarget, Symbols};

use super::{named_children, self_export, text, unresolved_count};

fn squash(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn visible(node: Node<'_>, source: &str) -> bool {
    named_children(node)
        .iter()
        .any(|n| n.kind() == "visibility_modifier" && text(*n, source) == "pub")
}
fn context(mut node: Node<'_>, source: &str) -> Vec<String> {
    let mut path = Vec::new();
    while let Some(parent) = node.parent() {
        if parent.kind() == "mod_item"
            && parent.child_by_field_name("body").is_some()
            && let Some(name) = parent.child_by_field_name("name")
        {
            path.push(text(name, source).into());
        }
        node = parent;
    }
    path.reverse();
    path
}
fn parts(node: Node<'_>, source: &str) -> Vec<String> {
    if matches!(node.kind(), "scoped_identifier" | "scoped_type_identifier") {
        let mut out = node
            .child_by_field_name("path")
            .map(|n| parts(n, source))
            .unwrap_or_default();
        if let Some(name) = node.child_by_field_name("name") {
            out.push(text(name, source).into());
        }
        out
    } else {
        vec![text(node, source).into()]
    }
}
fn use_paths(
    node: Node<'_>,
    source: &str,
    prefix: &[String],
    out: &mut Vec<(Vec<String>, String, bool)>,
) {
    match node.kind() {
        "use_list" => {
            for child in named_children(node) {
                use_paths(child, source, prefix, out);
            }
        }
        "scoped_use_list" => {
            let mut p = prefix.to_vec();
            if let Some(path) = node.child_by_field_name("path") {
                p.extend(parts(path, source));
            }
            if let Some(list) = node.child_by_field_name("list") {
                use_paths(list, source, &p, out);
            }
        }
        "use_as_clause" => {
            if let (Some(path), Some(alias)) = (
                node.child_by_field_name("path"),
                node.child_by_field_name("alias"),
            ) {
                let mut p = prefix.to_vec();
                p.extend(parts(path, source));
                out.push((p, text(alias, source).into(), false));
            }
        }
        "use_wildcard" => {
            let mut p = prefix.to_vec();
            if let Some(path) = named_children(node).first() {
                p.extend(parts(*path, source));
            }
            out.push((p, "*".into(), true));
        }
        _ => {
            let mut p = prefix.to_vec();
            let local = text(node, source);
            if local != "self" || p.is_empty() {
                p.extend(parts(node, source));
            }
            let name = p.last().cloned().unwrap_or_default();
            out.push((p, name, false));
        }
    }
}
#[derive(Clone)]
struct Request {
    path: Vec<String>,
    local: String,
    wildcard: bool,
    line: usize,
    context: Vec<String>,
    public: bool,
    written: String,
}
struct ModuleDecl {
    name: String,
    context: Vec<String>,
    path: Option<String>,
    inline: bool,
    line: usize,
}
struct Parsed {
    requests: Vec<Request>,
    modules: Vec<ModuleDecl>,
    public: BTreeSet<String>,
    definitions: BTreeSet<String>,
    pure: bool,
    limitations: Vec<String>,
    includes: Vec<(usize, String)>,
}
fn parse(file: &str, source: &str) -> Result<Parsed> {
    let language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("Rust tree-sitter parse cancelled")?;
    static QUERY: std::sync::OnceLock<Query> = std::sync::OnceLock::new();
    let query = QUERY.get_or_init(|| {
        Query::new(&language, include_str!("queries/rust.scm")).expect("bundled Rust query")
    });
    let mut result = Parsed {
        requests: Vec::new(),
        modules: Vec::new(),
        public: BTreeSet::new(),
        definitions: BTreeSet::new(),
        pure: true,
        limitations: Vec::new(),
        includes: Vec::new(),
    };
    if tree.root_node().has_error() {
        result.limitations.push(format!(
            "{file}: syntax errors; valid Rust captures retained"
        ));
    }
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, tree.root_node(), source.as_bytes());
    while let Some((matched, index)) = captures.next() {
        let capture = matched.captures()[*index];
        let node = capture.node;
        match query.capture_names()[capture.index as usize] {
            "use" => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    let mut paths = Vec::new();
                    use_paths(argument, source, &[], &mut paths);
                    for (path, local, wildcard) in paths {
                        result.requests.push(Request {
                            path,
                            local,
                            wildcard,
                            line: node.start_position().row + 1,
                            context: context(node, source),
                            public: visible(node, source),
                            written: squash(text(argument, source)),
                        });
                    }
                }
            }
            "module" => {
                let Some(name) = node.child_by_field_name("name") else {
                    continue;
                };
                let mut path = None;
                let mut sibling = node.prev_named_sibling();
                while let Some(attribute) = sibling.filter(|n| {
                    matches!(
                        n.kind(),
                        "attribute_item" | "line_comment" | "block_comment"
                    )
                }) {
                    if text(attribute, source).starts_with("#[path") {
                        let mut stack = vec![attribute];
                        while let Some(n) = stack.pop() {
                            if n.kind() == "string_literal" {
                                let raw = text(n, source);
                                path = raw
                                    .get(1..raw.len() - 1)
                                    .filter(|s| !s.contains('\\'))
                                    .map(str::to_owned);
                                break;
                            }
                            stack.extend(named_children(n));
                        }
                        break;
                    }
                    sibling = attribute.prev_named_sibling();
                }
                result.modules.push(ModuleDecl {
                    name: text(name, source).into(),
                    context: context(node, source),
                    path,
                    inline: node.child_by_field_name("body").is_some(),
                    line: node.start_position().row + 1,
                });
            }
            "extern" => {
                if let Some(name) = node.child_by_field_name("name") {
                    result.requests.push(Request {
                        path: vec![text(name, source).into()],
                        local: node
                            .child_by_field_name("alias")
                            .map(|n| text(n, source))
                            .unwrap_or(text(name, source))
                            .into(),
                        wildcard: false,
                        line: node.start_position().row + 1,
                        context: context(node, source),
                        public: visible(node, source),
                        written: squash(text(node, source)),
                    });
                }
            }
            "definition" => {
                if context(node, source).is_empty()
                    && let Some(name) = node.child_by_field_name("name")
                {
                    result.definitions.insert(text(name, source).into());
                    if visible(node, source) {
                        result.public.insert(text(name, source).into());
                    }
                }
                result.pure = false;
            }
            "reference" => {
                let mut parent = node.parent();
                let mut skip = false;
                while let Some(p) = parent {
                    if matches!(
                        p.kind(),
                        "use_declaration" | "scoped_identifier" | "scoped_type_identifier"
                    ) {
                        skip = true;
                        break;
                    }
                    parent = p.parent();
                }
                if !skip {
                    let path = parts(node, source);
                    if path
                        .first()
                        .is_some_and(|p| matches!(p.as_str(), "crate" | "self" | "super"))
                    {
                        result.requests.push(Request {
                            local: path.last().cloned().unwrap_or_default(),
                            path,
                            wildcard: false,
                            line: node.start_position().row + 1,
                            context: context(node, source),
                            public: false,
                            written: squash(text(node, source)),
                        });
                    }
                }
            }
            "macro" => {
                let raw = text(node, source);
                if raw.starts_with("include!")
                    || raw.starts_with("include_str!")
                    || raw.starts_with("include_bytes!")
                {
                    result
                        .includes
                        .push((node.start_position().row + 1, squash(raw)));
                }
                result.pure = false;
                result.limitations.push(format!(
                    "{file}:{}: macro expansion is not evaluated",
                    node.start_position().row + 1
                ));
            }
            _ => {}
        }
    }
    Ok(result)
}
#[derive(Clone)]
struct Package {
    dir: PathBuf,
    name: String,
    root: String,
    deps: BTreeMap<String, String>,
}
fn dependencies(
    value: &toml::Value,
    workspace: Option<&toml::Value>,
    out: &mut BTreeMap<String, String>,
) {
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = value.get(key).and_then(toml::Value::as_table) {
            for (alias, entry) in table {
                let entry = if entry.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                    workspace.and_then(|w| w.get(alias)).unwrap_or(entry)
                } else {
                    entry
                };
                let name = entry
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(alias);
                out.insert(alias.replace('-', "_"), name.into());
            }
        }
    }
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for value in targets.values() {
            dependencies(value, workspace, out);
        }
    }
}
fn manifests(root: &Path, files: &BTreeSet<String>, policy: &Policy) -> Result<Vec<Package>> {
    let mut values = BTreeMap::new();
    let manifests: Vec<_> = super::source_files(root, &["toml"])?
        .into_iter()
        .filter(|file| {
            Path::new(file)
                .file_name()
                .is_some_and(|n| n == "Cargo.toml")
        })
        .collect();
    // Packaged and vendored copies must not shadow the crates they duplicate.
    for file in policy.select(&manifests)? {
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(root.join(&file))?)
            .with_context(|| format!("reading Rust manifest {file}"))?;
        values.insert(Path::new(&file).parent().unwrap().to_path_buf(), value);
    }
    let workspace = values
        .get(Path::new(""))
        .and_then(|v| v.get("workspace"))
        .and_then(|v| v.get("dependencies"));
    let mut packages = Vec::new();
    for (dir, value) in &values {
        let Some(name) = value
            .get("package")
            .and_then(|v| v.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        let configured = value
            .get("lib")
            .and_then(|v| v.get("path"))
            .and_then(toml::Value::as_str);
        let mut candidates = vec![configured.unwrap_or("src/lib.rs"), "src/main.rs"];
        if let Some(bins) = value.get("bin").and_then(toml::Value::as_array) {
            candidates.extend(
                bins.iter()
                    .filter_map(|b| b.get("path").and_then(toml::Value::as_str)),
            );
        }
        let entry = candidates
            .iter()
            .map(|p| dir.join(p).to_string_lossy().replace('\\', "/"))
            .find(|p| files.contains(p));
        let Some(entry) = entry else {
            continue;
        };
        let mut deps = BTreeMap::new();
        dependencies(value, workspace, &mut deps);
        packages.push(Package {
            dir: dir.clone(),
            name: name.replace('-', "_"),
            root: entry,
            deps,
        });
    }
    packages.sort_by_key(|p| std::cmp::Reverse(p.dir.components().count()));
    Ok(packages)
}
fn safe_relative(root: &Path, path: &Path) -> Option<String> {
    let path = path.canonicalize().ok()?;
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}
fn module_file(
    root: &Path,
    file: &str,
    crate_root: bool,
    declaration: &ModuleDecl,
    known: &BTreeSet<String>,
) -> Option<String> {
    let path = Path::new(file);
    let parent = path.parent()?;
    let mut directory = parent.to_path_buf();
    if !crate_root
        && !matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("lib.rs" | "main.rs" | "mod.rs")
        )
    {
        directory.push(path.file_stem()?);
    }
    for part in &declaration.context {
        directory.push(part);
    }
    let mut candidates = Vec::new();
    if let Some(custom) = &declaration.path {
        candidates.push(parent.join(custom));
    } else {
        candidates.push(directory.join(format!("{}.rs", declaration.name)));
        candidates.push(directory.join(&declaration.name).join("mod.rs"));
    }
    candidates
        .into_iter()
        .filter_map(|p| safe_relative(root, &root.join(p)))
        .find(|p| known.contains(p))
}

fn resolve(
    path: &[String],
    package: usize,
    base: &[String],
    packages: &[Package],
    modules: &BTreeMap<(usize, Vec<String>), String>,
    parsed: &BTreeMap<String, Parsed>,
) -> (Option<String>, Option<String>, String) {
    if path.is_empty() {
        return (None, None, String::new());
    }
    let mut pkg = package;
    let mut components = path.to_vec();
    let mut location = Vec::new();
    match components.first().map(String::as_str) {
        Some("crate") => {
            components.remove(0);
        }
        Some("self") => {
            components.remove(0);
            location = base.to_vec();
        }
        Some("super") => {
            location = base.to_vec();
            while components.first().is_some_and(|p| p == "super") {
                components.remove(0);
                if location.pop().is_none() {
                    return (None, None, String::new());
                }
            }
        }
        Some("std" | "core" | "alloc") => {
            return (
                None,
                Some(components[0].clone()),
                components.last().cloned().unwrap_or_default(),
            );
        }
        Some(first) => {
            let mut child = base.to_vec();
            child.push(first.to_owned());
            if modules.contains_key(&(package, child)) {
                // Rust 2018 paths may start with a child module of the current module.
                location = base.to_vec();
            } else if let Some(dependency) = packages[package].deps.get(first) {
                if let Some(index) = packages
                    .iter()
                    .position(|p| p.name == dependency.replace('-', "_"))
                {
                    pkg = index;
                    components.remove(0);
                } else {
                    return (
                        None,
                        Some(dependency.clone()),
                        components.last().cloned().unwrap_or_default(),
                    );
                }
            } else if first == packages[package].name {
                components.remove(0);
            }
        }
        None => {}
    }
    location.extend(components);
    let name = location.last().cloned().unwrap_or_else(|| "*".into());
    for length in (0..=location.len()).rev() {
        if length == 0
            && pkg == package
            && !path
                .first()
                .is_some_and(|p| matches!(p.as_str(), "crate" | "self" | "super"))
        {
            continue;
        }
        if let Some(file) = modules.get(&(pkg, location[..length].to_vec())) {
            if location.len() - length > 1
                && !parsed
                    .get(file)
                    .is_some_and(|syntax| syntax.definitions.contains(&location[length]))
            {
                continue;
            }
            return (
                Some(file.clone()),
                None,
                location.get(length).cloned().unwrap_or(name),
            );
        }
    }
    (None, None, name)
}

pub fn collect(root: &Path, policy: &Policy) -> Result<super::Output> {
    let root = root.canonicalize().context("resolving native Rust root")?;
    let files = super::source_files(&root, &["rs"])?;
    let known: BTreeSet<_> = files.iter().cloned().collect();
    let selected: BTreeSet<_> = policy.select(&files)?.into_iter().collect();
    let packages = manifests(&root, &known, policy)?;
    if packages.is_empty() {
        bail!(
            "native Rust needs a Cargo.toml package with an existing src/lib.rs, src/main.rs or [lib].path"
        );
    }
    let mut parsed = BTreeMap::new();
    let mut pending: Vec<_> = selected.iter().cloned().collect();
    for package in &packages {
        pending.push(package.root.clone());
    }
    while let Some(file) = pending.pop() {
        if parsed.contains_key(&file) {
            continue;
        }
        let source = std::fs::read_to_string(root.join(&file))
            .with_context(|| format!("reading Rust source {file}"))?;
        let syntax = parse(&file, &source)?;
        for declaration in &syntax.modules {
            if !declaration.inline
                && let Some(target) = module_file(
                    &root,
                    &file,
                    packages.iter().any(|p| p.root == file),
                    declaration,
                    &known,
                )
            {
                pending.push(target);
            }
        }
        parsed.insert(file, syntax);
    }
    let mut modules = BTreeMap::new();
    let mut contexts = BTreeMap::new();
    let mut queue = Vec::new();
    for (index, package) in packages.iter().enumerate() {
        modules.insert((index, Vec::new()), package.root.clone());
        contexts.insert(package.root.clone(), (index, Vec::new()));
        queue.push(package.root.clone());
    }
    let mut seen = BTreeSet::new();
    while let Some(file) = queue.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        let Some(syntax) = parsed.get(&file) else {
            continue;
        };
        let (package, base) = contexts[&file].clone();
        for declaration in &syntax.modules {
            let mut path = base.clone();
            path.extend(declaration.context.clone());
            path.push(declaration.name.clone());
            if declaration.inline {
                modules.insert((package, path), file.clone());
            } else if let Some(target) = module_file(
                &root,
                &file,
                packages[package].root == file,
                declaration,
                &known,
            ) {
                modules.insert((package, path.clone()), target.clone());
                contexts.entry(target.clone()).or_insert((package, path));
                queue.push(target);
            }
        }
    }
    let mut graph = RawGraph {
        files,
        edges: Vec::new(),
        imports: Some(Vec::new()),
        metadata: BTreeMap::new(),
        collector: Collector {
            name: "tree-sitter-rust".into(),
            version: Some("tree-sitter 0.27.1".into()),
            cruised: parsed.len(),
            go_modules: packages
                .iter()
                .map(|p| {
                    if p.dir.as_os_str().is_empty() {
                        ".".into()
                    } else {
                        p.dir.to_string_lossy().replace('\\', "/")
                    }
                })
                .collect(),
            ..Default::default()
        },
    };
    let mut symbols = Symbols {
        complete: false,
        ..Default::default()
    };
    let mut barrels = Vec::new();
    for (file, syntax) in &parsed {
        symbols.limitations.extend(syntax.limitations.clone());
        for (line, specifier) in &syntax.includes {
            graph.imports.as_mut().unwrap().push(ImportRecord {
                from: file.clone(),
                specifier: specifier.clone(),
                line: Some(*line),
                kind: EdgeKind::Static,
                unresolved: Some(ImportReason::Native),
                ..Default::default()
            });
        }
        let fallback = packages
            .iter()
            .enumerate()
            .find(|(_, p)| Path::new(file).starts_with(&p.dir))
            .map(|(i, _)| (i, Vec::new()));
        let Some((package, base)) = contexts.get(file).cloned().or(fallback) else {
            continue;
        };
        let mut module = BarrelModule {
            file: file.clone(),
            pure: syntax.pure,
            declared: syntax.definitions.iter().cloned().collect(),
            exports: Vec::new(),
            imports: Vec::new(),
        };
        for name in &syntax.public {
            symbols.exports.push(self_export(file, name));
        }
        for request in &syntax.requests {
            let mut current = base.clone();
            current.extend(request.context.clone());
            let (target, external, mut name) = resolve(
                &request.path,
                package,
                &current,
                &packages,
                &modules,
                &parsed,
            );
            if request.wildcard {
                name = "*".into();
            }
            let mut record = ImportRecord {
                from: file.clone(),
                specifier: request.written.clone(),
                line: Some(request.line),
                kind: EdgeKind::Static,
                external,
                ..Default::default()
            };
            if let Some(target) = target {
                record.targets.push(target.clone());
                if target != *file {
                    let kind = if request.public {
                        EdgeKind::Reexport
                    } else {
                        EdgeKind::Static
                    };
                    graph.edges.push(Edge {
                        from: file.clone(),
                        to: target.clone(),
                        kind,
                        specifier: Some(request.written.clone()),
                        line: Some(request.line),
                        names: vec![name.clone()],
                        ..Default::default()
                    });
                    module.imports.push(ImportRoute {
                        to: target.clone(),
                        names: vec![name.clone()],
                        wildcard: request.wildcard,
                        kind,
                        specifier: Some(request.written.clone()),
                        line: request.line,
                        side_effect: false,
                    });
                    if request.public {
                        module.exports.push(ExportRoute {
                            to: target.clone(),
                            exported: request.local.clone(),
                            imported: name.clone(),
                        });
                    }
                    symbols.imports.push(SymbolImport {
                        from: file.clone(),
                        to: target.clone(),
                        name: name.clone(),
                        target: (!request.wildcard).then_some(SymbolTarget { file: target, name }),
                        wildcard: request.wildcard,
                    });
                }
            } else if record.external.is_none() {
                record.unresolved = Some(ImportReason::Missing);
            }
            graph.imports.as_mut().unwrap().push(record);
        }
        for declaration in &syntax.modules {
            if declaration.inline {
                continue;
            }
            let target = module_file(
                &root,
                file,
                packages[package].root == *file,
                declaration,
                &known,
            );
            let mut record = ImportRecord {
                from: file.clone(),
                specifier: format!("mod {}", declaration.name),
                line: Some(declaration.line),
                kind: EdgeKind::Static,
                ..Default::default()
            };
            if let Some(target) = target {
                record.targets.push(target.clone());
                graph.edges.push(Edge {
                    from: file.clone(),
                    to: target,
                    kind: EdgeKind::Static,
                    specifier: Some(record.specifier.clone()),
                    line: record.line,
                    ..Default::default()
                });
            } else {
                record.unresolved = Some(ImportReason::Missing);
            }
            graph.imports.as_mut().unwrap().push(record);
        }
        barrels.push(module);
    }
    symbols
        .limitations
        .extend(barrels::resolve(&mut graph, &barrels));
    barrels::forward_exports(&mut symbols.exports, &barrels, false);
    barrels::import_targets(&mut symbols.imports, &symbols.exports);
    barrels::retarget_records(&mut graph, false);
    symbols.limitations.push("Rust syntax only: cfg branches are combined; macros, include!, generated code, trait resolution and visibility semantics are not evaluated. Runtime graph kinds retain Rust imports conservatively.".into());
    symbols.limitations.sort();
    symbols.limitations.dedup();
    graph.collector.unresolved = Some(unresolved_count(graph.imports.as_ref().unwrap()));
    Ok(super::Output {
        raw: graph,
        symbols,
        source_root: ".".into(),
    })
}
