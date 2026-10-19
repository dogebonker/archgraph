//! Public API evidence. Unused means unused by consumers inside this scan.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{Document, Edge, Language, SCHEMA};

pub const DEFAULT_MAX_SYMBOLS: usize = 50;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Symbols {
    #[serde(default)]
    pub complete: bool,
    pub exports: Vec<SymbolExport>,
    pub imports: Vec<SymbolImport>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SymbolTarget {
    pub file: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SymbolExport {
    pub file: String,
    pub name: String,
    pub target: SymbolTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolImport {
    pub from: String,
    pub to: String,
    pub name: String,
    pub target: Option<SymbolTarget>,
    pub wildcard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceReport {
    pub schema: String,
    pub kind: String,
    pub max_symbols: usize,
    pub complete: bool,
    pub modules: Vec<ModuleSurface>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSurface {
    pub module: String,
    pub entrypoints: Vec<String>,
    pub exported: Vec<SymbolExport>,
    pub used_externally: Vec<SymbolUse>,
    pub could_be_internal: Vec<SymbolExport>,
    pub deep_imports: Vec<DeepImport>,
    pub oversized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolUse {
    pub target: SymbolTarget,
    pub importers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepImport {
    pub from: String,
    pub to: String,
    pub symbols: Vec<String>,
    pub suggestion: String,
}

pub fn validate(doc: &Document, symbols: &Symbols) -> Result<()> {
    let mut strings: Vec<&str> = symbols.limitations.iter().map(String::as_str).collect();
    for export in &symbols.exports {
        strings.extend([
            export.file.as_str(),
            export.name.as_str(),
            export.target.file.as_str(),
            export.target.name.as_str(),
        ]);
    }
    for import in &symbols.imports {
        strings.extend([
            import.from.as_str(),
            import.to.as_str(),
            import.name.as_str(),
        ]);
        if let Some(target) = &import.target {
            strings.extend([target.file.as_str(), target.name.as_str()]);
        }
    }
    if strings.iter().any(|s| s.chars().any(char::is_control)) {
        bail!("symbol evidence must not contain control characters");
    }
    let files: BTreeSet<_> = doc.files.iter().map(|f| f.path.as_str()).collect();
    for export in &symbols.exports {
        if !files.contains(export.file.as_str()) || !files.contains(export.target.file.as_str()) {
            bail!("symbol export refers to a file outside the scan");
        }
        if export.name.is_empty() || export.target.name.is_empty() {
            bail!("symbol names must not be empty");
        }
    }
    for import in &symbols.imports {
        if !files.contains(import.from.as_str())
            || !files.contains(import.to.as_str())
            || import
                .target
                .as_ref()
                .is_some_and(|t| !files.contains(t.file.as_str()) || t.name.is_empty())
        {
            bail!("symbol import refers to a file outside the scan or an empty symbol");
        }
    }
    Ok(())
}

pub fn attach(doc: &mut Document, symbols: Symbols, max_symbols: usize) -> Result<()> {
    validate(doc, &symbols)?;
    doc.symbols = Some(symbols);
    doc.surface = Some(analyze(doc, max_symbols)?);
    crate::recipes::attach(doc)?;
    Ok(())
}

pub fn analyze(doc: &Document, max_symbols: usize) -> Result<SurfaceReport> {
    if max_symbols == 0 {
        bail!("surface size threshold must be positive");
    }
    let rules = crate::rules::compile(&doc.config)?;
    let rules = rules.as_ref();
    let files: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.module.as_str()))
        .collect();
    let symbols = doc.symbols.as_ref();
    let mut limitations = symbols.map(|s| s.limitations.clone()).unwrap_or_else(|| {
        vec!["Symbol data is absent; scan with --symbols to assess exports and usage.".into()]
    });
    limitations.push("Could be internal means unused by consumers in this scan, not unused by downstream packages or runtime reflection. Review before removing public API.".into());
    let complete = symbols.is_some_and(|s| s.complete);
    if !complete {
        limitations.push(
            "Symbol coverage is incomplete; could-be-internal suggestions are withheld.".into(),
        );
    }
    let (exports, imports) = symbols.map_or((&[][..], &[][..]), |s| {
        (s.exports.as_slice(), s.imports.as_slice())
    });
    let mut members_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &doc.files {
        members_of.entry(&f.module).or_default().push(&f.path);
    }
    let mut exports_of: BTreeMap<&str, Vec<&SymbolExport>> = BTreeMap::new();
    for export in exports {
        exports_of.entry(&export.file).or_default().push(export);
    }
    let mut imports_from: BTreeMap<&str, Vec<&SymbolImport>> = BTreeMap::new();
    for import in imports {
        imports_from.entry(&import.from).or_default().push(import);
    }
    let mut edges_into: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    for edge in &doc.edges {
        if let Some(&module) = files.get(edge.to.as_str()) {
            edges_into.entry(module).or_default().push(edge);
        }
    }
    // Per module: each of its symbols imported from another module, with the importing files.
    let mut used_in: BTreeMap<&str, BTreeMap<SymbolTarget, BTreeSet<String>>> = BTreeMap::new();
    for import in imports {
        let targets: Vec<&SymbolTarget> = match &import.target {
            Some(target) => vec![target],
            None if import.wildcard => exports_of
                .get(import.to.as_str())
                .into_iter()
                .flatten()
                .map(|e| &e.target)
                .collect(),
            None => Vec::new(),
        };
        let from = files.get(import.from.as_str());
        for target in targets {
            let modules: BTreeSet<&str> = [import.to.as_str(), target.file.as_str()]
                .into_iter()
                .filter_map(|file| files.get(file).copied())
                .filter(|module| Some(module) != from)
                .collect();
            for module in modules {
                used_in
                    .entry(module)
                    .or_default()
                    .entry(target.clone())
                    .or_default()
                    .insert(import.from.clone());
            }
        }
    }
    let mut modules = Vec::new();
    for module in &doc.modules {
        let members = members_of
            .get(module.id.as_str())
            .map_or(&[][..], Vec::as_slice);
        let mut entries = BTreeSet::new();
        let mut explicit = false;
        if let Some(rules) = rules {
            // Each declared entry once, as glob and as written.
            let mut declared: BTreeMap<String, String> = BTreeMap::new();
            for path in members {
                if let (Some((_, written)), Some((_, globs))) =
                    (rules.public_for(path), rules.public_globs(path))
                {
                    explicit = true;
                    declared.extend(globs.into_iter().zip(written));
                }
            }
            for (glob, entry) in declared {
                let matches: Vec<_> = doc
                    .files
                    .iter()
                    .filter(|f| rules.bound_match(&glob, &f.path))
                    .collect();
                if matches.is_empty() {
                    limitations.push(format!(
                        "Module {} declares entrypoint {entry}, which is absent from the scan.",
                        module.id
                    ));
                }
                entries.extend(matches.into_iter().map(|f| f.path.clone()));
            }
        }
        if !explicit && doc.source.language != Language::Go {
            // Walk virtual files toward the scan root to recover the module boundary even
            // when all real files happen to live in nested folders.
            let mut root = None;
            if let Some(file) = members.first() {
                let mut dir = crate::modules::dir_of(file);
                loop {
                    let extension = file.rsplit_once('.').map_or("ts", |(_, ext)| ext);
                    let probe =
                        crate::modules::join(dir, &format!("__archgraph_surface__.{extension}"));
                    let assigned =
                        crate::modules::assigned(&probe, &doc.config, doc.source.language, rules);
                    if assigned != module.id {
                        break;
                    }
                    root = Some(dir.to_owned());
                    if dir.is_empty() {
                        break;
                    }
                    dir = crate::modules::dir_of(dir);
                }
            }
            entries.extend(
                members
                    .iter()
                    .filter(|path| {
                        let name = path.rsplit('/').next().unwrap_or(path);
                        root.as_deref() == Some(crate::modules::dir_of(path))
                            && crate::modules::is_index(name)
                    })
                    .map(|path| path.to_string()),
            );
            entries.extend(
                doc.package_entries
                    .iter()
                    .filter(|p| files.get(p.as_str()) == Some(&module.id.as_str()))
                    .cloned(),
            );
            // A Python package marker alone does not declare a public facade.
            if doc.source.language == Language::Python {
                entries.retain(|path| {
                    exports_of.get(path.as_str()).is_some_and(|exports| {
                        exports.iter().any(|e| !e.name.is_empty() && e.name != "*")
                    })
                });
            }
        }
        // Without a declared facade, direct file exports remain the observable public API.
        let all_exports = doc.source.language == Language::Go || entries.is_empty() && !explicit;
        let sources: Vec<&str> = if all_exports {
            members.to_vec()
        } else {
            entries.iter().map(String::as_str).collect()
        };
        let exported: Vec<_> = sources
            .into_iter()
            .flat_map(|file| exports_of.get(file).into_iter().flatten())
            .map(|&e| e.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let used = used_in.remove(module.id.as_str()).unwrap_or_default();
        let could_be_internal = if complete {
            exported
                .iter()
                .filter(|e| !used.contains_key(&e.target))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let mut deep_imports = Vec::new();
        for edge in edges_into.get(module.id.as_str()).into_iter().flatten() {
            let written = edge.original_to.as_deref().unwrap_or(&edge.to);
            let boundary =
                rules.and_then(|r| Some((r, r.public_for(written)?, r.public_globs(written)?)));
            let declared = boundary.is_some();
            let (outside, bypass, expected) =
                if let Some((rules, (_, entries), (pattern, globs))) = boundary {
                    let inside = rules.bound_match(&pattern, &edge.from);
                    let entry_match = globs.iter().any(|glob| rules.bound_match(glob, written));
                    (!inside, !entry_match, entries)
                } else {
                    (
                        files.get(edge.from.as_str()) != Some(&module.id.as_str()),
                        !entries.is_empty() && !entries.contains(written),
                        entries.iter().cloned().collect(),
                    )
                };
            if outside && bypass {
                let names = imports_from
                    .get(edge.from.as_str())
                    .into_iter()
                    .flatten()
                    .filter(|i| i.to == edge.to || i.to == written)
                    .map(|i| i.name.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let suggestion = if declared {
                    format!(
                        "Import through {}; re-export the required symbols there or declare a boundary rule.",
                        expected.join(", ")
                    )
                } else {
                    format!(
                        "Observed entrypoint convention: {}. No public boundary is declared; this direct import is informational, not a violation or a repair instruction.",
                        expected.join(", ")
                    )
                };
                deep_imports.push(DeepImport {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    symbols: names,
                    suggestion,
                });
            }
        }
        deep_imports.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        deep_imports.dedup_by(|a, b| a.from == b.from && a.to == b.to);
        let oversized = used.len() > max_symbols;
        modules.push(ModuleSurface {
            module: module.id.clone(),
            entrypoints: entries.into_iter().collect(),
            exported,
            used_externally: used
                .into_iter()
                .map(|(target, importers)| SymbolUse {
                    target,
                    importers: importers.into_iter().collect(),
                })
                .collect(),
            could_be_internal,
            deep_imports,
            oversized,
        });
    }
    limitations.sort();
    limitations.dedup();
    Ok(SurfaceReport {
        schema: SCHEMA.into(),
        kind: "surface".into(),
        max_symbols,
        complete,
        modules,
        limitations,
    })
}
