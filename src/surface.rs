//! Public API evidence. Unused means unused by consumers inside this scan.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{Document, Language, SCHEMA};

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
    Ok(())
}

pub fn analyze(doc: &Document, max_symbols: usize) -> Result<SurfaceReport> {
    if max_symbols == 0 {
        bail!("surface size threshold must be positive");
    }
    let rules = crate::rules::compile(&doc.config)?;
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
    let mut modules = Vec::new();
    for module in &doc.modules {
        let members: Vec<_> = doc.files.iter().filter(|f| f.module == module.id).collect();
        let mut entries = BTreeSet::new();
        let mut explicit = false;
        for file in &members {
            if let Some((_, declared)) = rules.as_ref().and_then(|r| r.public_for(&file.path)) {
                explicit = true;
                for entry in declared {
                    let matcher = crate::collect::globset(std::slice::from_ref(&entry))?;
                    let matches: Vec<_> = doc
                        .files
                        .iter()
                        .filter(|f| matcher.is_match(&f.path))
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
        }
        if !explicit && doc.source.language != Language::Go {
            // Walk virtual files toward the scan root to recover the module boundary even
            // when all real files happen to live in nested folders.
            let mut root = None;
            if let Some(file) = members.first() {
                let mut dir = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
                loop {
                    let extension = file.path.rsplit_once('.').map_or("ts", |(_, ext)| ext);
                    let probe =
                        crate::collect::join(dir, &format!("__archgraph_surface__.{extension}"))
                            .trim_start_matches('/')
                            .to_owned();
                    let assigned = crate::modules::assigned(
                        &probe,
                        &doc.config,
                        doc.source.language,
                        rules.as_ref(),
                    );
                    if assigned != module.id {
                        break;
                    }
                    root = Some(dir.to_owned());
                    if dir.is_empty() {
                        break;
                    }
                    dir = dir.rsplit_once('/').map_or("", |(parent, _)| parent);
                }
            }
            entries.extend(
                members
                    .iter()
                    .filter(|f| {
                        let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                        let parent = f.path.rsplit_once('/').map_or("", |(dir, _)| dir);
                        root.as_deref() == Some(parent)
                            && (name == "__init__.py" || crate::modules::TS_INDEX.contains(&name))
                    })
                    .map(|f| f.path.clone()),
            );
            entries.extend(
                doc.package_entries
                    .iter()
                    .filter(|p| files.get(p.as_str()) == Some(&module.id.as_str()))
                    .cloned(),
            );
        }
        // Without a declared facade, direct file exports remain the observable public API.
        let all_exports = doc.source.language == Language::Go || entries.is_empty() && !explicit;
        let exported: Vec<_> = symbols
            .into_iter()
            .flat_map(|s| &s.exports)
            .filter(|e| {
                if all_exports {
                    files.get(e.file.as_str()) == Some(&module.id.as_str())
                } else {
                    entries.contains(&e.file)
                }
            })
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut used: BTreeMap<SymbolTarget, BTreeSet<String>> = BTreeMap::new();
        for import in symbols.into_iter().flat_map(|s| &s.imports) {
            if files.get(import.from.as_str()) == Some(&module.id.as_str()) {
                continue;
            }
            let targets: Vec<_> = if let Some(target) = &import.target {
                vec![target.clone()]
            } else if import.wildcard {
                symbols
                    .into_iter()
                    .flat_map(|s| &s.exports)
                    .filter(|e| e.file == import.to)
                    .map(|e| e.target.clone())
                    .collect()
            } else {
                Vec::new()
            };
            for target in targets {
                if files.get(import.to.as_str()) == Some(&module.id.as_str())
                    || files.get(target.file.as_str()) == Some(&module.id.as_str())
                {
                    used.entry(target).or_default().insert(import.from.clone());
                }
            }
        }
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
        for edge in &doc.edges {
            if files.get(edge.to.as_str()) != Some(&module.id.as_str()) {
                continue;
            }
            let written = edge.original_to.as_deref().unwrap_or(&edge.to);
            let boundary = rules.as_ref().and_then(|r| r.public_for(written));
            let (outside, bypass, expected) = if let Some((pattern, entries)) = boundary {
                let inside = crate::collect::globset(&[pattern])?.is_match(&edge.from);
                let entry_match = crate::collect::globset(&entries)?.is_match(written);
                (!inside, !entry_match, entries)
            } else {
                (
                    files.get(edge.from.as_str()) != Some(&module.id.as_str()),
                    !entries.is_empty() && !entries.contains(written),
                    entries.iter().cloned().collect(),
                )
            };
            if outside && bypass {
                let names = symbols
                    .into_iter()
                    .flat_map(|s| &s.imports)
                    .filter(|i| i.from == edge.from && (i.to == edge.to || i.to == written))
                    .map(|i| i.name.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                deep_imports.push(DeepImport { from: edge.from.clone(), to: edge.to.clone(), symbols: names, suggestion: format!("Import through {}; re-export the required symbols there or declare a boundary rule.", expected.join(", ")) });
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
