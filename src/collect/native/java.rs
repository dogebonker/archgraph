//! Java syntax resolution over contained source files, without a JDK or project execution.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::collect::{RawGraph, excludes::Policy};
use crate::model::{Collector, Edge, ImportReason, ImportRecord};
use crate::surface::{SymbolImport, SymbolTarget, Symbols};

use super::{named_children, self_export, text, unresolved_count};

#[derive(Clone)]
struct Import {
    name: String,
    wildcard: bool,
    static_member: bool,
    line: usize,
}
#[derive(Default)]
struct Parsed {
    package: String,
    declarations: Vec<(String, String, bool)>,
    members: Vec<String>,
    public_heads: Vec<String>,
    imports: Vec<Import>,
    references: BTreeSet<(String, usize, bool)>,
    parameters: BTreeSet<String>,
    limitations: Vec<String>,
}

type Index = BTreeMap<String, BTreeSet<String>>;

pub fn collect(root: &Path, policy: &Policy) -> Result<super::Output> {
    let root = root.canonicalize().context("resolving native Java root")?;
    let inventory = super::source_files(&root, &["java"])?;
    let files = policy.select(&inventory)?;
    ensure!(!files.is_empty(), "no Java source files after excludes");
    let included: BTreeSet<_> = files.iter().cloned().collect();
    let language = tree_sitter_java::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let query = Query::new(&language, include_str!("queries/java.scm"))?;
    let mut parsed = BTreeMap::new();
    let mut index = Index::new();
    for file in &inventory {
        let source = fs::read_to_string(root.join(file))
            .with_context(|| format!("reading Java source {file}"))?;
        let tree = parser.parse(&source, None).context("parsing Java source")?;
        let info = parse(&source, tree.root_node(), &query);
        for (name, _, _) in &info.declarations {
            index.entry(name.clone()).or_default().insert(file.clone());
        }
        for member in &info.members {
            index
                .entry(member.clone())
                .or_default()
                .insert(file.clone());
        }
        parsed.insert(file.clone(), info);
    }
    // Excluded copies (generated sources, build output) must not make included types ambiguous.
    for candidates in index.values_mut() {
        if candidates.len() > 1 && candidates.iter().any(|file| included.contains(file)) {
            candidates.retain(|file| included.contains(file));
        }
    }
    let mut raw = RawGraph {
        files: inventory,
        edges: Vec::new(),
        metadata: Default::default(),
        imports: Some(Vec::new()),
        globals: None,
        collector: Collector {
            name: "tree-sitter-java".into(),
            version: Some("0.23.5".into()),
            cruised: files.len(),
            ..Default::default()
        },
    };
    let mut symbols = Symbols { complete: false, limitations: vec!["Java syntax evidence does not replace javac type attribution: inherited members, overloads, classpath libraries and reflection may be incomplete.".into()], ..Symbols::default() };
    let mut edge_keys = BTreeSet::new();
    let mut symbol_keys = BTreeSet::new();
    for file in &files {
        let info = &parsed[file];
        symbols
            .limitations
            .extend(info.limitations.iter().map(|s| format!("{file}: {s}")));
        for (_, name, public) in &info.declarations {
            if *public {
                symbols.exports.push(self_export(file, name));
            }
        }
        for name in &info.public_heads {
            symbols.exports.push(self_export(file, name));
        }
        for import in &info.imports {
            let mut record = ImportRecord {
                from: file.clone(),
                specifier: if import.wildcard {
                    format!("{}.*", import.name)
                } else {
                    import.name.clone()
                },
                line: Some(import.line),
                ..ImportRecord::default()
            };
            let target_name = &import.name;
            let candidates = if import.wildcard && !import.static_member {
                index
                    .iter()
                    .filter(|(name, _)| {
                        name.rsplit_once('.')
                            .is_some_and(|(package, _)| package == import.name)
                    })
                    .flat_map(|(_, files)| files.iter().cloned())
                    .collect()
            } else {
                lookup_exact(target_name, &index)
            };
            if !candidates.is_empty()
                && (import.wildcard && !import.static_member || candidates.len() == 1)
            {
                // The import alone gives no edge: the references below add one where the name is used.
                record.targets = candidates.into_iter().collect();
            } else if candidates.len() > 1 {
                record.unresolved = Some(ImportReason::Alias);
            } else if let Some(package) = jdk_package(target_name) {
                record.external = Some(package.into());
            } else {
                record.unresolved = Some(ImportReason::Missing);
            }
            raw.imports.as_mut().unwrap().push(record);
        }
        for (name, line, definite_type) in &info.references {
            if info.parameters.contains(name) {
                continue;
            }
            let candidates = resolve(name, info, &index);
            if candidates.len() == 1 {
                let target = candidates.iter().next().unwrap();
                add_edge(
                    file,
                    target,
                    name,
                    *line,
                    &included,
                    &mut raw.edges,
                    &mut edge_keys,
                    &mut symbols.imports,
                    &mut symbol_keys,
                );
            } else if *definite_type && candidates.len() > 1 {
                raw.imports.as_mut().unwrap().push(ImportRecord {
                    from: file.clone(),
                    specifier: name.clone(),
                    line: Some(*line),
                    unresolved: Some(ImportReason::Alias),
                    ..ImportRecord::default()
                });
            }
        }
    }
    raw.edges
        .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    symbols.exports.sort();
    symbols.exports.dedup();
    symbols.limitations.sort();
    symbols.limitations.dedup();
    raw.collector.unresolved = Some(unresolved_count(raw.imports.as_ref().unwrap()));
    Ok(super::Output {
        raw,
        symbols,
        source_root: ".".into(),
    })
}

fn lookup_exact(name: &str, index: &Index) -> BTreeSet<String> {
    index.get(name).cloned().unwrap_or_default()
}
fn resolve(name: &str, info: &Parsed, index: &Index) -> BTreeSet<String> {
    let exact = lookup_exact(name, index);
    if !exact.is_empty() {
        return exact;
    }
    let (head, suffix) = name
        .split_once('.')
        .map_or((name, ""), |(head, tail)| (head, tail));
    let append = |base: &str| {
        if suffix.is_empty() {
            base.into()
        } else {
            format!("{base}.{suffix}")
        }
    };
    let explicit: BTreeSet<_> = info
        .imports
        .iter()
        .filter(|i| !i.wildcard && i.name.rsplit('.').next() == Some(head))
        .flat_map(|i| {
            let full = if i.static_member {
                i.name.clone()
            } else {
                append(&i.name)
            };
            lookup_exact(&full, index)
        })
        .collect();
    if !explicit.is_empty() {
        return explicit;
    }
    let local = if info.package.is_empty() {
        name.into()
    } else {
        format!("{}.{name}", info.package)
    };
    let local = lookup_exact(&local, index);
    if !local.is_empty() {
        return local;
    }
    info.imports
        .iter()
        .filter(|i| i.wildcard)
        .flat_map(|i| lookup_exact(&format!("{}.{name}", i.name), index))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn add_edge(
    from: &str,
    to: &str,
    name: &str,
    line: usize,
    included: &BTreeSet<String>,
    edges: &mut Vec<Edge>,
    edge_keys: &mut BTreeSet<(String, String)>,
    imports: &mut Vec<SymbolImport>,
    symbol_keys: &mut BTreeSet<(String, String, String)>,
) {
    if from == to || !included.contains(to) {
        return;
    }
    if edge_keys.insert((from.into(), to.into())) {
        edges.push(Edge {
            from: from.into(),
            to: to.into(),
            specifier: Some(name.into()),
            line: Some(line),
            ..Edge::default()
        });
    }
    if symbol_keys.insert((from.into(), to.into(), name.into())) {
        imports.push(SymbolImport {
            from: from.into(),
            to: to.into(),
            name: name.into(),
            target: Some(SymbolTarget {
                file: to.into(),
                name: name.rsplit('.').next().unwrap_or(name).into(),
            }),
            wildcard: false,
        });
    }
}
fn is_type(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "class_declaration"
            | "interface_declaration"
            | "record_declaration"
            | "enum_declaration"
            | "annotation_type_declaration"
    )
}
fn in_header(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "package_declaration" | "import_declaration") {
            return true;
        }
        node = parent;
    }
    false
}
fn globally_named(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "method_declaration"
                | "constructor_declaration"
                | "lambda_expression"
                | "object_creation_expression"
                | "block"
        ) {
            return false;
        }
        node = parent;
    }
    true
}
fn type_name(node: Node<'_>, package: &str, source: &str) -> String {
    let mut parts = Vec::new();
    let mut ancestor = Some(node);
    while let Some(node) = ancestor {
        if is_type(node)
            && let Some(name) = node.child_by_field_name("name")
        {
            parts.push(text(name, source));
        }
        ancestor = node.parent();
    }
    parts.reverse();
    if package.is_empty() {
        parts.join(".")
    } else {
        format!("{package}.{}", parts.join("."))
    }
}
fn is_public(node: Node<'_>) -> bool {
    has_modifier(node, "public")
}
fn has_modifier(node: Node<'_>, modifier: &str) -> bool {
    named_children(node)
        .into_iter()
        .find(|n| n.kind() == "modifiers")
        .is_some_and(|n| {
            let mut cursor = n.walk();
            n.children(&mut cursor).any(|n| n.kind() == modifier)
        })
}
fn parse(source: &str, root: Node<'_>, query: &Query) -> Parsed {
    let mut parsed = Parsed::default();
    if root.has_error() {
        parsed
            .limitations
            .push("syntax errors: valid tree-sitter captures retained".into());
    }
    if let Some(package) = named_children(root)
        .into_iter()
        .find(|n| n.kind() == "package_declaration")
    {
        parsed.package = named_children(package)
            .into_iter()
            .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))
            .map(|n| text(n, source).into())
            .unwrap_or_default();
    }
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, root, source.as_bytes());
    while let Some((matched, capture_index)) = captures.next() {
        let capture = matched.captures()[*capture_index];
        let node = capture.node;
        match query.capture_names()[capture.index as usize] {
            "import" => {
                let name = named_children(node)
                    .into_iter()
                    .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))
                    .map(|n| text(n, source))
                    .unwrap_or("");
                let wildcard = named_children(node).iter().any(|n| n.kind() == "asterisk");
                let mut walk = node.walk();
                let static_member = node.children(&mut walk).any(|n| n.kind() == "static");
                parsed.imports.push(Import {
                    name: name.into(),
                    wildcard,
                    static_member,
                    line: node.start_position().row + 1,
                });
            }
            "declaration" if globally_named(node) => {
                if let Some(name) = node.child_by_field_name("name") {
                    parsed.declarations.push((
                        type_name(node, &parsed.package, source),
                        text(name, source).into(),
                        is_public(node),
                    ));
                }
            }
            "parameter" => {
                parsed.parameters.insert(text(node, source).into());
            }
            "type" if !in_header(node) => {
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == "scoped_type_identifier")
                {
                    continue;
                }
                parsed.references.insert((
                    text(node, source).into(),
                    node.start_position().row + 1,
                    true,
                ));
            }
            "receiver" if !in_header(node) => {
                let written = text(node, source);
                if written
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
                {
                    parsed.references.insert((
                        written.into(),
                        node.start_position().row + 1,
                        false,
                    ));
                }
            }
            "identifier" | "member" if !in_header(node) => {
                let name = text(node, source);
                if parsed.imports.iter().any(|i| {
                    i.static_member && (i.wildcard || i.name.rsplit('.').next() == Some(name))
                }) {
                    parsed
                        .references
                        .insert((name.into(), node.start_position().row + 1, false));
                }
            }
            "annotation" if !in_header(node) => {
                parsed.references.insert((
                    text(node, source).into(),
                    node.start_position().row + 1,
                    true,
                ));
            }
            "method.declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if is_public(node) {
                        parsed.public_heads.push(text(name, source).into());
                    }
                    if has_modifier(node, "static") {
                        parsed.members.push(format!(
                            "{}.{}",
                            type_name(node, &parsed.package, source),
                            text(name, source)
                        ));
                    }
                }
            }
            "field.declaration" => {
                let static_member = has_modifier(node, "static")
                    || node.parent().is_some_and(|p| p.kind() == "interface_body");
                for declarator in named_children(node)
                    .into_iter()
                    .filter(|n| n.kind() == "variable_declarator")
                {
                    if let Some(name) = declarator.child_by_field_name("name") {
                        if is_public(node) {
                            parsed.public_heads.push(text(name, source).into());
                        }
                        if static_member {
                            parsed.members.push(format!(
                                "{}.{}",
                                type_name(node, &parsed.package, source),
                                text(name, source)
                            ));
                        }
                    }
                }
            }
            "enum.declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    parsed.members.push(format!(
                        "{}.{}",
                        type_name(node, &parsed.package, source),
                        text(name, source)
                    ));
                }
            }
            "module" => {
                parsed.limitations.push(
                    "module-info requires/exports directives are not source-file import edges"
                        .into(),
                );
            }
            _ => {}
        }
    }
    parsed
}
fn jdk_package(name: &str) -> Option<&str> {
    let package = name
        .split('.')
        .take_while(|part| part.chars().next().is_some_and(char::is_lowercase))
        .collect::<Vec<_>>()
        .join(".");
    // JDK package identities only; javax and arbitrary classpath libraries are not guessed.
    const PACKAGES: &[&str] = &[
        "java.lang",
        "java.lang.annotation",
        "java.lang.invoke",
        "java.lang.ref",
        "java.lang.reflect",
        "java.io",
        "java.math",
        "java.net",
        "java.net.http",
        "java.nio",
        "java.nio.channels",
        "java.nio.charset",
        "java.nio.file",
        "java.nio.file.attribute",
        "java.security",
        "java.security.cert",
        "java.sql",
        "java.text",
        "java.time",
        "java.time.chrono",
        "java.time.format",
        "java.time.temporal",
        "java.time.zone",
        "java.util",
        "java.util.concurrent",
        "java.util.concurrent.atomic",
        "java.util.concurrent.locks",
        "java.util.function",
        "java.util.jar",
        "java.util.logging",
        "java.util.regex",
        "java.util.stream",
        "java.util.zip",
    ];
    PACKAGES.iter().copied().find(|known| *known == package)
}
