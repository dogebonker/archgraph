//! Reading order over the semantic module graph; source bodies are never opened.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::Result;

use crate::diff::count;
use crate::model::{ModuleClass, ScanInput};

#[derive(Default)]
pub struct Options {
    pub reverse: bool,
    pub agent: bool,
}

fn code(text: &str) -> String {
    let text = text.escape_debug().to_string();
    let width = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0) + 1;
    let fence = "`".repeat(width);
    format!("{fence} {text} {fence}")
}

pub fn markdown(input: &ScanInput, options: &Options) -> Result<String> {
    crate::validate_scan(input)?;
    let names: Vec<_> = input
        .files
        .iter()
        .filter(|file| file.class != Some(ModuleClass::Tests))
        .map(|file| file.module.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let ids: BTreeMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(i, &name)| (name, i))
        .collect();
    let files: BTreeMap<_, _> = input
        .files
        .iter()
        .filter(|file| file.class != Some(ModuleClass::Tests))
        .map(|file| (file.path.as_str(), ids[file.module.as_str()]))
        .collect();
    let mut outbound = vec![BTreeSet::new(); names.len()];
    for edge in &input.edges {
        if !input.config.all_kinds && !edge.kind.is_runtime() {
            continue;
        }
        if let (Some(&from), Some(&to)) =
            (files.get(edge.from.as_str()), files.get(edge.to.as_str()))
            && from != to
        {
            outbound[from].insert(to);
        }
    }
    let adjacency: Vec<_> = outbound
        .iter()
        .map(|links| links.iter().copied().collect())
        .collect();
    let (mut components, component_of, dependencies) = crate::graph::condense(&adjacency);
    for component in &mut components {
        component.sort_by_key(|&module| (outbound[module].len(), names[module]));
    }
    let mut importers = vec![BTreeSet::new(); components.len()];
    for (source, targets) in dependencies.iter().enumerate() {
        for &target in targets {
            importers[target].insert(source);
        }
    }
    let mut fan_in = vec![BTreeSet::new(); components.len()];
    let mut fan_out = fan_in.clone();
    for (from, links) in outbound.iter().enumerate() {
        for &to in links {
            let (source, target) = (component_of[from], component_of[to]);
            if source != target {
                fan_in[target].insert(from);
                fan_out[source].insert(to);
            }
        }
    }
    // Leaves first: a component's depth is its longest chain of dependencies.
    let depths = crate::graph::depths(&importers);
    let mut layers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (id, &depth) in depths.iter().enumerate() {
        layers.entry(depth).or_default().push(id);
    }
    for layer in layers.values_mut() {
        layer.sort_by_key(|&id| {
            (
                Reverse(fan_in[id].len()),
                components[id].iter().map(|&m| names[m]).min(),
            )
        });
    }

    let mut surfaces: BTreeMap<usize, BTreeSet<(&str, &str)>> = BTreeMap::new();
    if options.agent {
        let surface_exports = input
            .surface
            .iter()
            .flat_map(|surface| &surface.modules)
            .flat_map(|module| &module.exported);
        let symbol_exports = input
            .symbols
            .iter()
            .filter(|_| input.surface.is_none())
            .flat_map(|symbols| &symbols.exports);
        for export in surface_exports.chain(symbol_exports) {
            if let Some(&module) = files.get(export.file.as_str()) {
                surfaces
                    .entry(module)
                    .or_default()
                    .insert((&export.file, &export.name));
            }
        }
    }

    let mut out = format!(
        "# Codebase tour\n\nOrder: {}. {} imports; tests and external packages omitted. Counts are distinct other modules, not import statements. Module names do not establish architectural intent.\n",
        if options.reverse {
            "entrypoints first (reversed layers)"
        } else {
            "leaves first"
        },
        if input.config.all_kinds {
            "All recorded"
        } else {
            "Runtime"
        }
    );
    if options.agent {
        let evidence = input
            .surface
            .as_ref()
            .map(|surface| surface.complete)
            .or_else(|| input.symbols.as_ref().map(|symbols| symbols.complete));
        out.push_str(match evidence {
            Some(true) => "\nSurface: saved export names and locations; signatures and bodies omitted.\n",
            Some(false) => "\nSurface: partial saved export names and locations; missing names do not imply a private API. Signatures and bodies omitted.\n",
            None => "\nSurface unavailable; rescan with --symbols to include export names.\n",
        });
        if input.surface.is_none() && input.symbols.is_some() {
            out.push_str("File export index fallback; public module entrypoints are unverified.\n");
        }
    }
    if names.is_empty() {
        out.push_str("\nNo production modules in this scan.\n");
    }
    let mut ordered: Vec<_> = layers.into_iter().collect();
    if options.reverse {
        ordered.reverse();
    }
    let mut stop = 0;
    for (depth, layer) in ordered {
        let _ = writeln!(out, "\n## Layer {depth}\n");
        for id in layer {
            stop += 1;
            let component = &components[id];
            let label = component
                .iter()
                .map(|&m| code(names[m]))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "{stop}. {label}");
            let indent = " ".repeat(stop.to_string().len() + 2);
            let reason = if fan_out[id].is_empty() {
                "imports nothing".into()
            } else {
                format!("imports {}", count(fan_out[id].len(), "module"))
            };
            let _ = writeln!(
                out,
                "{indent}Why here: imported by {}, {reason} outside this stop.",
                count(fan_in[id].len(), "module")
            );
            if component.len() > 1 {
                let first = component[0];
                let _ = writeln!(
                    out,
                    "{indent}Cycle: these {} modules read together; start at {} (lowest outbound: {}, including cycle peers).",
                    component.len(),
                    code(names[first]),
                    count(outbound[first].len(), "module")
                );
            }
            if options.agent {
                for &module in component {
                    let _ = writeln!(out, "\n{indent}Surface {}:", code(names[module]));
                    if let Some(exports) = surfaces.get(&module) {
                        for (file, name) in exports {
                            let _ = writeln!(out, "{indent}- {} — {}", code(name), code(file));
                        }
                    } else {
                        let _ = writeln!(out, "{indent}- No export names recorded.");
                    }
                }
            }
            out.push('\n');
        }
    }
    out.push_str("\nThis is a reading suggestion over saved imports; dynamic dependencies and unscanned files may be missing.\n");
    Ok(out)
}
