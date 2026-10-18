//! Elixir syntax and lexical module references, without starting BEAM or expanding project macros.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::collect::{RawGraph, excludes::Policy};
use crate::model::{Collector, Edge, EdgeKind, ImportReason, ImportRecord};
use crate::surface::{SymbolExport, SymbolImport, SymbolTarget, Symbols};

// Generated from the installed Elixir 1.20.1 / OTP 29 application manifests, without loading apps.
static APPLICATIONS: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    let apps: BTreeMap<String, Vec<String>> =
        serde_json::from_str(include_str!("elixir_apps.json"))
            .expect("bundled OTP application catalog");
    apps.into_iter()
        .flat_map(|(app, modules)| {
            modules
                .into_iter()
                .map(move |module| (module.trim_start_matches("Elixir.").to_owned(), app.clone()))
        })
        .collect()
});

#[derive(Clone, Default)]
struct Scope {
    module: Option<String>,
    aliases: BTreeMap<String, String>,
    kind: EdgeKind,
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
fn arguments(node: Node<'_>) -> Vec<Node<'_>> {
    children(node)
        .into_iter()
        .find(|n| n.kind() == "arguments")
        .map(children)
        .unwrap_or_default()
}
fn renamed<'tree>(args: &[Node<'tree>], source: &str) -> Option<Node<'tree>> {
    args.iter().find(|n| n.kind() == "keywords").and_then(|n| {
        children(*n).into_iter().find_map(|pair| {
            let key = pair.child_by_field_name("key")?;
            (text(key, source).trim().trim_end_matches(':') == "as")
                .then(|| pair.child_by_field_name("value"))
                .flatten()
        })
    })
}
fn module(node: Node<'_>, source: &str, scope: &Scope) -> Option<String> {
    match node.kind() {
        "alias" => {
            let raw = text(node, source);
            if let Some(raw) = raw.strip_prefix("Elixir.") {
                return Some(raw.to_owned());
            }
            let (first, rest) = raw.split_once('.').unwrap_or((raw, ""));
            let base = scope.aliases.get(first).map_or(first, String::as_str);
            Some(if rest.is_empty() {
                base.to_owned()
            } else {
                format!("{base}.{rest}")
            })
        }
        "identifier" if text(node, source) == "__MODULE__" => scope.module.clone(),
        "atom" => {
            let raw = text(node, source).strip_prefix(':')?;
            (!raw.contains(['\'', '"', '\\'])).then(|| raw.trim_start_matches("Elixir.").to_owned())
        }
        "dot" => {
            let left = module(node.child_by_field_name("left")?, source, scope)?;
            let right = node.child_by_field_name("right")?;
            (right.kind() == "alias").then(|| format!("{left}.{}", text(right, source)))
        }
        _ => None,
    }
}

struct Walker<'a> {
    file: &'a str,
    source: &'a str,
    modules: &'a mut BTreeMap<String, Vec<String>>,
    calls: BTreeSet<usize>,
    indexing: bool,
    records: Vec<ImportRecord>,
    exports: Vec<SymbolExport>,
    limitations: Vec<String>,
}
impl Walker<'_> {
    fn record(&mut self, node: Node<'_>, scope: &Scope) {
        if self.indexing {
            return;
        }
        let name = module(node, self.source, scope);
        let mut record = ImportRecord {
            from: self.file.to_owned(),
            specifier: name
                .clone()
                .unwrap_or_else(|| text(node, self.source).to_owned()),
            line: Some(node.start_position().row + 1),
            kind: scope.kind,
            ..Default::default()
        };
        match name {
            Some(name) if self.modules.contains_key(&name) => {
                record.targets = self.modules[&name].clone()
            }
            Some(name) if APPLICATIONS.contains_key(&name) => {
                record.external = Some(APPLICATIONS[&name].clone())
            }
            Some(_) => record.unresolved = Some(ImportReason::Missing),
            None => record.unresolved = Some(ImportReason::Dynamic),
        }
        self.records.push(record);
    }
    fn export(&mut self, name: String) {
        if !self.indexing {
            self.exports.push(SymbolExport {
                file: self.file.to_owned(),
                name: name.clone(),
                target: SymbolTarget {
                    file: self.file.to_owned(),
                    name,
                },
            });
        }
    }
    fn bind(&self, node: Node<'_>, alias: Option<Node<'_>>, scope: &mut Scope) {
        if let Some(name) = module(node, self.source, scope) {
            let binding = alias
                .map(|n| {
                    text(n, self.source)
                        .trim_start_matches("Elixir.")
                        .to_owned()
                })
                .unwrap_or_else(|| name.rsplit('.').next().unwrap_or(&name).to_owned());
            if !binding.is_empty() {
                scope.aliases.insert(binding, name);
            }
        }
    }
    fn walk(&mut self, node: Node<'_>, scope: &mut Scope) {
        if node.kind() == "call" && self.calls.contains(&node.start_byte()) {
            let target = node.child_by_field_name("target");
            let args = arguments(node);
            let name = target
                .filter(|n| n.kind() == "identifier")
                .map(|n| text(n, self.source));
            match name {
                Some("quote") => return,
                Some("defmodule" | "defprotocol") => {
                    let Some(first) = args.first().copied() else {
                        return;
                    };
                    let Some(mut name) = module(first, self.source, scope) else {
                        self.limitations.push(format!(
                            "{}:{}: dynamic module definition",
                            self.file,
                            node.start_position().row + 1
                        ));
                        return;
                    };
                    let raw = text(first, self.source);
                    let mut inner = scope.clone();
                    if first.kind() == "alias"
                        && !raw.starts_with("Elixir.")
                        && let Some(parent) = &scope.module
                    {
                        let prefix = raw.split('.').next().unwrap_or(raw);
                        if !scope.aliases.contains_key(prefix) {
                            name = format!("{parent}.{name}");
                            scope
                                .aliases
                                .insert(prefix.to_owned(), format!("{parent}.{prefix}"));
                            inner.aliases.clone_from(&scope.aliases);
                        }
                    }
                    if self.indexing {
                        let files = self.modules.entry(name.clone()).or_default();
                        if !files.iter().any(|p| p == self.file) {
                            files.push(self.file.to_owned());
                        }
                    }
                    self.export(name.clone());
                    inner.module = Some(name);
                    for child in children(node)
                        .into_iter()
                        .filter(|n| n.kind() == "do_block")
                    {
                        self.walk(child, &mut inner);
                    }
                    return;
                }
                Some("alias") => {
                    if let Some(first) = args.first().copied() {
                        let rename = renamed(&args, self.source);
                        if first.kind() == "dot"
                            && let (Some(prefix), Some(tuple)) = (
                                first.child_by_field_name("left"),
                                first.child_by_field_name("right"),
                            )
                            && tuple.kind() == "tuple"
                        {
                            if let Some(prefix) = module(prefix, self.source, scope) {
                                for suffix in children(tuple) {
                                    if suffix.kind() == "alias" {
                                        scope.aliases.insert(
                                            text(suffix, self.source).to_owned(),
                                            format!("{prefix}.{}", text(suffix, self.source)),
                                        );
                                    }
                                }
                            }
                        } else {
                            self.bind(first, rename, scope);
                        }
                    }
                    return;
                }
                Some("use" | "require" | "import") => {
                    if let Some(first) = args.first() {
                        self.record(*first, scope);
                    }
                    if name == Some("require")
                        && let (Some(first), Some(alias)) =
                            (args.first(), renamed(&args, self.source))
                    {
                        self.bind(*first, Some(alias), scope);
                    }
                    for argument in args.iter().skip(1) {
                        self.walk(*argument, &mut scope.clone());
                    }
                    return;
                }
                Some("def" | "defp" | "defmacro" | "defmacrop") => {
                    if matches!(name, Some("def" | "defmacro"))
                        && let (Some(module), Some(head)) = (&scope.module, args.first())
                    {
                        let function = if head.kind() == "call" {
                            head.child_by_field_name("target")
                                .map(|n| text(n, self.source))
                        } else if head.kind() == "identifier" {
                            Some(text(*head, self.source))
                        } else {
                            None
                        };
                        if let Some(function) = function {
                            self.export(format!("{module}.{function}"));
                        }
                    }
                    let mut inner = scope.clone();
                    for child in children(node)
                        .into_iter()
                        .filter(|n| n.kind() != "identifier")
                    {
                        self.walk(child, &mut inner);
                    }
                    return;
                }
                _ => {}
            }
            if let Some(dot) = target.filter(|n| n.kind() == "dot")
                && let Some(receiver) = dot.child_by_field_name("left")
            {
                let function = dot
                    .child_by_field_name("right")
                    .map(|n| text(n, self.source));
                if module(receiver, self.source, scope).as_deref() == Some("Module")
                    && matches!(function, Some("concat" | "safe_concat"))
                {
                    self.record(node, scope);
                    for arg in args {
                        self.concat_argument(arg, scope);
                    }
                } else {
                    self.record(receiver, scope);
                    for arg in args {
                        self.walk(arg, &mut scope.clone());
                    }
                }
                return;
            }
        }
        if node.kind() == "unary_operator"
            && text(node, self.source).starts_with('@')
            && let Some(operand) = node.child_by_field_name("operand")
        {
            let name = operand
                .child_by_field_name("target")
                .map(|n| text(n, self.source));
            if matches!(
                name,
                Some("spec" | "type" | "typep" | "opaque" | "callback" | "macrocallback")
            ) {
                let mut typed = scope.clone();
                typed.kind = EdgeKind::Type;
                self.walk(operand, &mut typed);
                return;
            }
            if name == Some("behaviour") {
                if let Some(first) = arguments(operand).first() {
                    self.record(*first, scope);
                }
                return;
            }
        }
        if node.kind() == "struct" {
            if let Some(first) = children(node).first() {
                self.record(*first, scope);
            }
            return;
        }
        if node.kind() == "alias" {
            if let Some(name) = module(node, self.source, scope)
                && (scope.kind == EdgeKind::Type
                    || self.modules.contains_key(&name)
                    || APPLICATIONS.contains_key(&name))
            {
                self.record(node, scope);
            }
            return;
        }
        let scoped = !matches!(
            node.kind(),
            "source" | "do_block" | "arguments" | "keywords" | "pair"
        );
        let mut inner = scope.clone();
        for child in children(node) {
            self.walk(child, if scoped { &mut inner } else { scope });
        }
    }
    fn concat_argument(&mut self, node: Node<'_>, scope: &Scope) {
        if node.kind() == "alias"
            || node.kind() == "atom"
            || text(node, self.source) == "__MODULE__"
        {
            return;
        }
        if matches!(node.kind(), "list" | "tuple") {
            for child in children(node) {
                self.concat_argument(child, scope);
            }
        } else {
            self.walk(node, &mut scope.clone());
        }
    }
}

pub fn collect(root: &Path, policy: &Policy) -> Result<super::Output> {
    let root = root.canonicalize()?;
    let all = super::source_files(&root, &["ex", "exs"])?;
    let files = policy.select(&all)?;
    let language = tree_sitter_elixir::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let query = Query::new(&language, include_str!("queries/elixir.scm"))?;
    let mut trees = Vec::new();
    for file in &files {
        let source = fs::read_to_string(root.join(file))
            .with_context(|| format!("reading Elixir {file}"))?;
        let tree = parser.parse(&source, None).context("parsing Elixir")?;
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut calls = BTreeSet::new();
        while let Some(found) = matches.next() {
            for capture in found.captures() {
                if capture.node.kind() == "call" {
                    calls.insert(capture.node.start_byte());
                }
            }
        }
        drop(matches);
        trees.push((file, source, tree, calls));
    }
    let mut modules = BTreeMap::new();
    for (file, source, tree, calls) in &trees {
        Walker {
            file,
            source,
            modules: &mut modules,
            calls: calls.clone(),
            indexing: true,
            records: Vec::new(),
            exports: Vec::new(),
            limitations: Vec::new(),
        }
        .walk(tree.root_node(), &mut Scope::default());
    }
    let mut records = Vec::new();
    let mut symbols = Symbols { complete: false, limitations: vec!["Elixir macros, dynamic dispatch and generated definitions are not expanded; the bundled OTP application catalog is from Elixir 1.20.1 / OTP 29.".into()], ..Default::default() };
    for (file, source, tree, calls) in &trees {
        let mut walker = Walker {
            file,
            source,
            modules: &mut modules,
            calls: calls.clone(),
            indexing: false,
            records: Vec::new(),
            exports: Vec::new(),
            limitations: Vec::new(),
        };
        if tree.root_node().has_error() {
            symbols.limitations.push(format!(
                "{file}: tree-sitter syntax errors; valid captures retained"
            ));
        }
        walker.walk(tree.root_node(), &mut Scope::default());
        records.extend(walker.records);
        symbols.exports.extend(walker.exports);
        symbols.limitations.extend(walker.limitations);
    }
    let mut edges = BTreeMap::new();
    for record in &records {
        for target in &record.targets {
            if record.from != *target {
                edges
                    .entry((record.from.clone(), target.clone(), record.kind))
                    .or_insert_with(|| Edge {
                        from: record.from.clone(),
                        to: target.clone(),
                        kind: record.kind,
                        line: record.line,
                        specifier: Some(record.specifier.clone()),
                        ..Default::default()
                    });
                symbols.imports.push(SymbolImport {
                    from: record.from.clone(),
                    to: target.clone(),
                    name: record.specifier.clone(),
                    target: Some(SymbolTarget {
                        file: target.clone(),
                        name: record.specifier.clone(),
                    }),
                    wildcard: false,
                });
            }
        }
    }
    symbols.exports.sort();
    symbols.exports.dedup();
    let unresolved = records.iter().filter(|r| r.unresolved.is_some()).count();
    Ok(super::Output {
        source_root: ".".into(),
        symbols,
        raw: RawGraph {
            files: all,
            edges: edges.into_values().collect(),
            imports: Some(records),
            metadata: Default::default(),
            collector: Collector {
                name: "tree-sitter-elixir".into(),
                version: Some("0.3.5".into()),
                cruised: files.len(),
                unresolved: Some(unresolved),
                type_marking: None,
                main_packages: Vec::new(),
                bound_files: Vec::new(),
                go_modules: Vec::new(),
                limitations: Vec::new(),
            },
        },
    })
}
