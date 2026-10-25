//! Syntax-level Go references with package identities supplied by read-only `go list` metadata.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::collect::{RawGraph, excludes::Policy};
use crate::model::{
    Collector, Confidence, Edge, EdgeKind, ImportReason, ImportRecord, NodeMetadata,
};
use crate::surface::{SymbolImport, SymbolTarget, Symbols};

use super::{named_children, self_export, text, unresolved_count};

#[path = "go_app.rs"]
mod app;
pub(crate) use app::app_facts;

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Package {
    #[serde(default)]
    dir: String,
    #[serde(default)]
    import_path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    go_files: Vec<String>,
    #[serde(default)]
    ignored_go_files: Vec<String>,
    #[serde(default)]
    test_go_files: Vec<String>,
    #[serde(default)]
    x_test_go_files: Vec<String>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Clone)]
struct Import {
    path: String,
    alias: Option<String>,
    line: usize,
}
#[derive(Clone, Default)]
struct Parsed {
    package_name: String,
    imports: Vec<Import>,
    declarations: BTreeSet<String>,
    identifiers: Vec<(String, usize)>,
    selectors: Vec<(String, String)>,
    shadowed: BTreeSet<String>,
    errors: bool,
}

pub fn collect(root: &Path, go: &str, policy: &Policy) -> Result<super::Output> {
    collect_cached(root, go, policy, &mut Cache::default())
}
#[derive(Default)]
pub(crate) struct Cache {
    parsed: super::ParseCache<Parsed>,
    packages: Option<BTreeMap<String, Package>>,
    warnings: Vec<String>,
    dirs: Vec<String>,
    dirty: Vec<(String, String)>,
}
impl Cache {
    pub(crate) fn invalidate(&mut self, files: &[String]) {
        self.dirty = files
            .iter()
            .filter_map(|file| {
                self.parsed
                    .entries
                    .get(file)
                    .map(|old| (file.clone(), old.package_name.clone()))
            })
            .collect();
        self.parsed.invalidate(files);
    }
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.parsed.parsed, self.parsed.reused)
    }
}
pub(crate) fn collect_cached(
    root: &Path,
    go: &str,
    policy: &Policy,
    cache: &mut Cache,
) -> Result<super::Output> {
    let root = root.canonicalize()?;
    let language = tree_sitter_go::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let query = Query::new(&language, include_str!("queries/go.scm"))?;
    for (file, previous_package) in std::mem::take(&mut cache.dirty) {
        let info = cache.parsed.get(&file, || {
            let source = fs::read_to_string(root.join(&file))?;
            parse(&source, &mut parser, &query)
        })?;
        if info.package_name != previous_package
            || cache.packages.as_ref().is_some_and(|packages| {
                info.imports
                    .iter()
                    .any(|import| !packages.contains_key(&import.path))
            })
        {
            cache.packages = None;
        }
    }
    if cache.packages.is_none() {
        let (dirs, warnings) =
            crate::collect::go::module_dirs(&root, &crate::collect::globset(policy.patterns())?)?;
        if dirs.is_empty() {
            bail!("native Go collection needs a go.mod with a module declaration");
        }
        let go = crate::collect::resolve_executable("go", go)?;
        let mut packages = BTreeMap::new();
        for dir in &dirs {
            let output = crate::collect::run(
                Command::new(&go)
                    .args(["list", "-e", "-deps", "-json", "./..."])
                    .current_dir(root.join(dir))
                    .envs(crate::collect::go::ENV.iter().copied())
                    .env("GOSUMDB", "off"),
            )
            .with_context(|| {
                format!(
                    "running {} list for native Go package metadata",
                    go.display()
                )
            })?;
            if !output.status.success() {
                bail!(
                    "native Go metadata failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            for package in
                serde_json::Deserializer::from_slice(&output.stdout).into_iter::<Package>()
            {
                let package = package.context("reading go list package metadata")?;
                if !package.import_path.is_empty() {
                    packages.insert(package.import_path.clone(), package);
                }
            }
        }
        cache.packages = Some(packages);
        cache.warnings = warnings;
        cache.dirs = dirs;
    }
    let dirs = cache.dirs.clone();
    let packages = cache.packages.as_ref().unwrap();
    let warnings = cache.warnings.clone();
    let mut file_packages = BTreeMap::new();
    for (identity, package) in packages {
        if package.dir.is_empty() {
            continue;
        }
        let Ok(package_dir) = fs::canonicalize(&package.dir) else {
            continue;
        };
        let Ok(dir) = package_dir.strip_prefix(&root) else {
            continue;
        };
        for file in package
            .go_files
            .iter()
            .chain(&package.ignored_go_files)
            .chain(&package.test_go_files)
            .chain(&package.x_test_go_files)
        {
            let path = dir.join(file).to_string_lossy().replace('\\', "/");
            // Do not follow a repository symlink out of the source tree.
            if root
                .join(&path)
                .canonicalize()
                .is_ok_and(|p| p.starts_with(&root))
            {
                file_packages.insert(path, identity.clone());
            }
        }
    }
    let all_files: Vec<_> = file_packages.keys().cloned().collect();
    let files = policy.select(&all_files)?;
    let selected: BTreeSet<_> = files.iter().cloned().collect();

    let mut parsed = BTreeMap::new();
    for file in &all_files {
        parsed.insert(
            file.clone(),
            cache.parsed.get(file, || {
                let source = fs::read_to_string(root.join(file))?;
                parse(&source, &mut parser, &query)
            })?,
        );
    }
    let mut declarations: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, data) in &parsed {
        let identity = &file_packages[file];
        if data.package_name == packages[identity].name {
            entries
                .entry(identity.clone())
                .or_default()
                .push(file.clone());
        }
        for name in &data.declarations {
            declarations
                .entry((identity.clone(), data.package_name.clone(), name.clone()))
                .or_default()
                .push(file.clone());
        }
    }
    for paths in entries.values_mut() {
        paths.sort_by_key(|p| (!p.ends_with("/doc.go") && p != "doc.go", p.clone()));
    }
    let mut symbols = Symbols {
        complete: false,
        limitations: vec![
            "Go syntax references do not resolve methods, field types, dot imports or build-variant semantics; native Go files stay package-bound for move safety".into(),
            "Go header-only generated markers (Code generated ... DO NOT EDIT.) are not detected; filename and directory exclusions still apply".into(),
        ],
        ..Symbols::default()
    };
    symbols.limitations.extend(warnings);
    let mut edges = BTreeMap::new();
    let mut imports = Vec::new();
    for file in &files {
        let data = &parsed[file];
        if data.errors {
            symbols
                .limitations
                .push(format!("{file}: tree-sitter reported incomplete Go syntax"));
        }
        let identity = &file_packages[file];
        for name in &data.declarations {
            if name.chars().next().is_some_and(char::is_uppercase) {
                symbols.exports.push(self_export(file, name));
            }
        }
        for import in &data.imports {
            let mut record = ImportRecord {
                from: file.clone(),
                specifier: import.path.clone(),
                line: Some(import.line),
                kind: EdgeKind::Static,
                ..ImportRecord::default()
            };
            let Some(package) = packages.get(&import.path) else {
                record.unresolved = Some(ImportReason::Missing);
                imports.push(record);
                continue;
            };
            let alias = import.alias.as_deref().unwrap_or(&package.name);
            if let Some(candidates) = entries.get(&import.path) {
                let mut targets = BTreeSet::new();
                if !data.shadowed.contains(alias) && alias != "." {
                    for (_, name) in data
                        .selectors
                        .iter()
                        .filter(|(qualifier, _)| qualifier == alias)
                    {
                        if let Some(definitions) = declarations.get(&(
                            import.path.clone(),
                            package.name.clone(),
                            name.clone(),
                        )) {
                            for target in definitions {
                                targets.insert(target.clone());
                                if selected.contains(target) {
                                    symbols.imports.push(SymbolImport {
                                        from: file.clone(),
                                        to: target.clone(),
                                        name: name.clone(),
                                        target: Some(SymbolTarget {
                                            file: target.clone(),
                                            name: name.clone(),
                                        }),
                                        wildcard: false,
                                    });
                                }
                            }
                        }
                    }
                }
                let fallback = targets.is_empty();
                if fallback && let Some(entry) = candidates.first() {
                    targets.insert(entry.clone());
                }
                record.targets = targets.into_iter().collect();
                for target in &record.targets {
                    if selected.contains(target) && target != file {
                        edges.entry((file.clone(), target.clone())).or_insert_with(|| Edge {
                            from: file.clone(), to: target.clone(), kind: EdgeKind::Static,
                            specifier: Some(import.path.clone()), line: Some(import.line),
                            confidence: Some(if fallback { Confidence::Low } else { Confidence::High }),
                            resolution_reason: fallback.then(|| "Go package-entry fallback without a proven selector declaration".into()),
                            ..Edge::default()
                        });
                    }
                }
            } else if !package.dir.is_empty() && package.error.is_none() {
                record.external = Some(import.path.clone());
            } else {
                record.unresolved = Some(ImportReason::Missing);
            }
            imports.push(record);
        }
        for (name, line) in &data.identifiers {
            if data.shadowed.contains(name) {
                continue;
            }
            if let Some(targets) =
                declarations.get(&(identity.clone(), data.package_name.clone(), name.clone()))
            {
                for target in targets
                    .iter()
                    .filter(|target| *target != file && selected.contains(*target))
                {
                    edges
                        .entry((file.clone(), target.clone()))
                        .or_insert_with(|| Edge {
                            from: file.clone(),
                            to: target.clone(),
                            kind: EdgeKind::Static,
                            line: Some(*line),
                            confidence: Some(Confidence::Low),
                            resolution_reason: Some(
                                "Go unqualified package declaration reference inferred from syntax"
                                    .into(),
                            ),
                            ..Edge::default()
                        });
                }
            }
        }
    }
    symbols.exports.sort();
    symbols.exports.dedup();
    symbols.limitations.sort();
    symbols.limitations.dedup();
    imports.sort_by(|a, b| (&a.from, a.line, &a.specifier).cmp(&(&b.from, b.line, &b.specifier)));
    let main_packages = files
        .iter()
        .filter(|f| parsed[*f].package_name == "main")
        .map(|f| {
            Path::new(f)
                .parent()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .map(|p| if p.is_empty() { ".".into() } else { p })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let unresolved = unresolved_count(&imports);
    Ok(super::Output {
        source_root: ".".into(),
        symbols,
        raw: RawGraph {
            metadata: files
                .iter()
                .map(|f| {
                    (
                        f.clone(),
                        NodeMetadata {
                            test: crate::collect::excludes::is_test(f),
                            barrel: false,
                        },
                    )
                })
                .collect(),
            collector: Collector {
                name: "tree-sitter-go".into(),
                version: Some("0.25.0".into()),
                cruised: all_files.len(),
                unresolved: Some(unresolved),
                main_packages,
                bound_files: files.clone(),
                go_modules: dirs,
                ..Default::default()
            },
            files: all_files,
            edges: edges.into_values().collect(),
            imports: Some(imports),
            globals: None,
        },
    })
}

fn top_level(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "block" | "function_declaration" | "method_declaration" | "func_literal"
        ) {
            return false;
        }
        node = parent;
    }
    true
}
fn parse(source: &str, parser: &mut Parser, query: &Query) -> Result<Parsed> {
    let tree = parser.parse(source, None).context("parsing Go source")?;
    let mut result = Parsed {
        errors: tree.root_node().has_error(),
        ..Parsed::default()
    };
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(found) = matches.next() {
        for capture in found.captures() {
            let node = capture.node;
            match query.capture_names()[capture.index as usize] {
                "package" => {
                    if let Some(name) = named_children(node).first() {
                        result.package_name = text(*name, source).into();
                    }
                }
                "import" => {
                    let Some(path) = node.child_by_field_name("path") else {
                        continue;
                    };
                    let raw = text(path, source);
                    let decoded = if raw.starts_with('`') {
                        Some(raw.trim_matches('`').to_owned())
                    } else {
                        serde_json::from_str::<String>(raw).ok()
                    };
                    if decoded.is_none() {
                        result.errors = true;
                    }
                    result.imports.push(Import {
                        path: decoded.unwrap_or_else(|| raw.trim_matches('"').to_owned()),
                        alias: node
                            .child_by_field_name("name")
                            .map(|n| text(n, source).to_owned()),
                        line: node.start_position().row + 1,
                    });
                }
                "function" | "type" if top_level(node) => {
                    if let Some(name) = node.child_by_field_name("name") {
                        result.declarations.insert(text(name, source).into());
                    }
                }
                "variable" => {
                    let names: Vec<_> = named_children(node)
                        .into_iter()
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| text(n, source).to_owned())
                        .collect();
                    if top_level(node) {
                        result.declarations.extend(names);
                    } else {
                        result.shadowed.extend(names);
                    }
                }
                "selector" | "qualified" => {
                    let (operand_field, name_field) = if node.kind() == "qualified_type" {
                        ("package", "name")
                    } else {
                        ("operand", "field")
                    };
                    if let (Some(operand), Some(field)) = (
                        node.child_by_field_name(operand_field),
                        node.child_by_field_name(name_field),
                    ) && matches!(operand.kind(), "identifier" | "package_identifier")
                    {
                        result
                            .selectors
                            .push((text(operand, source).into(), text(field, source).into()));
                    }
                }
                "binding" => {
                    let target = node.child_by_field_name("left").unwrap_or(node);
                    result.shadowed.extend(
                        named_children(target)
                            .into_iter()
                            .filter(|n| n.kind() == "identifier")
                            .map(|n| text(n, source).to_owned()),
                    );
                }
                "identifier" if node.parent().is_none_or(|p| p.kind() != "qualified_type") => {
                    result
                        .identifiers
                        .push((text(node, source).into(), node.start_position().row + 1))
                }
                _ => {}
            }
        }
    }
    Ok(result)
}
