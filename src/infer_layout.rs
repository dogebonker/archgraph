//! Evidence-backed drafts of the rules supported by layout.yaml.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::{Result, bail};

use crate::model::{Edge, Link, ModuleClass, ScanInput};
use crate::rules::{Compiled, Rules};

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub min_potential_edges: usize,
    pub max_density: f64,
    pub max_rules: usize,
    pub min_slices: usize,
    pub min_share: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_potential_edges: 100,
            max_density: 0.01,
            max_rules: 20,
            min_slices: 5,
            min_share: 0.8,
        }
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

fn literal(path: &str) -> String {
    // Commas also need escaping when a literal is placed inside a brace union.
    globset::escape(path)
        .replace(',', "[,]")
        .replace('\\', "\\\\")
}

fn union(patterns: &[String]) -> String {
    match patterns {
        [one] => one.clone(),
        _ => format!("{{{}}}", patterns.join(",")),
    }
}

/// Compress only directories whose observed descendants all belong to this module.
fn patterns(
    files: &BTreeMap<&str, Option<&str>>,
    members: &BTreeMap<&str, Vec<&str>>,
) -> BTreeMap<String, Vec<String>> {
    let mut directories: BTreeMap<&str, BTreeSet<Option<&str>>> = BTreeMap::new();
    for (&path, &module) in files {
        for (at, _) in path.match_indices('/') {
            directories.entry(&path[..at]).or_default().insert(module);
        }
    }
    members
        .iter()
        .map(|(&module, paths)| {
            let mut globs = BTreeSet::new();
            for &path in paths {
                let directory = path
                    .match_indices('/')
                    .map(|(at, _)| &path[..at])
                    .find(|dir| {
                        directories[*dir].len() == 1 && directories[*dir].contains(&Some(module))
                    });
                globs.insert(match directory {
                    Some(dir) => format!("{}/**", literal(dir)),
                    None => literal(path),
                });
            }
            (module.to_owned(), globs.into_iter().collect())
        })
        .collect()
}

/// A drafted `from ↛ to` claim over the sibling slices `<root>/<name>/`.
struct SliceClaim<'a> {
    root: &'a str,
    from: &'a str,
    to: &'a str,
    bound: usize,
    held: usize,
    /// Slices where `to` imports `from`.
    imports: usize,
    hits: Vec<(String, Vec<Link>)>,
}

/// The role of `rest`, the path below a slice folder: its first folder, or a file name cut at
/// its first `.` or `_`.
fn role(rest: &str) -> &str {
    match rest.split_once('/') {
        Some((folder, _)) => folder,
        None => rest.split(['.', '_']).next().unwrap_or(rest),
    }
}

fn role_glob(root: &str, role: &str) -> String {
    let root = if root.is_empty() {
        String::new()
    } else {
        format!("{}/", literal(root))
    };
    format!("{root}<name>/{}{{/**,.*,_*}}", literal(role))
}

/// Claims `B ↛ A` where A imports B in at least `min_slices` sibling slices and `B ↛ A` binds at
/// least `min_slices` and holds in at least `min_share` of them. Counts come from
/// `Compiled::forbid_slices`, as `check` takes them.
fn slice_claims<'a>(input: &'a ScanInput, options: Options) -> Result<Vec<SliceClaim<'a>>> {
    // root -> name -> roles.
    let mut families: BTreeMap<&str, BTreeMap<&str, BTreeSet<&str>>> = BTreeMap::new();
    for file in &input.files {
        let path = file.path.as_str();
        let starts = std::iter::once(0).chain(path.match_indices('/').map(|(at, _)| at + 1));
        for start in starts {
            let root = &path[..start.saturating_sub(1)];
            let Some((name, rest)) = path[start..].split_once('/') else {
                break;
            };
            let role = role(rest);
            if role.is_empty()
                || [root, name, role]
                    .iter()
                    .any(|s| s.contains(|c: char| c.is_control() || matches!(c, '<' | '>')))
            {
                continue;
            }
            families
                .entry(root)
                .or_default()
                .entry(name)
                .or_default()
                .insert(role);
        }
    }
    let mut pairs = Vec::new();
    for (&root, names) in &families {
        if names.len() < options.min_slices {
            continue;
        }
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for &role in names.values().flatten() {
            *seen.entry(role).or_default() += 1;
        }
        let roles: Vec<&str> = seen
            .into_iter()
            .filter(|&(_, n)| n >= options.min_slices)
            .map(|(role, _)| role)
            .collect();
        for &from in &roles {
            for &to in &roles {
                if from != to {
                    pairs.push((root, from, to));
                }
            }
        }
    }
    // ponytail: role pairs × slices × files, bounded by the slice prefilter; group by family if
    // drafts get slow on large monorepos.
    let rules = Rules {
        version: 1,
        forbid: pairs
            .iter()
            .map(|&(root, from, to)| crate::rules::Forbid {
                from: role_glob(root, from),
                to: role_glob(root, to),
                ..Default::default()
            })
            .collect(),
        ..Rules::default()
    };
    let files: Vec<&str> = input.files.iter().map(|f| f.path.as_str()).collect();
    let edges: Vec<&Edge> = input.edges.iter().collect();
    let evidence = Compiled::new(&rules)?.forbid_slices(&files, &edges, &BTreeMap::new());
    let index: BTreeMap<_, usize> = pairs.iter().enumerate().map(|(i, &p)| (p, i)).collect();
    let mut claims = Vec::new();
    for (i, &(root, from, to)) in pairs.iter().enumerate() {
        let bound: Vec<_> = evidence[i].iter().filter(|(_, s)| s.bound()).collect();
        let hits: Vec<(String, Vec<Link>)> = bound
            .iter()
            .filter(|(_, s)| !s.hits.is_empty())
            .map(|(name, s)| {
                (
                    name.as_deref().unwrap_or_default().to_owned(),
                    s.hits.clone(),
                )
            })
            .collect();
        let imports = evidence[index[&(root, to, from)]]
            .values()
            .filter(|s| !s.hits.is_empty())
            .count();
        let held = bound.len() - hits.len();
        if imports >= options.min_slices
            && bound.len() >= options.min_slices
            && held as f64 >= options.min_share * bound.len() as f64
        {
            claims.push(SliceClaim {
                root,
                from,
                to,
                bound: bound.len(),
                held,
                imports,
                hits,
            });
        }
    }
    claims.sort_by(|a, b| {
        b.bound
            .cmp(&a.bound)
            .then_with(|| (b.held * a.bound).cmp(&(a.held * b.bound)))
            .then_with(|| (a.root, a.from, a.to).cmp(&(b.root, b.from, b.to)))
    });
    Ok(claims)
}

/// Infer observations, not intent. Every generated rule remains subject to user review.
pub fn infer(input: &ScanInput, options: Options) -> Result<String> {
    crate::validate_scan(input)?;
    if options.min_potential_edges == 0 {
        bail!("minimum potential edges must be greater than zero");
    }
    if !options.max_density.is_finite() || !(0.0..=1.0).contains(&options.max_density) {
        bail!("maximum density must be finite and between 0 and 1");
    }
    if options.min_slices < 2 {
        bail!("minimum slices must be at least 2");
    }
    if !(options.min_share > 0.0 && options.min_share <= 1.0) {
        bail!("minimum share must be above 0 and at most 1");
    }
    for file in &input.files {
        if file.path.starts_with('/')
            || file
                .path
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
        {
            bail!(
                "inference requires relative file paths inside the scan root: {:?}",
                file.path
            );
        }
    }
    let mut files: BTreeMap<&str, Option<&str>> = input
        .files
        .iter()
        .map(|file| {
            (
                file.path.as_str(),
                (file.class != Some(ModuleClass::Tests)).then_some(file.module.as_str()),
            )
        })
        .collect();
    let mut members: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (&path, &module) in &files {
        if let Some(module) = module {
            members.entry(module).or_default().push(path);
        }
    }
    let names: Vec<&str> = members.keys().copied().collect();
    let ids: BTreeMap<&str, usize> = names.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut links: BTreeMap<(usize, usize), Vec<&Edge>> = BTreeMap::new();
    let mut omitted = 0;
    for edge in &input.edges {
        let written = edge.original_to.as_deref().unwrap_or(&edge.to);
        if files.get(edge.to.as_str()).is_some_and(Option::is_none) {
            continue;
        }
        let Some(Some(from)) = files.get(edge.from.as_str()) else {
            continue;
        };
        let from = ids[from];
        let Some(Some(to)) = files.get(written) else {
            if !files.contains_key(written) {
                omitted += 1;
            }
            continue;
        };
        let to = ids[to];
        if from != to {
            links.entry((from, to)).or_default().push(edge);
        }
    }
    // Unscanned facades must not accidentally fall under a generated directory glob.
    for edge in &input.edges {
        if let Some(written) = edge.original_to.as_deref() {
            files.entry(written).or_insert(None);
        }
    }
    let globs = patterns(&files, &members);
    let mut adjacency = vec![Vec::new(); names.len()];
    for &(from, to) in links.keys() {
        adjacency[from].push(to);
    }
    let (components, component_of, dag) = crate::graph::condense(&adjacency);
    let depth = crate::graph::depths(&dag);
    let cyclic: BTreeSet<usize> = components
        .iter()
        .filter(|c| c.len() > 1)
        .flatten()
        .copied()
        .collect();
    let mut levels: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for module in 0..names.len() {
        if !cyclic.contains(&module) {
            levels
                .entry(depth[component_of[module]])
                .or_default()
                .push(module);
        }
    }
    let mut out = String::from(
        "# DRAFT: inferred observations, not architectural intent. Review and edit before adoption.\n# Folder names do not establish intent. Only this scan's files and imports are evidence.\n# Exclusions and unresolved/dynamic imports limit absence claims; tests are ignored; external rules require import accounting.\n# All import kinds are included because layout rules apply to all kinds.\nversion: 1\n",
    );
    writeln!(
        out,
        "# Omitted {omitted} dependency edges with unscanned written destinations; their paths block glob compression."
    )?;
    let mut unresolved: Vec<Vec<&str>> = components
        .iter()
        .filter(|c| c.len() > 1)
        .map(|c| {
            let mut names: Vec<&str> = c.iter().map(|&i| names[i]).collect();
            names.sort_unstable();
            names
        })
        .collect();
    unresolved.sort();
    for cycle in unresolved {
        writeln!(
            out,
            "# unresolved: cycle between {}",
            cycle
                .iter()
                .map(|name| quoted(name))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    out.push_str("modules:");
    if names.is_empty() {
        out.push_str(" []\n");
    } else {
        out.push('\n');
    }
    for (i, &name) in names.iter().enumerate() {
        if cyclic.contains(&i) {
            writeln!(
                out,
                "  # {} files; unresolved cycle, grouping only",
                members[name].len()
            )?;
        } else {
            writeln!(
                out,
                "  # {} files; inferred top-down layer {}",
                members[name].len(),
                depth[component_of[i]]
            )?;
        }
        writeln!(
            out,
            "  - id: {}\n    path: {}",
            quoted(name),
            quoted(&union(&globs[name]))
        )?;
    }
    out.push_str(
        "# Top-down layers: 0 means no incoming module imports; larger ranks are dependencies.\n",
    );
    for (&rank, modules) in &levels {
        writeln!(
            out,
            "# Layer {rank}: {}",
            modules
                .iter()
                .map(|&i| quoted(names[i]))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    out.push_str(
        "# Executable layout layers run foundation-first, so dependencies are allowed.\nlayers:",
    );
    if levels.is_empty() {
        out.push_str(" []\n");
    } else {
        out.push('\n');
    }
    for (&rank, modules) in levels.iter().rev() {
        let patterns: Vec<String> = modules
            .iter()
            .flat_map(|&i| globs[names[i]].iter().cloned())
            .collect();
        let modules = modules
            .iter()
            .map(|&i| quoted(names[i]))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "  # Top-down layer {rank}: {modules}\n  - {}",
            quoted(&union(&patterns))
        )?;
    }
    let mut external_proposals = Vec::new();
    if let Some(imports) = &input.imports {
        let packages: BTreeSet<_> = imports
            .iter()
            .filter_map(|i| i.external.as_deref())
            .collect();
        for (&rank, modules) in &levels {
            let layer: BTreeSet<_> = modules.iter().map(|&i| names[i]).collect();
            let count: usize = modules.iter().map(|&i| members[names[i]].len()).sum();
            let potential = count
                .checked_mul(packages.len())
                .ok_or_else(|| anyhow::anyhow!("potential package edge count exceeds usize"))?;
            let uncertain_or_external = imports.iter().any(|import| {
                files
                    .get(import.from.as_str())
                    .and_then(|m| *m)
                    .is_some_and(|m| layer.contains(m))
                    && (import.external.is_some() || import.unresolved.is_some())
            });
            if potential >= options.min_potential_edges && !uncertain_or_external {
                let patterns: Vec<_> = modules
                    .iter()
                    .flat_map(|&i| globs[names[i]].iter().cloned())
                    .collect();
                external_proposals.push((rank, union(&patterns), count, packages.len(), potential));
            }
        }
    } else {
        out.push_str("# Package rules unavailable: no import accounting; rescan before inferring external boundaries.\n");
    }
    let mut claims = slice_claims(input, options)?;
    let claims_eligible = claims.len();
    claims.truncate(options.max_rules);
    let external_eligible = external_proposals.len();
    external_proposals.truncate(options.max_rules - claims.len());
    let internal_limit = options.max_rules - claims.len() - external_proposals.len();
    let mut proposals: Vec<(usize, usize, usize, usize, f64)> = Vec::new();
    let mut eligible_count = 0;
    for from in 0..names.len() {
        if cyclic.contains(&from) || members[names[from]].len() < 2 {
            continue;
        }
        for to in 0..names.len() {
            if from == to
                || cyclic.contains(&to)
                || members[names[to]].len() < 2
                || depth[component_of[from]] > depth[component_of[to]]
            {
                continue;
            }
            let potential = members[names[from]]
                .len()
                .checked_mul(members[names[to]].len())
                .ok_or_else(|| anyhow::anyhow!("potential edge count exceeds usize"))?;
            if potential < options.min_potential_edges {
                continue;
            }
            let observed = links
                .get(&(from, to))
                .map(|edges| {
                    edges
                        .iter()
                        .map(|e| (e.from.as_str(), e.original_to.as_deref().unwrap_or(&e.to)))
                        .collect::<BTreeSet<_>>()
                        .len()
                })
                .unwrap_or(0);
            let density = observed as f64 / potential as f64;
            if density <= options.max_density {
                eligible_count += 1;
                let position = proposals
                    .binary_search_by(|current| {
                        current
                            .4
                            .total_cmp(&density)
                            .then_with(|| potential.cmp(&current.2))
                            .then_with(|| (current.0, current.1).cmp(&(from, to)))
                    })
                    .unwrap_or_else(|position| position);
                if position < internal_limit {
                    proposals.insert(position, (from, to, potential, observed, density));
                    proposals.truncate(internal_limit);
                }
            }
        }
    }
    writeln!(
        out,
        "# {claims_eligible} eligible slice claims; {} omitted by --max-rules.",
        claims_eligible - claims.len()
    )?;
    writeln!(
        out,
        "# {} eligible sparse boundaries; {} omitted by --max-rules.",
        eligible_count,
        eligible_count - proposals.len()
    )?;
    writeln!(
        out,
        "# {external_eligible} eligible external-free layers; {} omitted by --max-rules.",
        external_eligible - external_proposals.len()
    )?;
    out.push_str("forbid:");
    if proposals.is_empty() && external_proposals.is_empty() && claims.is_empty() {
        out.push_str(" []\n");
    } else {
        out.push('\n');
    }
    for (i, claim) in claims.iter().enumerate() {
        let root = if claim.root.is_empty() {
            String::new()
        } else {
            format!("{}/", claim.root)
        };
        let (from, to, bound) = (claim.from, claim.to, claim.bound);
        writeln!(
            out,
            "  # {root}<name>: {to} imports {from} in {} of {bound} slices; {from} imports {to} in {} of {bound} (holds in {}).",
            claim.imports,
            bound - claim.held,
            claim.held
        )?;
        if !claim.hits.is_empty() {
            let names: Vec<&str> = claim.hits.iter().map(|(name, _)| name.as_str()).collect();
            writeln!(out, "  # Breaks in {}:", names.join(", "))?;
            for hit in claim.hits.iter().flat_map(|(_, hits)| hits) {
                writeln!(out, "  #   {} -> {}", quoted(&hit.from), quoted(&hit.to))?;
            }
        }
        writeln!(
            out,
            "  - id: infer-claim-{}\n    from: {}\n    to: {}\n    reason: {}",
            i + 1,
            quoted(&role_glob(claim.root, from)),
            quoted(&role_glob(claim.root, to)),
            quoted(&format!(
                "{from} does not import {to} (drafted from the graph; review intent)"
            ))
        )?;
    }
    for (rank, pattern, count, packages, potential) in external_proposals {
        writeln!(
            out,
            "  # Layer {rank}: 0 recorded external imports in {count} files; no unresolved imports.\n  # {packages} package identities observed repo-wide; {potential} file/package opportunities. Review intent.\n  - id: infer-no-external-layer-{rank}\n    from: {}\n    to: \"pkg:**\"\n    reason: Draft external-free layer; review intent",
            quoted(&pattern)
        )?;
    }
    for (i, &(from, to, potential, observed, density)) in proposals.iter().enumerate() {
        writeln!(
            out,
            "  # {} ({} files) -> {} ({} files): {observed}/{potential} distinct file pairs ({:.3}% density).",
            quoted(names[from]),
            members[names[from]].len(),
            quoted(names[to]),
            members[names[to]].len(),
            density * 100.0
        )?;
        if let Some(edges) = links.get(&(from, to)) {
            let mut evidence = BTreeSet::new();
            for edge in edges {
                evidence.insert(format!(
                    "{}{} -> {} [{}]",
                    quoted(&edge.from),
                    edge.line.map(|line| format!(":{line}")).unwrap_or_default(),
                    quoted(edge.original_to.as_deref().unwrap_or(&edge.to)),
                    edge.kind.as_str()
                ));
            }
            writeln!(
                out,
                "  # Adopt this rule and fix these dependencies ({} edge records):",
                edges.len()
            )?;
            for line in evidence {
                writeln!(out, "  #   {line}")?;
            }
        }
        writeln!(
            out,
            "  - id: infer-forbid-{}\n    from: {}\n    to: {}\n    reason: Draft inferred sparse boundary; review intent",
            i + 1,
            quoted(&union(&globs[names[from]])),
            quoted(&union(&globs[names[to]]))
        )?;
    }
    let rules: Rules = serde_yaml_ng::from_str(&out)?;
    Compiled::new(&rules)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeKind, File};
    use serde_json::json;

    fn input(modules: &[(&str, &[&str])], edges: &[(&str, &str)]) -> ScanInput {
        let mut input: ScanInput = serde_json::from_value(json!({
            "schema": "archgraph/v1.1", "kind": "scan",
            "source": {"language":"typescript", "collector":{"name":"fixture", "cruised":0}, "generated_at":"fixture"},
            "config": {"depth":1, "source_root":"", "excludes":[], "include_tests":false},
            "files": [], "edges": []
        })).unwrap();
        let mut rules = Rules {
            version: 1,
            ..Rules::default()
        };
        for &(module, paths) in modules {
            for &path in paths {
                input.files.push(File {
                    path: path.into(),
                    module: module.into(),
                    barrel: false,
                    class: None,
                    owners: Vec::new(),
                });
                rules.modules.push(crate::rules::ModuleRule {
                    id: module.into(),
                    path: literal(path),
                });
            }
        }
        input.config.rules = Some(crate::model::RulesRef {
            path: "fixture.yaml".into(),
            sha256: String::new(),
            known: None,
            doc: rules,
        });
        input.edges = edges
            .iter()
            .map(|&(from, to)| Edge {
                from: from.into(),
                to: to.into(),
                kind: EdgeKind::Static,
                line: Some(7),
                ..Edge::default()
            })
            .collect();
        input
    }

    fn parsed(input: &ScanInput, options: Options) -> (String, Rules) {
        let text = infer(input, options).unwrap();
        let rules = serde_yaml_ng::from_str(&text).unwrap();
        Compiled::new(&rules).unwrap();
        (text, rules)
    }

    #[test]
    fn external_free_layers_require_accounting_and_sufficient_certain_evidence() {
        let mut scan = input(
            &[
                ("adapter", &["adapter/a.ts"]),
                ("domain", &["domain/a.ts", "domain/b.ts"]),
            ],
            &[("adapter/a.ts", "domain/a.ts")],
        );
        let options = Options {
            min_potential_edges: 2,
            max_rules: 1,
            ..Options::default()
        };
        assert!(parsed(&scan, options).1.forbid.is_empty());
        scan.imports = Some(vec![crate::model::ImportRecord {
            from: "adapter/a.ts".into(),
            specifier: "typeorm".into(),
            external: Some("typeorm".into()),
            ..Default::default()
        }]);
        let (text, rules) = parsed(&scan, options);
        assert!(text.contains("2 file/package opportunities"));
        assert_eq!(rules.forbid.len(), 1);
        assert_eq!(rules.forbid[0].from, "domain/**");
        assert_eq!(rules.forbid[0].to, "pkg:**");
        let compiled = Compiled::new(&rules).unwrap();
        let mut edges = vec![Edge {
            from: "domain/a.ts".into(),
            to: "external:typescript:prisma".into(),
            ..Default::default()
        }];
        compiled.decide_with_externals(
            &mut edges,
            &scan
                .files
                .iter()
                .map(|f| (f.path.clone(), f.module.clone()))
                .collect(),
            &[("external:typescript:prisma".into(), "prisma".into())]
                .into_iter()
                .collect(),
        );
        assert_eq!(edges[0].verdict, Some(crate::model::Verdict::Forbidden));
        assert!(
            parsed(
                &scan,
                Options {
                    max_rules: 0,
                    ..options
                }
            )
            .1
            .forbid
            .is_empty()
        );
        assert!(
            parsed(
                &scan,
                Options {
                    min_potential_edges: 3,
                    ..options
                }
            )
            .1
            .forbid
            .is_empty()
        );
        scan.imports
            .as_mut()
            .unwrap()
            .push(crate::model::ImportRecord {
                from: "domain/a.ts".into(),
                specifier: "unknown".into(),
                unresolved: Some(crate::model::ImportReason::Missing),
                ..Default::default()
            });
        assert!(parsed(&scan, options).1.forbid.is_empty());
        scan.imports = Some(Vec::new());
        assert!(parsed(&scan, options).1.forbid.is_empty());
    }

    #[test]
    fn diamond_longest_path_is_foundation_first_and_isolates_share_rank_zero() {
        let scan = input(
            &[
                ("app", &["app/a"]),
                ("left", &["left/a"]),
                ("right", &["right/a"]),
                ("domain", &["domain/a"]),
                ("alone", &["alone/a"]),
            ],
            &[
                ("app/a", "left/a"),
                ("app/a", "right/a"),
                ("right/a", "left/a"),
                ("left/a", "domain/a"),
            ],
        );
        let (_, rules) = parsed(&scan, Options::default());
        assert_eq!(
            rules.layers,
            ["domain/**", "left/**", "right/**", "{alone/**,app/**}"]
        );
        let mut edges = scan.edges.clone();
        let module_of = scan
            .files
            .iter()
            .map(|f| (f.path.clone(), f.module.clone()))
            .collect();
        Compiled::new(&rules)
            .unwrap()
            .decide(&mut edges, &module_of);
        assert!(
            edges
                .iter()
                .all(|e| e.verdict == Some(crate::model::Verdict::Allowed))
        );
    }

    #[test]
    fn cycles_are_unresolved_but_influence_downstream_depth() {
        let mut scan = input(
            &[("a", &["a/x"]), ("b", &["b/x"]), ("leaf", &["leaf/x"])],
            &[("a/x", "b/x"), ("b/x", "a/x"), ("b/x", "leaf/x")],
        );
        scan.edges[0].kind = EdgeKind::Dynamic;
        scan.edges[1].kind = EdgeKind::Type;
        let (text, rules) = parsed(&scan, Options::default());
        assert!(text.contains("# unresolved: cycle between \"a\", \"b\""));
        assert!(text.contains("Top-down layer 1: \"leaf\""));
        assert_eq!(rules.modules.len(), 3);
        assert_eq!(rules.layers, ["leaf/**"]);
        assert!(rules.forbid.is_empty());
    }

    #[test]
    fn sparse_boundaries_count_unique_pairs_and_list_violations() {
        let a: Vec<String> = (0..20).map(|i| format!("a/{i}")).collect();
        let b: Vec<String> = (0..25).map(|i| format!("b/{i}")).collect();
        let aa: Vec<&str> = a.iter().map(String::as_str).collect();
        let bb: Vec<&str> = b.iter().map(String::as_str).collect();
        let scan = input(
            &[("a", &aa), ("b", &bb)],
            &[("a/0", "b/0"), ("a/1", "b/1"), ("a/0", "b/0")],
        );
        let (text, rules) = parsed(&scan, Options::default());
        assert_eq!(rules.forbid.len(), 1);
        assert!(text.contains("2/500 distinct file pairs (0.400% density)"));
        assert!(text.contains("\"a/0\":7 -> \"b/0\" [runtime]"));
        assert!(
            parsed(
                &scan,
                Options {
                    min_potential_edges: 501,
                    ..Options::default()
                }
            )
            .1
            .forbid
            .is_empty()
        );
        let empty = input(&[("a", &aa), ("b", &bb)], &[]);
        let (text, rules) = parsed(
            &empty,
            Options {
                max_rules: 1,
                ..Options::default()
            },
        );
        assert_eq!(rules.forbid.len(), 1);
        assert!(text.contains("2 eligible sparse boundaries; 1 omitted"));
    }

    #[test]
    fn bounded_proposals_keep_the_full_ranking_and_total_eligible_count() {
        let scan = input(
            &[
                ("a", &["a/0", "a/1"]),
                ("b", &["b/0", "b/1", "b/2"]),
                ("c", &["c/0", "c/1", "c/2", "c/3"]),
                ("d", &["d/0", "d/1", "d/2", "d/3", "d/4"]),
            ],
            &[("a/0", "b/0"), ("c/0", "d/0")],
        );
        let options = Options {
            min_potential_edges: 1,
            max_density: 1.0,
            max_rules: usize::MAX,
            ..Options::default()
        };
        let (_, all) = parsed(&scan, options);
        assert!(all.forbid.len() > 3);
        // Zero density wins first, then larger potential, then module names.
        assert_eq!(
            (&all.forbid[0].from, &all.forbid[0].to),
            (&"b/**".to_owned(), &"d/**".to_owned())
        );
        for max_rules in [0, 3, all.forbid.len() - 1] {
            let (text, bounded) = parsed(
                &scan,
                Options {
                    max_rules,
                    ..options
                },
            );
            assert_eq!(
                serde_json::to_value(&bounded.forbid).unwrap(),
                serde_json::to_value(&all.forbid[..max_rules]).unwrap()
            );
            assert!(text.contains(&format!(
                "{} eligible sparse boundaries; {} omitted",
                all.forbid.len(),
                all.forbid.len() - max_rules
            )));
        }
    }

    /// `internal/<name>/{handler,service,repository}.go` per name, `handler.go -> service.go` in
    /// each, and `service.go -> handler.go` in each of `back`.
    fn slices(names: &[&str], back: &[&str]) -> ScanInput {
        let paths: Vec<Vec<String>> = names
            .iter()
            .map(|n| {
                ["handler", "service", "repository"]
                    .map(|role| format!("internal/{n}/{role}.go"))
                    .to_vec()
            })
            .collect();
        let paths: Vec<Vec<&str>> = paths
            .iter()
            .map(|p| p.iter().map(String::as_str).collect())
            .collect();
        let modules: Vec<(&str, &[&str])> = names
            .iter()
            .zip(&paths)
            .map(|(&n, p)| (n, &p[..]))
            .collect();
        let mut edges: Vec<(String, String)> = names
            .iter()
            .map(|n| {
                (
                    format!("internal/{n}/handler.go"),
                    format!("internal/{n}/service.go"),
                )
            })
            .collect();
        edges.extend(back.iter().map(|n| {
            (
                format!("internal/{n}/service.go"),
                format!("internal/{n}/handler.go"),
            )
        }));
        let edges: Vec<(&str, &str)> = edges
            .iter()
            .map(|(from, to)| (from.as_str(), to.as_str()))
            .collect();
        input(&modules, &edges)
    }

    #[test]
    fn slice_claims_need_a_direction_that_holds_across_enough_siblings() {
        let five = ["a", "b", "c", "d", "e"];
        let (text, rules) = parsed(&slices(&five, &["e"]), Options::default());
        assert_eq!(rules.forbid.len(), 1, "{text}");
        let claim = &rules.forbid[0];
        assert_eq!(claim.id.as_deref(), Some("infer-claim-1"));
        assert_eq!(claim.from, "internal/<name>/service{/**,.*,_*}");
        assert_eq!(claim.to, "internal/<name>/handler{/**,.*,_*}");
        assert_eq!(
            claim.reason.as_deref(),
            Some("service does not import handler (drafted from the graph; review intent)")
        );
        assert!(text.contains(
            "  # internal/<name>: handler imports service in 5 of 5 slices; service imports handler in 1 of 5 (holds in 4).\n  # Breaks in e:\n  #   \"internal/e/service.go\" -> \"internal/e/handler.go\"\n  - id: infer-claim-1\n"
        ), "{text}");
        assert!(text.contains("# 1 eligible slice claims; 0 omitted by --max-rules.\n"));
        let (text, rules) = parsed(
            &slices(&five, &["e"]),
            Options {
                max_rules: 0,
                ..Options::default()
            },
        );
        assert!(rules.forbid.is_empty());
        assert!(text.contains("# 1 eligible slice claims; 1 omitted by --max-rules.\n"));
        assert!(
            parsed(&slices(&five, &["d", "e"]), Options::default())
                .1
                .forbid
                .is_empty()
        );
        let four = ["a", "b", "c", "d"];
        assert!(
            parsed(&slices(&four, &[]), Options::default())
                .1
                .forbid
                .is_empty()
        );
        let (text, rules) = parsed(
            &slices(&four, &[]),
            Options {
                min_slices: 4,
                ..Options::default()
            },
        );
        assert_eq!(rules.forbid.len(), 1);
        assert!(text.contains(
            "  # internal/<name>: handler imports service in 4 of 4 slices; service imports handler in 0 of 4 (holds in 4).\n  - id: infer-claim-1\n"
        ), "{text}");
    }

    #[test]
    fn globs_preserve_custom_membership_and_escape_literals() {
        let mut scan = input(
            &[
                ("parent", &["src/a.ts", "root.ts"]),
                ("child", &["src/nested/b.ts"]),
                ("literal", &["[x]{y}*,.ts"]),
            ],
            &[],
        );
        scan.files.push(File {
            path: "src/test/t.ts".into(),
            module: "tests::../src".into(),
            barrel: false,
            class: Some(ModuleClass::Tests),
            owners: Vec::new(),
        });
        let (text, rules) = parsed(&scan, Options::default());
        assert!(text.contains("src/a.ts"));
        assert!(!rules.layers.iter().any(|g| g == "src/**"));
        let compiled = Compiled::new(&rules).unwrap();
        for file in scan.files.iter().filter(|f| f.class.is_none()) {
            assert_eq!(
                compiled.module_of(&file.path).as_deref(),
                Some(file.module.as_str())
            );
        }
        assert!(compiled.module_of("src/test/t.ts").is_none());
    }

    #[test]
    fn unscanned_written_barrel_blocks_globs_and_is_omitted_from_dag() {
        let mut scan = input(&[("a", &["a/x"]), ("b", &["b/x"])], &[("a/x", "b/x")]);
        scan.edges[0].original_to = Some("b/index.ts".into());
        let (text, rules) = parsed(&scan, Options::default());
        assert!(text.contains("Omitted 1 dependency edges"));
        assert_eq!(rules.layers, ["{a/**,b/x}"]);
    }

    #[test]
    fn invalid_options_and_unrepresentable_capture_ids_fail() {
        let scan = input(&[("a", &["a/x"])], &[]);
        assert!(
            infer(
                &scan,
                Options {
                    max_density: f64::NAN,
                    ..Options::default()
                }
            )
            .is_err()
        );
        assert!(
            infer(
                &scan,
                Options {
                    min_potential_edges: 0,
                    ..Options::default()
                }
            )
            .is_err()
        );
        for (min_slices, min_share) in [(1, 0.8), (5, 0.0), (5, 1.1), (5, f64::NAN)] {
            let options = Options {
                min_slices,
                min_share,
                ..Options::default()
            };
            assert!(infer(&scan, options).is_err());
        }
        let mut scan = scan;
        scan.files[0].module = "<bad>".into();
        scan.config.rules.as_mut().unwrap().doc.modules[0].id = "<bad>".into();
        assert!(infer(&scan, Options::default()).is_err());
    }
}
