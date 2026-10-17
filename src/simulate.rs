//! Proposed moves: rename files, rerun the pipeline, compare.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use crate::model::{
    Changes, Document, Edge, FindingSimulation, ModuleSummary, Move, MovedFile, SCHEMA, Simulation,
    Summary, Verdict,
};
use crate::rules::Compiled;

pub fn summary(doc: &Document) -> Summary {
    let count = |v| doc.edges.iter().filter(|e| e.verdict == Some(v)).count();
    Summary {
        forbidden: count(Verdict::Forbidden),
        unplanned: count(Verdict::Unplanned),
        cycles: doc.cycles.len(),
        type_only_cycles: doc.type_only_cycles.len(),
        modules: doc
            .modules
            .iter()
            .map(|m| {
                let summary = ModuleSummary {
                    cohesion: m.cohesion,
                    ca: m.ca,
                    ce: m.ce,
                    weighted_ca: m.weighted_ca,
                    weighted_ce: m.weighted_ce,
                    instability: m.instability,
                };
                (m.id.clone(), summary)
            })
            .collect(),
    }
}

/// `doc` rebuilt (without findings) with every moved file at its destination; the destination
/// path decides the new module, layer and verdicts.
pub fn apply(doc: &Document, rules: Option<&Compiled>, moves: &[Move]) -> Result<Document> {
    let exists = |path: &str| doc.files.iter().any(|f| f.path == path);
    let (mut sources, mut destinations) = (HashSet::new(), HashSet::new());
    for m in moves {
        if !exists(&m.file) {
            bail!("{} is not a file in the scan", m.file);
        }
        if exists(&m.to) {
            bail!("{} already exists", m.to);
        }
        if !sources.insert(m.file.as_str()) {
            bail!("{} is moved twice", m.file);
        }
        if !destinations.insert(m.to.as_str()) {
            bail!("two files are moved to {}", m.to);
        }
        if m.to.starts_with('/')
            || m.to
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            bail!("{} must be a relative path inside the root", m.to);
        }
    }
    let renamed: HashMap<&str, &str> = moves
        .iter()
        .map(|m| (m.file.as_str(), m.to.as_str()))
        .collect();
    let rename = |path: &str| renamed.get(path).copied().unwrap_or(path).to_owned();
    let mut config = doc.config.clone();
    config.file_metadata = doc
        .files
        .iter()
        .map(|f| {
            (
                rename(&f.path),
                crate::model::NodeMetadata {
                    barrel: f.barrel,
                    test: f.class == Some(crate::model::ModuleClass::Tests),
                },
            )
        })
        .collect();
    let files = doc.files.iter().map(|f| rename(&f.path)).collect();
    let edges = doc
        .edges
        .iter()
        .map(|e| Edge {
            from: rename(&e.from),
            to: rename(&e.to),
            original_to: e.original_to.as_deref().map(rename),
            ..e.clone()
        })
        .collect();
    let entries = doc.package_entries.iter().map(|p| rename(p)).collect();
    let owners = doc
        .config
        .codeowners
        .as_ref()
        .map(crate::codeowners::compile)
        .transpose()?;
    Ok(crate::build(
        doc.source.clone(),
        config,
        files,
        edges,
        entries,
        rules,
        owners.as_ref(),
    ))
}

/// `after` against `before`, as findings carry it: the counts, and the modules whose metrics
/// changed.
pub fn changes(before: &Document, after: &Document) -> FindingSimulation {
    let (was, is) = (&before.summary.modules, &after.summary.modules);
    let mut modules: std::collections::BTreeMap<String, Option<ModuleSummary>> = is
        .iter()
        .filter(|(id, m)| was.get(*id) != Some(m))
        .map(|(id, m)| (id.clone(), Some(m.clone())))
        .collect();
    for id in was.keys().filter(|id| !is.contains_key(*id)) {
        modules.insert(id.clone(), None);
    }
    FindingSimulation {
        after: Changes {
            forbidden: after.summary.forbidden,
            unplanned: after.summary.unplanned,
            cycles: after.summary.cycles,
            type_only_cycles: after.summary.type_only_cycles,
            modules,
        },
    }
}

/// `changes` of `moves`.
pub fn compare(
    doc: &Document,
    rules: Option<&Compiled>,
    moves: &[Move],
) -> Result<FindingSimulation> {
    Ok(changes(doc, &apply(doc, rules, moves)?))
}

/// The `simulate` document for one move.
pub fn simulate(
    doc: &Document,
    rules: Option<&Compiled>,
    file: &str,
    to: &str,
) -> Result<Simulation> {
    let moves = [Move {
        file: file.to_owned(),
        to: to.to_owned(),
    }];
    let after = apply(doc, rules, &moves)?;
    // A moved edge is the same edge under its old name.
    let old_name = |path: &str| {
        if path == to {
            file.to_owned()
        } else {
            path.to_owned()
        }
    };
    let new_name = |path: &str| {
        if path == file {
            to.to_owned()
        } else {
            path.to_owned()
        }
    };
    let forbidden = |d: &Document| -> Vec<Edge> {
        d.edges
            .iter()
            .filter(|e| e.verdict == Some(Verdict::Forbidden))
            .cloned()
            .collect()
    };
    let (was, is) = (forbidden(doc), forbidden(&after));
    let was_set: HashSet<(String, String)> =
        was.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
    let is_set: HashSet<(String, String)> =
        is.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
    let module = after
        .files
        .iter()
        .find(|f| f.path == to)
        .map(|f| f.module.clone())
        .unwrap_or_default();
    Ok(Simulation {
        schema: SCHEMA,
        kind: "simulation",
        move_to: MovedFile {
            file: file.to_owned(),
            to: to.to_owned(),
            module,
        },
        before: doc.summary.clone(),
        after: after.summary.clone(),
        new_forbidden: is
            .into_iter()
            .filter(|e| !was_set.contains(&(old_name(&e.from), old_name(&e.to))))
            .collect(),
        removed_forbidden: was
            .into_iter()
            .filter(|e| !is_set.contains(&(new_name(&e.from), new_name(&e.to))))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::model::{Collector, Config, Language, Move, Source};

    fn doc() -> crate::model::Document {
        let source = Source {
            language: Language::Typescript,
            collector: Collector {
                name: "test".into(),
                version: None,
                cruised: 3,
                unresolved: None,
                type_marking: None,
                main_packages: Vec::new(),
                bound_files: Vec::new(),
                go_modules: Vec::new(),
            },
            git: None,
            generated_at: String::new(),
        };
        let config = Config {
            depth: 1,
            package_modules: false,
            source_root: ".".into(),
            excludes: Vec::new(),
            include_tests: false,
            include_generated: false,
            includes: Vec::new(),
            excluded_files: Vec::new(),
            explicit_excludes: None,
            all_kinds: false,
            no_default_excludes: false,
            rules: None,
            codeowners: None,
            collection: None,
            file_metadata: Default::default(),
            legacy_module_classes: false,
        };
        let files = ["a/x.ts", "a/y.ts", "b/z.ts"].map(String::from).to_vec();
        crate::build(source, config, files, Vec::new(), Vec::new(), None, None)
    }

    fn mv(file: &str, to: &str) -> Move {
        Move {
            file: file.into(),
            to: to.into(),
        }
    }

    #[test]
    fn apply_rejects_a_file_moved_twice_or_two_files_to_one_place() {
        let doc = doc();
        let twice = apply(
            &doc,
            None,
            &[mv("a/x.ts", "c/x.ts"), mv("a/x.ts", "d/x.ts")],
        );
        assert!(twice.unwrap_err().to_string().contains("a/x.ts"));
        let same = apply(
            &doc,
            None,
            &[mv("a/x.ts", "c/n.ts"), mv("a/y.ts", "c/n.ts")],
        );
        assert!(same.unwrap_err().to_string().contains("c/n.ts"));
        assert!(
            apply(
                &doc,
                None,
                &[mv("a/x.ts", "c/x.ts"), mv("a/y.ts", "c/y.ts")]
            )
            .is_ok()
        );
    }
}
