//! Query-driven ECMAScript syntax; oxc resolves module requests without executing the project.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::collect::{
    RawGraph,
    barrels::{self, BarrelModule, ExportRoute, ImportRoute},
    excludes::Policy,
    imports::{ImportStatement, ImportedName},
    typescript_resolution::package_name,
};
use crate::model::{Collector, Edge, EdgeKind, ImportReason, ImportRecord};
use crate::surface::{SymbolImport, Symbols};

use super::{named_children, self_export, text, unresolved_count};

fn literal(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let value = text(node, source);
    // Escaped module names are uncommon; do not misresolve an undecoded escape.
    let body = value.get(1..value.len().checked_sub(1)?)?;
    (!body.contains('\\')).then(|| body.to_owned())
}
fn has_token(node: Node<'_>, token: &str) -> bool {
    let mut c = node.walk();
    node.children(&mut c).any(|n| n.kind() == token)
}
fn name(node: Node<'_>, source: &str) -> String {
    literal(node, source).unwrap_or_else(|| text(node, source).into())
}
fn names(node: Node<'_>, source: &str, type_only: bool, exports: bool) -> Vec<ImportedName> {
    let mut out = Vec::new();
    for child in named_children(node) {
        match child.kind() {
            "import_specifier" | "export_specifier" => {
                if let Some(original) = child.child_by_field_name("name") {
                    let original = name(original, source);
                    let local = child
                        .child_by_field_name("alias")
                        .map(|n| name(n, source))
                        .unwrap_or_else(|| original.clone());
                    out.push(ImportedName {
                        imported: original,
                        local,
                        type_only: type_only || has_token(child, "type"),
                    });
                }
            }
            "namespace_import" | "namespace_export" => {
                let local = named_children(child)
                    .last()
                    .map(|n| name(*n, source))
                    .unwrap_or_else(|| "*".into());
                out.push(ImportedName {
                    imported: "*".into(),
                    local,
                    type_only,
                });
            }
            "identifier" if !exports => out.push(ImportedName {
                imported: "default".into(),
                local: name(child, source),
                type_only,
            }),
            "import_clause" | "named_imports" | "export_clause" => {
                out.extend(names(child, source, type_only, exports))
            }
            _ => {}
        }
    }
    out
}
fn inside_function(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "generator_function"
                | "generator_function_declaration"
        ) {
            return true;
        }
        node = parent;
    }
    false
}
fn binds_require(node: Node<'_>, source: &str) -> bool {
    if matches!(
        node.kind(),
        "variable_declarator"
            | "function_declaration"
            | "class_declaration"
            | "import_specifier"
            | "required_parameter"
            | "optional_parameter"
    ) && node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))
        .is_some_and(|n| text(n, source) == "require")
    {
        return true;
    }
    if node.kind() == "import_clause"
        && named_children(node)
            .iter()
            .any(|n| n.kind() == "identifier" && text(*n, source) == "require")
    {
        return true;
    }
    if matches!(
        node.kind(),
        "formal_parameters" | "object_pattern" | "array_pattern"
    ) && named_children(node)
        .iter()
        .any(|n| text(*n, source) == "require")
    {
        return true;
    }
    named_children(node)
        .into_iter()
        .any(|n| binds_require(n, source))
}

/// Explicit declarations only; structural implementations cannot be established from syntax.
#[derive(Clone, Default)]
pub(crate) struct ArchitectureFacts {
    pub interfaces: BTreeSet<String>,
    pub implementations: Vec<(String, String)>,
    pub imports: Vec<(String, String, String)>,
    pub incomplete: bool,
}

pub(crate) fn architecture(file: &str, source: &str) -> Result<ArchitectureFacts> {
    let parsed = parse(file, source)?;
    let mut facts = parsed.architecture;
    facts.incomplete |= !parsed.limitations.is_empty();
    for statement in parsed.statements.iter().filter(|s| !s.reexport) {
        if let Some(specifier) = &statement.specifier {
            for name in &statement.names {
                facts
                    .imports
                    .push((name.local.clone(), name.imported.clone(), specifier.clone()));
            }
        }
    }
    Ok(facts)
}

fn architecture_facts(root: Node<'_>, source: &str) -> ArchitectureFacts {
    let mut facts = ArchitectureFacts {
        incomplete: root.has_error(),
        ..Default::default()
    };
    for child in named_children(root) {
        let declaration = if child.kind() == "export_statement" {
            child.child_by_field_name("declaration").unwrap_or(child)
        } else {
            child
        };
        if declaration.kind() == "interface_declaration" {
            if let Some(name) = declaration.child_by_field_name("name") {
                facts.interfaces.insert(text(name, source).into());
            }
        } else if declaration.kind() == "class_declaration" {
            // Generic parameters can shadow imported interface names; leave their binding unknown.
            if declaration.child_by_field_name("type_parameters").is_some() {
                facts.incomplete = true;
                continue;
            }
            let Some(class) = declaration.child_by_field_name("name") else {
                facts.incomplete = true;
                continue;
            };
            for heritage in named_children(declaration)
                .into_iter()
                .filter(|n| n.kind() == "class_heritage")
            {
                for clause in named_children(heritage) {
                    if clause.kind() != "implements_clause" {
                        facts.incomplete = true;
                        continue;
                    }
                    for target in named_children(clause) {
                        let target = if target.kind() == "generic_type" {
                            target.child_by_field_name("name").unwrap_or(target)
                        } else {
                            target
                        };
                        if target.kind() == "type_identifier" {
                            facts
                                .implementations
                                .push((text(class, source).into(), text(target, source).into()));
                        } else {
                            facts.incomplete = true;
                        }
                    }
                }
            }
        }
    }
    facts
}

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn generic_type_parameters_do_not_become_interface_implementation_evidence() {
        let facts = super::architecture(
            "a.ts",
            "export interface Port {} export class Fake<Port> implements Port {}",
        )
        .unwrap();
        assert!(facts.interfaces.contains("Port"));
        assert!(facts.implementations.is_empty());
        assert!(facts.incomplete);
    }
}

#[derive(Clone, Default)]
struct Parsed {
    statements: Vec<ImportStatement>,
    declared: Vec<String>,
    local: Vec<ImportedName>,
    pure: bool,
    limitations: Vec<String>,
    type_declarations: BTreeSet<String>,
    architecture: ArchitectureFacts,
    type_uses: BTreeSet<String>,
    value_uses: BTreeSet<String>,
}
fn parse(file: &str, source: &str) -> Result<Parsed> {
    let javascript = matches!(
        Path::new(file).extension().and_then(|v| v.to_str()),
        Some("js" | "jsx" | "mjs" | "cjs")
    );
    let language: tree_sitter::Language = if javascript {
        tree_sitter_javascript::LANGUAGE.into()
    } else if file.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("tree-sitter cancelled parsing")?;
    let root = tree.root_node();
    static JS_QUERY: std::sync::OnceLock<Query> = std::sync::OnceLock::new();
    static TS_QUERY: std::sync::OnceLock<Query> = std::sync::OnceLock::new();
    static TSX_QUERY: std::sync::OnceLock<Query> = std::sync::OnceLock::new();
    let query = if javascript {
        JS_QUERY.get_or_init(|| {
            Query::new(&language, include_str!("queries/javascript.scm"))
                .expect("bundled JavaScript query")
        })
    } else if file.ends_with(".tsx") {
        TSX_QUERY.get_or_init(|| {
            Query::new(&language, include_str!("queries/typescript.scm"))
                .expect("bundled TSX query")
        })
    } else {
        TS_QUERY.get_or_init(|| {
            Query::new(&language, include_str!("queries/typescript.scm"))
                .expect("bundled TypeScript query")
        })
    };
    let mut parsed = Parsed {
        statements: Vec::new(),
        declared: Vec::new(),
        local: Vec::new(),
        pure: true,
        limitations: Vec::new(),
        type_declarations: BTreeSet::new(),
        architecture: architecture_facts(root, source),
        type_uses: BTreeSet::new(),
        value_uses: BTreeSet::new(),
    };
    if root.has_error() {
        parsed.limitations.push(format!(
            "{file}: tree-sitter reported syntax errors; valid import captures retained"
        ));
    }
    // shortcut: any lexical require binding suppresses require calls in the file; use scope binding analysis when this loses useful edges.
    let shadowed = binds_require(root, source);
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, root, source.as_bytes());
    while let Some((m, i)) = captures.next() {
        let node = m.captures()[*i].node;
        match query.capture_names()[m.captures()[*i].index as usize] {
            "import" | "export" => {
                let export = node.kind() == "export_statement";
                let require_clause = named_children(node)
                    .into_iter()
                    .find(|n| n.kind() == "import_require_clause");
                let require = require_clause.is_some();
                let source_node = node
                    .child_by_field_name("source")
                    .or_else(|| require_clause?.child_by_field_name("source"));
                // The pinned grammar wraps `type` in ERROR for TS 5 `export type *`.
                let type_only = has_token(node, "type")
                    || export
                        && has_token(node, "*")
                        && named_children(node).iter().any(|child| {
                            child.kind() == "ERROR"
                                && child.child_count() == 1
                                && child.child(0).is_some_and(|token| token.kind() == "type")
                        });
                let imported = names(node, source, type_only, export);
                let type_only =
                    type_only || !imported.is_empty() && imported.iter().all(|name| name.type_only);
                if export && source_node.is_none() {
                    if let Some(declaration) = node.child_by_field_name("declaration") {
                        if has_token(node, "default") {
                            parsed.declared.push("default".into());
                        } else if let Some(n) = declaration.child_by_field_name("name") {
                            parsed.declared.push(name(n, source));
                        } else {
                            for child in named_children(declaration) {
                                if let Some(n) = child.child_by_field_name("name") {
                                    if n.kind() == "identifier" {
                                        parsed.declared.push(name(n, source));
                                    } else {
                                        parsed.limitations.push(format!(
                                            "{file}: destructured exports are not indexed"
                                        ));
                                    }
                                }
                            }
                        }
                        parsed.pure = false;
                    } else if has_token(node, "default") {
                        parsed.declared.push("default".into());
                        parsed.pure = false;
                    } else {
                        parsed.local.extend(imported);
                    }
                    continue;
                }
                if source_node.is_none() {
                    parsed
                        .limitations
                        .push(format!("{file}: unsupported import/export syntax"));
                    continue;
                }
                let mut imported = imported;
                if export && imported.is_empty() && has_token(node, "*") {
                    imported.push(ImportedName {
                        imported: "*".into(),
                        local: "*".into(),
                        type_only,
                    });
                }
                let wildcard = require || imported.iter().any(|n| n.imported == "*");
                let side_effect = !export && !require && imported.is_empty();
                parsed.statements.push(ImportStatement {
                    specifier: source_node.and_then(|n| literal(n, source)),
                    line: node.start_position().row + 1,
                    kind: if type_only {
                        if export {
                            EdgeKind::TypeReexport
                        } else {
                            EdgeKind::Type
                        }
                    } else if export {
                        EdgeKind::Reexport
                    } else {
                        EdgeKind::Static
                    },
                    names: imported,
                    reexport: export,
                    wildcard,
                    side_effect,
                    require,
                });
            }
            "type_definition" => {
                parsed
                    .type_declarations
                    .insert(text(node, source).to_owned());
            }
            "identifier" => {
                let mut parent = node.parent();
                let mut import = false;
                let mut type_context = node.kind() == "type_identifier";
                let mut type_query = false;
                while let Some(ancestor) = parent {
                    if ancestor.kind() == "import_statement" {
                        import = true;
                        break;
                    }
                    if ancestor.kind() == "type_query" {
                        type_query = true;
                    }
                    if matches!(
                        ancestor.kind(),
                        "type_annotation"
                            | "type_alias_declaration"
                            | "interface_declaration"
                            | "type_arguments"
                            | "type_parameters"
                            | "implements_clause"
                    ) {
                        type_context = true;
                    }
                    parent = ancestor.parent();
                }
                if !import {
                    if type_context && !type_query {
                        parsed.type_uses.insert(text(node, source).into());
                    } else {
                        parsed.value_uses.insert(text(node, source).into());
                    }
                }
            }
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    continue;
                };
                let function = text(function, source);
                if function != "import" && (function != "require" || shadowed) {
                    continue;
                }
                let arg = node
                    .child_by_field_name("arguments")
                    .and_then(|n| named_children(n).first().copied());
                parsed.statements.push(ImportStatement {
                    specifier: arg.and_then(|n| literal(n, source)),
                    line: node.start_position().row + 1,
                    kind: if function == "import" || inside_function(node) {
                        EdgeKind::Dynamic
                    } else {
                        EdgeKind::Static
                    },
                    names: Vec::new(),
                    reexport: false,
                    wildcard: true,
                    side_effect: false,
                    require: function == "require",
                });
            }
            _ => {}
        }
    }
    for child in named_children(root) {
        if !matches!(
            child.kind(),
            "import_statement" | "export_statement" | "comment" | "empty_statement"
        ) {
            parsed.pure = false;
        }
        if child.kind() == "expression_statement"
            && let Some(assignment) = named_children(child)
                .into_iter()
                .find(|n| n.kind() == "assignment_expression")
            && let Some(left) = assignment.child_by_field_name("left")
            && let left = text(left, source)
            && (left == "module.exports"
                || ["exports.", "exports[", "module.exports.", "module.exports["]
                    .iter()
                    .any(|prefix| left.starts_with(prefix)))
        {
            parsed
                .limitations
                .push(format!("{file}: CommonJS exports are not symbol-indexed"));
        }
    }
    if parsed.statements.iter().any(|s| s.side_effect) {
        parsed.pure = false;
    }
    Ok(parsed)
}

/// Restrict configuration reads as well as final targets, including symlink escapes.
#[derive(Clone)]
struct RepositoryFs {
    root: PathBuf,
    footprint: Arc<Mutex<BTreeSet<PathBuf>>>,
}
impl RepositoryFs {
    fn record(&self, path: &Path) {
        if path.starts_with(&self.root) {
            self.footprint
                .lock()
                .expect("resolver metadata lock")
                .insert(path.to_owned());
        }
    }
    fn checked(&self, path: &Path) -> std::io::Result<PathBuf> {
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(&self.root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "outside repository",
            ));
        }
        Ok(canonical)
    }
}
impl oxc_resolver::FileSystem for RepositoryFs {
    fn new() -> Self {
        Self {
            root: PathBuf::new(),
            footprint: Default::default(),
        }
    }
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.record(path);
        let checked = self.checked(path)?;
        self.record(&checked);
        std::fs::read(checked)
    }
    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        self.record(path);
        let checked = self.checked(path).inspect_err(|error| {
            // Oxc appends .json after a failed extends-path probe, even for a .json request.
            if error.kind() == std::io::ErrorKind::NotFound
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                self.record(&path.with_extension(""));
            }
        })?;
        self.record(&checked);
        std::fs::read_to_string(checked)
    }
    fn metadata(&self, path: &Path) -> std::io::Result<oxc_resolver::FileMetadata> {
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("json" | "jsonc")
        ) {
            self.record(path);
        }
        Ok(std::fs::metadata(self.checked(path)?)?.into())
    }
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<oxc_resolver::FileMetadata> {
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("json" | "jsonc")
        ) {
            self.record(path);
        }
        self.checked(path)?;
        Ok(std::fs::symlink_metadata(path)?.into())
    }
    fn read_link(&self, path: &Path) -> std::result::Result<PathBuf, oxc_resolver::ResolveError> {
        self.record(path);
        self.checked(path)
            .map_err(|_| oxc_resolver::ResolveError::NotFound(path.display().to_string()))?;
        std::fs::read_link(path)
            .map_err(|_| oxc_resolver::ResolveError::NotFound(path.display().to_string()))
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        self.checked(path)
    }
}
fn repo_root(root: &Path) -> PathBuf {
    root.ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(root)
        .to_path_buf()
}
fn resolver(
    root: &Path,
    require: bool,
    workspaces: &[(String, PathBuf)],
    tsconfig: bool,
    footprint: Arc<Mutex<BTreeSet<PathBuf>>>,
) -> oxc_resolver::ResolverGeneric<RepositoryFs> {
    let options = oxc_resolver::ResolveOptions {
        tsconfig: tsconfig.then_some(oxc_resolver::TsconfigDiscovery::Auto),
        alias: workspaces
            .iter()
            .map(|(name, path)| {
                (
                    name.clone(),
                    vec![oxc_resolver::AliasValue::Path(
                        path.to_string_lossy().into_owned(),
                    )],
                )
            })
            .collect(),
        allow_package_exports_in_directory_resolve: true,
        extensions: [
            ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".json", ".node",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        extension_alias: vec![
            (
                ".js".into(),
                vec![".ts".into(), ".tsx".into(), ".js".into()],
            ),
            (".mjs".into(), vec![".mts".into(), ".mjs".into()]),
            (".cjs".into(), vec![".cts".into(), ".cjs".into()]),
        ],
        condition_names: vec![
            if require { "require" } else { "import" }.into(),
            "node".into(),
            "default".into(),
        ],
        main_fields: if require {
            vec!["main".into()]
        } else {
            vec!["module".into(), "main".into()]
        },
        node_path: false,
        builtin_modules: true,
        ..Default::default()
    };
    oxc_resolver::ResolverGeneric::new_with_file_system(
        RepositoryFs {
            root: repo_root(root),
            footprint,
        },
        options,
    )
}
fn package_manifests(
    root: &Path,
    footprint: &Arc<Mutex<BTreeSet<PathBuf>>>,
) -> Result<BTreeMap<PathBuf, serde_json::Value>> {
    let mut manifests = BTreeMap::new();
    for file in super::source_files(root, &["json"])? {
        if Path::new(&file)
            .file_name()
            .is_some_and(|n| n == "package.json")
        {
            let path = root.join(file);
            footprint
                .lock()
                .expect("resolver metadata lock")
                .insert(path.clone());
            let source = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            // Invalid fixture/package manifests cannot prove package identity.
            if let Ok(value) = serde_json::from_str(&source) {
                manifests.insert(path.parent().unwrap().to_owned(), value);
            }
        }
    }
    Ok(manifests)
}
fn declared_package(
    file: &Path,
    package: &str,
    manifests: &BTreeMap<PathBuf, serde_json::Value>,
) -> bool {
    file.ancestors()
        .skip(1)
        .filter_map(|dir| manifests.get(dir))
        .any(|manifest| {
            [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ]
            .iter()
            .any(|field| manifest[*field].get(package).is_some())
        })
}
fn workspaces(
    root: &Path,
    manifests: &BTreeMap<PathBuf, serde_json::Value>,
) -> Result<Vec<(String, PathBuf)>> {
    let mut patterns = Vec::new();
    if let Some(manifest) = manifests.get(root)
        && let Some(values) = manifest["workspaces"]
            .as_array()
            .or_else(|| manifest["workspaces"]["packages"].as_array())
    {
        patterns.extend(values.iter().filter_map(|v| v.as_str()).map(str::to_owned));
    }
    let pnpm = root.join("pnpm-workspace.yaml");
    if pnpm.is_file() && pnpm.canonicalize()?.starts_with(root) {
        let config: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&std::fs::read_to_string(pnpm)?)?;
        if let Some(values) = config["packages"].as_sequence() {
            patterns.extend(values.iter().filter_map(|v| v.as_str()).map(str::to_owned));
        }
    }
    let positive = crate::collect::globset(
        &patterns
            .iter()
            .filter(|p| !p.starts_with('!'))
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let negative = crate::collect::globset(
        &patterns
            .iter()
            .filter_map(|p| p.strip_prefix('!').map(str::to_owned))
            .collect::<Vec<_>>(),
    )?;
    let mut out = Vec::new();
    for (dir, manifest) in manifests {
        let relative = dir.strip_prefix(root)?.to_string_lossy();
        if positive.is_match(relative.as_ref())
            && !negative.is_match(relative.as_ref())
            && let Some(name) = manifest["name"].as_str()
        {
            out.push((name.to_owned(), dir.clone()));
        }
    }
    Ok(out)
}

pub fn collect(
    root: &Path,
    source_dir: &str,
    policy: &Policy,
    scan_all: bool,
) -> Result<super::Output> {
    collect_cached(root, source_dir, policy, scan_all, &mut Cache::default())
}

#[derive(Default)]
pub(crate) struct Cache {
    parsed: super::ParseCache<Parsed>,
    metadata: Option<Metadata>,
    resolutions: Resolutions,
    inventory: Option<Vec<String>>,
    selection: Option<(Vec<u8>, BTreeSet<String>)>,
    footprint: Arc<Mutex<BTreeSet<PathBuf>>>,
}
type Resolved = std::result::Result<oxc_resolver::Resolution, oxc_resolver::ResolveError>;
type Resolutions = BTreeMap<String, BTreeMap<(String, bool, bool), Resolved>>;
fn resolve_cached(
    resolutions: &mut Resolutions,
    file: &str,
    specifier: &str,
    mode: (bool, bool),
    resolver: &oxc_resolver::ResolverGeneric<RepositoryFs>,
    root: &Path,
) -> Resolved {
    resolutions
        .entry(file.to_owned())
        .or_default()
        .entry((specifier.to_owned(), mode.0, mode.1))
        .or_insert_with(|| resolver.resolve_file(root.join(file), specifier))
        .clone()
}
struct Metadata {
    manifests: BTreeMap<PathBuf, serde_json::Value>,
    workspaces: Vec<(String, PathBuf)>,
    imports: oxc_resolver::ResolverGeneric<RepositoryFs>,
    requires: oxc_resolver::ResolverGeneric<RepositoryFs>,
    direct_imports: oxc_resolver::ResolverGeneric<RepositoryFs>,
    direct_requires: oxc_resolver::ResolverGeneric<RepositoryFs>,
}
impl Cache {
    pub(crate) fn invalidate(&mut self, files: &[String]) {
        self.parsed.invalidate(files);
        for file in files {
            self.resolutions.remove(file);
        }
    }
    pub(crate) fn metadata_files(&self) -> Vec<PathBuf> {
        self.footprint
            .lock()
            .expect("resolver metadata lock")
            .iter()
            .cloned()
            .collect()
    }
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.parsed.parsed, self.parsed.reused)
    }
}
pub(crate) fn collect_cached(
    root: &Path,
    source_dir: &str,
    policy: &Policy,
    scan_all: bool,
    cache: &mut Cache,
) -> Result<super::Output> {
    let root = root
        .canonicalize()
        .context("resolving native TypeScript scan root")?;
    if cache.inventory.is_none() {
        cache.inventory = Some(super::source_files(
            &root,
            &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"],
        )?);
    }
    let files = cache.inventory.as_ref().unwrap().clone();
    let mut policy_key = policy.cache_key(&files)?;
    policy_key.extend_from_slice(source_dir.as_bytes());
    policy_key.push(u8::from(scan_all));
    if cache
        .selection
        .as_ref()
        .is_none_or(|(key, _)| key != &policy_key)
    {
        let selected = policy
            .select(&files)?
            .into_iter()
            .filter(|path| scan_all || source_dir == "." || Path::new(path).starts_with(source_dir))
            .collect();
        cache.selection = Some((policy_key, selected));
    }
    let selected = cache.selection.as_ref().unwrap().1.clone();
    if cache.metadata.is_none() {
        let manifests = package_manifests(&repo_root(&root), &cache.footprint)?;
        cache
            .footprint
            .lock()
            .expect("resolver metadata lock")
            .insert(repo_root(&root).join("pnpm-workspace.yaml"));
        let workspaces = workspaces(&repo_root(&root), &manifests)?;
        cache.metadata = Some(Metadata {
            imports: resolver(&root, false, &workspaces, true, cache.footprint.clone()),
            requires: resolver(&root, true, &workspaces, true, cache.footprint.clone()),
            direct_imports: resolver(&root, false, &workspaces, false, cache.footprint.clone()),
            direct_requires: resolver(&root, true, &workspaces, false, cache.footprint.clone()),
            manifests,
            workspaces,
        });
    }
    let Metadata {
        manifests,
        workspaces,
        imports,
        requires,
        direct_imports,
        direct_requires,
    } = cache.metadata.as_ref().unwrap();
    let mut config_failures = BTreeSet::new();
    let known: BTreeSet<_> = files.iter().cloned().collect();
    let mut graph = RawGraph {
        files,
        edges: Vec::new(),
        metadata: BTreeMap::new(),
        imports: Some(Vec::new()),
        collector: Collector {
            name: "tree-sitter-typescript".into(),
            version: Some("tree-sitter 0.27.1; oxc_resolver 11.24.3".into()),
            cruised: 0,
            type_marking: Some("unavailable".into()),
            ..Default::default()
        },
    };
    let mut symbols = Symbols {
        complete: true,
        ..Default::default()
    };
    let mut modules = Vec::new();
    // Read excluded forwarders only if a selected file actually imports them.
    let mut queue: Vec<_> = selected.into_iter().collect();
    let mut parsed_files = BTreeSet::new();
    let mut type_declarations = BTreeMap::new();
    let mut implicit_types = BTreeSet::new();
    while let Some(file) = queue.pop() {
        if !parsed_files.insert(file.clone()) {
            continue;
        }
        let parsed = cache.parsed.get(&file, || {
            let source = std::fs::read_to_string(root.join(&file))
                .with_context(|| format!("reading TypeScript source {file}"))?;
            parse(&file, &source).with_context(|| format!("parsing {file}"))
        })?;
        if !parsed.limitations.is_empty() {
            symbols.complete = false;
            symbols.limitations.extend(parsed.limitations);
        }
        type_declarations.insert(file.clone(), parsed.type_declarations.clone());
        let mut module = BarrelModule {
            file: file.clone(),
            pure: parsed.pure,
            declared: parsed.declared,
            exports: Vec::new(),
            imports: Vec::new(),
        };
        let mut bindings = BTreeMap::new();
        for statement in parsed.statements {
            let specifier = statement
                .specifier
                .clone()
                .unwrap_or_else(|| "<computed>".into());
            let mut record = ImportRecord {
                from: file.clone(),
                specifier: specifier.clone(),
                line: Some(statement.line),
                kind: statement.kind,
                ..Default::default()
            };
            if statement.specifier.is_none() {
                record.unresolved = Some(ImportReason::Dynamic);
            } else {
                let mut resolved = resolve_cached(
                    &mut cache.resolutions,
                    &file,
                    &specifier,
                    (statement.require, true),
                    if statement.require { requires } else { imports },
                    &root,
                );
                if let Err(
                    error @ (oxc_resolver::ResolveError::TsconfigLoadFailed { .. }
                    | oxc_resolver::ResolveError::TsconfigNotFound(_)
                    | oxc_resolver::ResolveError::TsconfigCircularExtend(_)),
                ) = &resolved
                {
                    config_failures.insert(format!("{file}: {error}; relative imports and Node builtins resolved without the unavailable tsconfig"));
                    // Relative requests do not depend on paths/baseUrl; never guess aliases from a broken config.
                    if specifier.starts_with('.')
                        || specifier.starts_with("node:")
                        || package_name(&specifier).is_some()
                    {
                        let fallback = resolve_cached(
                            &mut cache.resolutions,
                            &file,
                            &specifier,
                            (statement.require, false),
                            if statement.require {
                                direct_requires
                            } else {
                                direct_imports
                            },
                            &root,
                        );
                        if specifier.starts_with('.')
                            || matches!(fallback, Err(oxc_resolver::ResolveError::Builtin { .. }))
                        {
                            resolved = fallback;
                        }
                    }
                }
                match resolved {
                    Ok(resolved) => {
                        let path = resolved.path();
                        if path.components().any(|c| c.as_os_str() == "node_modules") {
                            record.external = resolved
                                .package_json()
                                .and_then(|p| p.name())
                                .map(str::to_owned)
                                .or_else(|| package_name(&specifier));
                        } else if let Ok(relative) = path.strip_prefix(&root) {
                            let target = relative.to_string_lossy().replace('\\', "/");
                            if known.contains(&target) {
                                record.targets.push(target.clone());
                                queue.push(target.clone());
                                for named in &statement.names {
                                    if !named.type_only
                                        && named.imported != "*"
                                        && (statement.reexport
                                            || parsed.type_uses.contains(&named.local)
                                                && !parsed.value_uses.contains(&named.local))
                                    {
                                        implicit_types.insert((
                                            file.clone(),
                                            target.clone(),
                                            statement.line,
                                            named.imported.clone(),
                                        ));
                                    }
                                }
                                let mut groups: BTreeMap<bool, Vec<String>> = BTreeMap::new();
                                for named in &statement.names {
                                    groups
                                        .entry(named.type_only)
                                        .or_default()
                                        .push(named.imported.clone());
                                }
                                if groups.is_empty() {
                                    groups.insert(false, Vec::new());
                                }
                                for (type_only, names) in groups {
                                    let kind = if type_only {
                                        if statement.reexport {
                                            EdgeKind::TypeReexport
                                        } else {
                                            EdgeKind::Type
                                        }
                                    } else {
                                        statement.kind
                                    };
                                    graph.edges.push(Edge {
                                        from: file.clone(),
                                        to: target.clone(),
                                        kind,
                                        specifier: Some(specifier.clone()),
                                        line: Some(statement.line),
                                        names: names.clone(),
                                        ..Default::default()
                                    });
                                    module.imports.push(ImportRoute {
                                        to: target.clone(),
                                        names,
                                        wildcard: statement.wildcard,
                                        kind,
                                        specifier: Some(specifier.clone()),
                                        line: statement.line,
                                        side_effect: statement.side_effect,
                                    });
                                }
                                for named in &statement.names {
                                    if statement.reexport {
                                        module.exports.push(ExportRoute {
                                            exported: named.local.clone(),
                                            imported: named.imported.clone(),
                                            to: target.clone(),
                                        });
                                    } else {
                                        bindings.insert(
                                            named.local.clone(),
                                            (named.imported.clone(), target.clone()),
                                        );
                                    }
                                    symbols.imports.push(SymbolImport {
                                        from: file.clone(),
                                        to: target.clone(),
                                        name: named.imported.clone(),
                                        target: None,
                                        wildcard: statement.wildcard,
                                    });
                                }
                            } else if path.components().any(|c| c.as_os_str() == "node_modules") {
                                record.external = package_name(&specifier);
                            } else {
                                // Resolved assets belong in accounting, not in the source import graph.
                                record.targets.push(target);
                            }
                        } else {
                            // A resolved sibling workspace is external to this scanned root, not missing.
                            let identity = path.ancestors().skip(1).find_map(|dir| {
                                manifests.get(dir).and_then(|p| p["name"].as_str())
                            });
                            if let Some(identity) = identity.filter(|identity| {
                                package_name(&specifier).as_deref() == Some(*identity)
                            }) {
                                record.external = Some(identity.to_owned());
                            } else {
                                record.unresolved = Some(ImportReason::Missing);
                            }
                        }
                    }
                    Err(oxc_resolver::ResolveError::Builtin { resolved, .. }) => {
                        record.external = Some(resolved)
                    }
                    Err(oxc_resolver::ResolveError::MatchedAliasNotFound(_, alias)) => {
                        let workspace = workspaces.iter().find(|(name, dir)| {
                            name == &specifier
                                && alias == specifier
                                && !dir.starts_with(&root)
                                && dir
                                    .canonicalize()
                                    .is_ok_and(|p| p.starts_with(repo_root(&root)))
                        });
                        if workspace.is_some()
                            && declared_package(&root.join(&file), &specifier, manifests)
                        {
                            record.external = Some(specifier.clone());
                        } else {
                            record.unresolved = Some(ImportReason::Alias);
                        }
                    }
                    Err(
                        oxc_resolver::ResolveError::PackagePathNotExported { .. }
                        | oxc_resolver::ResolveError::InvalidPackageTarget(..),
                    ) => {
                        record.unresolved = Some(ImportReason::Missing);
                    }
                    Err(_) => {
                        let declared = package_name(&specifier).filter(|pkg| {
                            !workspaces.iter().any(|(name, _)| name == pkg)
                                && declared_package(&root.join(&file), pkg, manifests)
                        });
                        if let Some(pkg) = declared {
                            record.external = Some(pkg);
                        } else {
                            record.unresolved = Some(if specifier.starts_with('#') {
                                ImportReason::Alias
                            } else {
                                ImportReason::Missing
                            });
                        }
                    }
                }
            }
            if record.targets.is_empty() && record.external.is_none() && record.unresolved.is_none()
            {
                record.unresolved = Some(ImportReason::Missing);
            }
            graph.imports.as_mut().unwrap().push(record);
        }
        for local in parsed.local {
            if let Some((imported, to)) = bindings.get(&local.imported) {
                module.exports.push(ExportRoute {
                    exported: local.local,
                    imported: imported.clone(),
                    to: to.clone(),
                });
            } else {
                module.declared.push(local.local);
            }
        }
        modules.push(module);
    }
    let implicit = |from: &str, to: &str, line: Option<usize>, name: &str| {
        line.is_some_and(|line| {
            implicit_types.contains(&(from.to_owned(), to.to_owned(), line, name.to_owned()))
        })
    };
    let positions: BTreeMap<_, _> = modules
        .iter()
        .enumerate()
        .map(|(index, module)| (module.file.clone(), index))
        .collect();
    // Require both a declaration erased by TypeScript and exclusively type-context AST uses.
    for edge in &mut graph.edges {
        if matches!(edge.kind, EdgeKind::Static | EdgeKind::Reexport)
            && !edge.names.is_empty()
            && edge.names.iter().all(|name| {
                type_declarations
                    .get(&edge.to)
                    .is_some_and(|types| types.contains(name))
                    && implicit(&edge.from, &edge.to, edge.line, name)
            })
        {
            edge.kind = if edge.kind == EdgeKind::Reexport {
                EdgeKind::TypeReexport
            } else {
                EdgeKind::Type
            };
            if let Some(&index) = positions.get(&edge.from) {
                for route in &mut modules[index].imports {
                    if route.to == edge.to
                        && Some(route.line) == edge.line
                        && route.names == edge.names
                    {
                        route.kind = edge.kind;
                    }
                }
            }
        }
    }
    symbols.limitations.extend(config_failures);
    symbols
        .limitations
        .extend(barrels::resolve(&mut graph, &modules));
    for module in &modules {
        for declared in &module.declared {
            symbols.exports.push(self_export(&module.file, declared));
        }
    }
    // Resolve exports using the same route graph, including renamed and star forwarding.
    barrels::forward_exports(&mut symbols.exports, &modules, true);
    for edge in &mut graph.edges {
        let written = edge.original_to.as_ref().unwrap_or(&edge.to);
        if matches!(edge.kind, EdgeKind::Static | EdgeKind::Reexport)
            && !edge.names.is_empty()
            && edge.names.iter().all(|name| {
                implicit(&edge.from, written, edge.line, name)
                    && symbols.exports.iter().any(|export| {
                        export.file == *written
                            && export.name == *name
                            && export.target.file == edge.to
                            && type_declarations
                                .get(&export.target.file)
                                .is_some_and(|types| types.contains(&export.target.name))
                    })
            })
        {
            edge.kind = if edge.kind == EdgeKind::Reexport {
                EdgeKind::TypeReexport
            } else {
                EdgeKind::Type
            };
        }
    }
    barrels::import_targets(&mut symbols.imports, &symbols.exports);
    barrels::retarget_records(&mut graph, true);
    graph.files.retain(|file| {
        scan_all
            || source_dir == "."
            || Path::new(file).starts_with(source_dir)
            || parsed_files.contains(file)
    });
    graph.collector.cruised = parsed_files.len();
    graph.collector.unresolved = Some(unresolved_count(graph.imports.as_ref().unwrap()));
    symbols.limitations.push("Implicit types require interface/type-alias declarations and exclusively type-context uses; uncertain usage remains runtime. CommonJS export surfaces are not indexed.".into());
    symbols.complete = false;
    Ok(super::Output {
        raw: graph,
        symbols,
        source_root: if scan_all {
            ".".into()
        } else {
            source_dir.into()
        },
    })
}
