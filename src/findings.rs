//! Recommendation analyses over a built document. Thresholds are tuning knobs, not truths.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::model::{
    Cluster, Component, Document, Evidence, Finding, FindingKind, Language, Move, Suggestion,
    Verdict,
};
use crate::modules;
use crate::rules::Compiled;
use crate::simulate;

/// A module imported by at least this many others is a shared sink.
const SINK_CA: usize = 3;
/// A file needs at least this many edges to one other module before it can look misplaced.
const MISPLACED_MIN_EDGES: usize = 2;
/// A file nobody imports is an entry point a framework locates by path (`page.tsx`, `route.ts`).
const MISPLACED_MIN_IMPORTERS: usize = 1;
/// A file imported from more other modules than this is a shared provider, not misplaced.
const MISPLACED_MAX_IMPORTER_MODULES: usize = 1;
/// Stable dependencies principle: flag A -> B when B is this much less stable than A.
const SDP_MARGIN: f64 = 0.25;
/// Files with a degree above this percentile are hubs that would glue every cluster together.
const HUB_PERCENTILE: f64 = 0.9;
/// Label propagation usually settles in a handful of rounds; this bounds oscillation.
const LP_MAX_ITERATIONS: usize = 50;
/// The clustering must beat the folders' modularity by this much to be worth reporting.
const SEAM_MIN_GAIN: f64 = 0.05;
/// Clusters must hold at least this share of the non-barrel files: a few clustered files among
/// many loose ones propose no boundary.
const SEAM_MIN_COVERAGE: f64 = 0.5;
/// Below either size, findings carry little signal.
const SMALL_FILES: usize = 20;
const SMALL_MODULES: usize = 3;
/// Absorbs float error in threshold comparisons of rounded ratios.
const EPSILON: f64 = 1e-9;

struct Graph<'a> {
    doc: &'a Document,
    rules: Option<&'a Compiled>,
    module_of: BTreeMap<&'a str, &'a str>,
    /// Barrel-attributed file pairs.
    pairs: BTreeSet<(String, String)>,
    raw: HashSet<(&'a str, &'a str)>,
    /// Longest common directory of each module's files: where a file moved into it goes.
    dirs: BTreeMap<&'a str, String>,
}

impl Graph<'_> {
    fn via_barrel<'p>(&self, mut used: impl Iterator<Item = &'p (String, String)>) -> bool {
        used.any(|(a, b)| !self.raw.contains(&(a.as_str(), b.as_str())))
    }

    /// The move of `file` into `module`'s directory, unless a file of that name is there.
    fn move_into(&self, file: &str, module: &str) -> Option<Move> {
        let to = self.path_in(module, file.rsplit('/').next().unwrap_or(file));
        (!self.module_of.contains_key(to.as_str())).then(|| Move {
            file: file.to_owned(),
            to,
        })
    }

    /// `move_into`, or when the name is taken, the same move under `moved-<name>` (repeated
    /// until free): what to simulate when there is no move to suggest.
    fn free_move(&self, file: &str, module: &str) -> Move {
        let mut name = file.rsplit('/').next().unwrap_or(file).to_owned();
        while self
            .module_of
            .contains_key(self.path_in(module, &name).as_str())
        {
            name = format!("moved-{name}");
        }
        Move {
            file: file.to_owned(),
            to: self.path_in(module, &name),
        }
    }

    fn path_in(&self, module: &str, name: &str) -> String {
        match self.dirs[module].as_str() {
            "" => name.to_owned(),
            dir => format!("{dir}/{name}"),
        }
    }
}

pub fn find(doc: &Document, rules: Option<&Compiled>) -> Vec<Finding> {
    let module_of: BTreeMap<&str, &str> = doc
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.module.as_str()))
        .collect();
    let barrels: BTreeSet<String> = doc
        .files
        .iter()
        .filter(|f| f.barrel)
        .map(|f| f.path.clone())
        .collect();
    let mut dirs: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &doc.files {
        let dir = f.path.rsplit_once('/').map_or("", |(d, _)| d);
        let common = dirs
            .entry(&f.module)
            .or_insert_with(|| dir.split('/').collect());
        let shared = common
            .iter()
            .zip(dir.split('/'))
            .take_while(|(a, b)| *a == b)
            .count();
        common.truncate(shared);
    }
    let g = Graph {
        doc,
        rules,
        pairs: modules::attribute(&doc.edges, &barrels),
        raw: doc
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect(),
        dirs: dirs.into_iter().map(|(m, d)| (m, d.join("/"))).collect(),
        module_of,
    };
    let mut findings = Vec::new();
    sink_split(&g, &mut findings);
    misplaced_file(&g, &mut findings);
    for c in &doc.cycles {
        findings.push(finding(
            FindingKind::Cycle,
            c.members.join(", "),
            Evidence::Cycle(c.clone()),
        ));
    }
    internals_reach(&g, &mut findings);
    hidden_coupling(&g, &mut findings);
    unstable_dependency(&g, &mut findings);
    seam(&g, &mut findings);
    let modules = doc.modules.len();
    if doc.files.len() < SMALL_FILES || modules < SMALL_MODULES {
        let evidence = Evidence::SmallRepo {
            files: doc.files.len(),
            modules,
        };
        findings.push(finding(FindingKind::SmallRepo, ".".into(), evidence));
    }
    findings.sort_by(|a, b| (a.kind, &a.subject).cmp(&(b.kind, &b.subject)));
    findings
}

fn finding(kind: FindingKind, subject: String, evidence: Evidence) -> Finding {
    Finding {
        kind,
        subject,
        evidence,
        suggestion: None,
        simulation: None,
    }
}

/// Components of the bipartite graph between a sink's files and the modules importing them.
fn sink_split(g: &Graph, out: &mut Vec<Finding>) {
    for module in g.doc.modules.iter().filter(|m| m.ca >= SINK_CA) {
        let sink = module.id.as_str();
        let into: Vec<&(String, String)> = g
            .pairs
            .iter()
            .filter(|(_, to)| g.module_of[to.as_str()] == sink)
            .collect();
        let outside = |from: &str| g.module_of[from] != sink;
        let imported_from_outside: HashSet<&str> = into
            .iter()
            .filter(|(from, _)| outside(from))
            .map(|(_, to)| to.as_str())
            .collect();
        // Nodes are files of the sink and consumer modules, tagged to keep the namespaces apart.
        let mut links: BTreeMap<(bool, &str), Vec<(bool, &str)>> = BTreeMap::new();
        for (from, to) in &into {
            let other = if outside(from) {
                (true, g.module_of[from.as_str()])
            } else if !imported_from_outside.contains(to.as_str()) {
                (false, from.as_str())
            } else {
                continue;
            };
            let file = (false, to.as_str());
            links.entry(file).or_default().push(other);
            links.entry(other).or_default().push(file);
        }
        let mut seen = HashSet::new();
        let mut components = Vec::new();
        for &start in links.keys() {
            if !seen.insert(start) {
                continue;
            }
            let (mut files, mut consumers) = (Vec::new(), Vec::new());
            let mut stack = vec![start];
            while let Some(node @ (is_module, name)) = stack.pop() {
                if is_module {
                    consumers.push(name.to_owned());
                } else {
                    files.push(name.to_owned());
                }
                stack.extend(links[&node].iter().filter(|n| seen.insert(**n)));
            }
            if consumers.is_empty() {
                continue;
            }
            files.sort();
            consumers.sort();
            let moves = match consumers.as_slice() {
                [only] => files.iter().filter_map(|f| g.move_into(f, only)).collect(),
                _ => Vec::new(),
            };
            components.push(Component {
                files,
                consumers,
                moves,
            });
        }
        if components.len() < 2 {
            continue;
        }
        // Two files of one name cannot both move into one directory: neither is suggested.
        let mut targets: BTreeMap<String, usize> = BTreeMap::new();
        for m in components.iter().flat_map(|c| &c.moves) {
            *targets.entry(m.to.clone()).or_default() += 1;
        }
        for c in &mut components {
            c.moves.retain(|m| targets[&m.to] == 1);
        }
        components.sort_by(|a, b| a.files.cmp(&b.files));
        let moves: Vec<Move> = components.iter().flat_map(|c| c.moves.clone()).collect();
        let mut f = finding(
            FindingKind::SinkSplit,
            sink.to_owned(),
            Evidence::SinkSplit {
                ca: module.ca,
                via_barrel: g.via_barrel(into.iter().copied().filter(|(from, _)| outside(from))),
                components,
            },
        );
        if !moves.is_empty() {
            f.simulation = simulate::compare(g.doc, g.rules, &moves).ok();
        }
        out.push(f);
    }
}

/// An imported file with more edges to one other module than to its own, imported from at most
/// one other module, whose move would add no forbidden import or cycle, give the target module no
/// new dependency and raise the cohesion of the two modules taken together.
fn misplaced_file(g: &Graph, out: &mut Vec<Finding>) {
    let mut counts: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for (a, b) in &g.pairs {
        *counts
            .entry(a)
            .or_default()
            .entry(g.module_of[b.as_str()])
            .or_default() += 1;
        *counts
            .entry(b)
            .or_default()
            .entry(g.module_of[a.as_str()])
            .or_default() += 1;
    }
    let sizes: BTreeMap<&str, usize> = g
        .doc
        .modules
        .iter()
        .map(|m| (m.id.as_str(), m.files))
        .collect();
    for file in &g.doc.files {
        // A package's __init__.py cannot move without the package, and moving a module's only
        // file just renames the module.
        if file.barrel || file.path.ends_with("__init__.py") || sizes[file.module.as_str()] == 1 {
            continue;
        }
        let Some(by_module) = counts.get(file.path.as_str()) else {
            continue;
        };
        let importers: Vec<&str> = g
            .pairs
            .iter()
            .filter(|(_, b)| *b == file.path)
            .map(|(a, _)| g.module_of[a.as_str()])
            .collect();
        let importer_modules: BTreeSet<&str> = importers
            .iter()
            .copied()
            .filter(|m| *m != file.module)
            .collect();
        if importers.len() < MISPLACED_MIN_IMPORTERS
            || importer_modules.len() > MISPLACED_MAX_IMPORTER_MODULES
        {
            continue;
        }
        let own = by_module.get(file.module.as_str()).copied().unwrap_or(0);
        let Some((&target, &edges)) = by_module
            .iter()
            .filter(|(m, n)| **m != file.module && **n >= MISPLACED_MIN_EDGES && **n > own)
            .max_by_key(|(m, n)| (**n, Reverse(**m)))
        else {
            continue;
        };
        let path = file.path.as_str();
        let touches = |(a, b): &&(String, String)| {
            (a == path && g.module_of[b.as_str()] == target)
                || (b == path && g.module_of[a.as_str()] == target)
        };
        let mut f = finding(
            FindingKind::MisplacedFile,
            file.path.clone(),
            Evidence::MisplacedFile {
                own_module_edges: own,
                target_module: target.to_owned(),
                target_module_edges: edges,
                via_barrel: g.via_barrel(g.pairs.iter().filter(touches)),
            },
        );
        // ponytail: every proposed move reruns the whole pipeline; batch or make it incremental
        // if scans of large repositories with many misplaced files get slow.
        //
        // When the file's name is taken in the target directory there is no move to suggest, and
        // the check runs on a free name there instead.
        let suggested = g.move_into(path, target);
        let probe = suggested
            .clone()
            .unwrap_or_else(|| g.free_move(path, target));
        let Ok(after) = simulate::apply(g.doc, g.rules, std::slice::from_ref(&probe)) else {
            continue;
        };
        let (was, is) = (&g.doc.summary, &after.summary);
        let modules = [file.module.as_str(), target];
        // A file that only uses the target (a component calling into a library) would drag its
        // other dependencies into it.
        let ce = |s: &crate::model::Summary| s.modules.get(target).map_or(0, |m| m.ce);
        if is.forbidden > was.forbidden
            || is.cycles > was.cycles
            || ce(is) > ce(was)
            || cohesion(&after, &modules) <= cohesion(g.doc, &modules) + EPSILON
        {
            continue;
        }
        if let Some(m) = suggested {
            f.simulation = Some(simulate::changes(g.doc, &after));
            f.suggestion = Some(Suggestion { move_to: m });
        }
        out.push(f);
    }
}

/// Internal over internal plus outgoing edges of `modules` taken together.
fn cohesion(doc: &Document, modules: &[&str]) -> f64 {
    let (internal, total) = doc
        .modules
        .iter()
        .filter(|m| modules.contains(&m.id.as_str()))
        .fold((0, 0), |(i, t), m| {
            (
                i + m.internal_edges,
                t + m.internal_edges + m.outgoing_edges,
            )
        });
    if total == 0 {
        0.0
    } else {
        internal as f64 / total as f64
    }
}

/// With rules: imports forbidden by a `public` rule. Without, TypeScript only: cross-module
/// imports of a non-barrel file in a module that has a barrel.
fn internals_reach(g: &Graph, out: &mut Vec<Finding>) {
    let mut barrels: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for f in g.doc.files.iter().filter(|f| f.barrel) {
        barrels.entry(&f.module).or_default().push(f.path.clone());
    }
    let is_barrel: HashSet<&str> = barrels.values().flatten().map(String::as_str).collect();
    for e in &g.doc.edges {
        let module = g.module_of[e.to.as_str()];
        let entries = match g.rules {
            Some(rules)
                if e.verdict == Some(Verdict::Forbidden) && e.rule.as_deref() == Some("public") =>
            {
                rules.entries_for(&e.to).unwrap_or_default()
            }
            None if g.doc.source.language == Language::Typescript
                && g.module_of[e.from.as_str()] != module
                && !is_barrel.contains(e.to.as_str()) =>
            {
                match barrels.get(module) {
                    Some(b) => b.clone(),
                    None => continue,
                }
            }
            _ => continue,
        };
        out.push(finding(
            FindingKind::InternalsReach,
            format!("{} -> {}", e.from, e.to),
            Evidence::InternalsReach {
                from: e.from.clone(),
                to: e.to.clone(),
                module: module.to_owned(),
                entries,
            },
        ));
    }
}

/// Files in different modules that change together without importing each other.
fn hidden_coupling(g: &Graph, out: &mut Vec<Finding>) {
    let Some(cochange) = &g.doc.cochange else {
        return;
    };
    for pair in &cochange.pairs {
        let [a, b] = &pair.files;
        let (Some(&ma), Some(&mb)) = (g.module_of.get(a.as_str()), g.module_of.get(b.as_str()))
        else {
            continue;
        };
        let linked = |x: &str, y: &str| g.raw.contains(&(x, y));
        if ma == mb || linked(a, b) || linked(b, a) {
            continue;
        }
        out.push(finding(
            FindingKind::HiddenCoupling,
            format!("{a} <-> {b}"),
            Evidence::HiddenCoupling {
                files: pair.files.clone(),
                modules: [ma.to_owned(), mb.to_owned()],
                commits: pair.commits,
            },
        ));
    }
}

/// Over the barrel-attributed module pairs Ca and Ce count, so an import of a barrel is an
/// import of what it re-exports.
fn unstable_dependency(g: &Graph, out: &mut Vec<Finding>) {
    let instability: BTreeMap<&str, Option<f64>> = g
        .doc
        .modules
        .iter()
        .map(|m| (m.id.as_str(), m.instability))
        .collect();
    let mut imports: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (a, b) in &g.pairs {
        let (ma, mb) = (g.module_of[a.as_str()], g.module_of[b.as_str()]);
        if ma != mb {
            *imports.entry((ma, mb)).or_default() += 1;
        }
    }
    for ((ma, mb), n) in imports {
        let (Some(from), Some(to)) = (instability[ma], instability[mb]) else {
            continue;
        };
        if to - from >= SDP_MARGIN - EPSILON {
            out.push(finding(
                FindingKind::UnstableDependency,
                format!("{ma} -> {mb}"),
                Evidence::UnstableDependency {
                    from: ma.to_owned(),
                    to: mb.to_owned(),
                    from_instability: from,
                    to_instability: to,
                    imports: n,
                },
            ));
        }
    }
}

/// Label propagation over the undirected file graph without hub files, compared with the folders
/// by modularity; reported only when the clusters hold enough of the files.
fn seam(g: &Graph, out: &mut Vec<Finding>) {
    let mut neighbours: BTreeMap<&str, BTreeSet<&str>> = g
        .doc
        .files
        .iter()
        .map(|f| (f.path.as_str(), BTreeSet::new()))
        .collect();
    for (a, b) in &g.pairs {
        neighbours.get_mut(a.as_str()).unwrap().insert(b);
        neighbours.get_mut(b.as_str()).unwrap().insert(a);
    }
    let mut degrees: Vec<usize> = neighbours.values().map(BTreeSet::len).collect();
    degrees.sort();
    // Nearest-rank percentile.
    let rank = (HUB_PERCENTILE * degrees.len() as f64).ceil() as usize;
    let Some(&limit) = degrees.get(rank.saturating_sub(1)) else {
        return;
    };
    let (hubs, nodes): (Vec<&str>, Vec<&str>) = neighbours
        .keys()
        .partition(|f| neighbours[*f].len() > limit);
    let index: BTreeMap<&str, usize> = nodes.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let adjacency: Vec<Vec<usize>> = nodes
        .iter()
        .map(|f| {
            neighbours[f]
                .iter()
                .filter_map(|n| index.get(n).copied())
                .collect()
        })
        .collect();
    let m = adjacency.iter().map(Vec::len).sum::<usize>() / 2;
    if m == 0 {
        return;
    }

    let mut labels: Vec<usize> = (0..nodes.len()).collect();
    for _ in 0..LP_MAX_ITERATIONS {
        let mut changed = false;
        for v in 0..nodes.len() {
            let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
            for &w in &adjacency[v] {
                *counts.entry(labels[w]).or_default() += 1;
            }
            let Some(&max) = counts.values().max() else {
                continue;
            };
            // Keep the current label on a tie, else take the smallest: deterministic.
            if counts.get(&labels[v]) != Some(&max) {
                labels[v] = *counts.iter().find(|(_, c)| **c == max).unwrap().0;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Q = sum over communities of (internal edges / m) - (degree sum / 2m)^2; each internal
    // edge is seen from both ends, hence / 2m.
    let modularity = |community: &[usize]| {
        let mut inside: BTreeMap<usize, usize> = BTreeMap::new();
        let mut degree: BTreeMap<usize, usize> = BTreeMap::new();
        for (v, ws) in adjacency.iter().enumerate() {
            *degree.entry(community[v]).or_default() += ws.len();
            let same = ws.iter().filter(|&&w| community[w] == community[v]).count();
            *inside.entry(community[v]).or_default() += same;
        }
        let two_m = 2.0 * m as f64;
        degree
            .iter()
            .map(|(c, &d)| inside[c] as f64 / two_m - (d as f64 / two_m).powi(2))
            .sum::<f64>()
    };
    let folder_ids: BTreeMap<&str, usize> = g
        .doc
        .modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.as_str(), i))
        .collect();
    let by_folder: Vec<usize> = nodes.iter().map(|f| folder_ids[g.module_of[f]]).collect();
    let (clustered, folders) = (modularity(&labels), modularity(&by_folder));

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (v, &label) in labels.iter().enumerate() {
        groups.entry(label).or_default().push(v);
    }
    let mut clusters: Vec<Cluster> = groups
        .into_values()
        .filter(|members| members.len() >= 2)
        .map(|members| Cluster {
            files: members.iter().map(|&v| nodes[v].to_owned()).collect(),
            modules: members
                .iter()
                .map(|&v| g.module_of[nodes[v]].to_owned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        })
        .collect();
    let non_barrel = g.doc.files.iter().filter(|f| !f.barrel).count();
    let covered = clusters.iter().map(|c| c.files.len()).sum::<usize>();
    // A single cluster proposes no boundary.
    if clusters.len() < 2
        || clustered - folders < SEAM_MIN_GAIN - EPSILON
        || (covered as f64) < SEAM_MIN_COVERAGE * non_barrel as f64 - EPSILON
    {
        return;
    }
    clusters.sort_by(|a, b| a.files.cmp(&b.files));
    let kept: HashSet<&str> = nodes.iter().copied().collect();
    let used = g
        .pairs
        .iter()
        .filter(|(a, b)| kept.contains(a.as_str()) && kept.contains(b.as_str()));
    out.push(finding(
        FindingKind::Seam,
        ".".into(),
        Evidence::Seam {
            modularity: round(clustered),
            folder_modularity: round(folders),
            clusters,
            hubs: hubs.iter().map(|h| h.to_string()).collect(),
            via_barrel: g.via_barrel(used),
        },
    ));
}

fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}
