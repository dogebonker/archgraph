//! Module metrics, strongly connected components and cycle break sets.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Cycle, CycleLevel, Edge, EdgeKind, Link, Module, ModuleEdge, RegistrarMove, TwoWay, Verdict,
};

/// Above this many edges in one SCC the break set comes from the greedy heuristic: the exact
/// search tries every subset, 2^16 = 65536 of them at the limit.
const EXACT_LIMIT: usize = 16;
/// A two-way pair lists at most this many of its lighter direction's file imports; `imports`
/// counts them all.
pub const TWO_WAY_EDGES: usize = 20;
/// A file nobody imports that imports this many other modules is a composition root.
pub const ROOT_MIN_MODULES: usize = 3;
/// A file importing this many files of one module registers them (a driver registry, a router).
const REGISTRAR_MIN_IMPORTS: usize = 5;
/// A file importing from this many sibling modules (one parent folder) picks among them: a
/// registrar too, though it imports each only once.
const FANOUT_MIN_SIBLINGS: usize = 5;
/// A two-way pair whose directions both have at most this many imports, within this ratio of
/// each other, is a mutual dependency with no lighter side to cut.
const EVEN_MAX_IMPORTS: usize = 2;
/// One registrar forming the other side of this many two-way pairs is worth moving out.
const REGISTRAR_MOVE_MIN_PAIRS: usize = 3;
const EVEN_MAX_RATIO: usize = 2;

/// Ca, Ce, instability and cohesion from barrel-attributed file pairs.
pub fn modules(
    module_of: &BTreeMap<String, String>,
    pairs: &BTreeSet<(String, String)>,
) -> Vec<Module> {
    #[derive(Default)]
    struct Acc<'a> {
        files: usize,
        ca: BTreeSet<&'a str>,
        ce: BTreeSet<&'a str>,
        internal: usize,
        outgoing: usize,
        outgoing_to: BTreeMap<String, usize>,
    }
    let mut acc: BTreeMap<&str, Acc> = BTreeMap::new();
    for module in module_of.values() {
        acc.entry(module).or_default().files += 1;
    }
    for (from, to) in pairs {
        let (mf, mt) = (module_of[from].as_str(), module_of[to].as_str());
        if mf == mt {
            acc.get_mut(mf).unwrap().internal += 1;
        } else {
            let a = acc.get_mut(mf).unwrap();
            a.outgoing += 1;
            *a.outgoing_to.entry(mt.to_owned()).or_default() += 1;
            a.ce.insert(mt);
            acc.get_mut(mt).unwrap().ca.insert(mf);
        }
    }
    acc.into_iter()
        .map(|(id, a)| Module {
            id: id.to_owned(),
            files: a.files,
            ca: a.ca.len(),
            ce: a.ce.len(),
            instability: ratio(a.ce.len(), a.ca.len() + a.ce.len()),
            internal_edges: a.internal,
            outgoing_edges: a.outgoing,
            cohesion: ratio(a.internal, a.internal + a.outgoing),
            outgoing_to: a.outgoing_to,
        })
        .collect()
}

fn ratio(part: usize, whole: usize) -> Option<f64> {
    (whole > 0).then(|| (part as f64 / whole as f64 * 10_000.0).round() / 10_000.0)
}

/// Cross-module import and forbidden-import counts from the raw file edges.
pub fn module_edges(module_of: &BTreeMap<String, String>, edges: &[Edge]) -> Vec<ModuleEdge> {
    let mut counts: BTreeMap<(&str, &str), (usize, usize)> = BTreeMap::new();
    for e in edges {
        let (mf, mt) = (module_of[&e.from].as_str(), module_of[&e.to].as_str());
        if mf != mt {
            let count = counts.entry((mf, mt)).or_default();
            count.0 += 1;
            count.1 += usize::from(e.verdict == Some(Verdict::Forbidden));
        }
    }
    counts
        .into_iter()
        .map(|((from, to), (imports, forbidden))| ModuleEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            imports,
            forbidden,
        })
        .collect()
}

/// Module-level cycles, plus file-level cycles over the imports within each module (a file cycle
/// across modules already shows as a module cycle), over runtime imports only: break sets and
/// two-way pairs count only those too. A re-export by one of `entries` (package entries, the
/// source root's index, a package `__init__.py` importing its own submodules) is public API: no
/// break removes it unless nothing else breaks the cycle, and a two-way side holding one comes
/// last. A re-export by another of the `barrels` is internal: the side to cut first.
pub fn cycles(
    module_of: &BTreeMap<String, String>,
    edges: &[Edge],
    entries: &BTreeSet<String>,
    barrels: &BTreeSet<String>,
) -> Vec<Cycle> {
    let wiring = wiring(|p| module_of[p].as_str(), edges);
    // A file nobody imports is an entry point build config names, and an index is its module's
    // entry: neither is a move to propose.
    let imported: BTreeSet<&str> = edges.iter().map(|e| e.to.as_str()).collect();
    let edges: Vec<Edge> = edges
        .iter()
        .filter(|e| e.kind.is_runtime())
        .cloned()
        .collect();
    let edges = edges.as_slice();
    let public = |e: &Edge| {
        entries.contains(&e.from)
            && (e.kind == EdgeKind::Reexport || {
                // Python has no re-export kind: a package `__init__.py` importing its own files.
                let dir = e.from.strip_suffix("__init__.py");
                dir.is_some_and(|d| e.to.starts_with(d))
            })
    };
    // Heavier than every cuttable import together.
    let keep = edges.len() + 1;
    let module_edges = module_edges(module_of, edges);
    let mut by_side: BTreeMap<(&str, &str), Vec<&Edge>> = BTreeMap::new();
    for e in edges {
        let key = (module_of[&e.from].as_str(), module_of[&e.to].as_str());
        by_side.entry(key).or_default().push(e);
    }
    let sides: BTreeMap<(&str, &str), Side> = by_side
        .iter()
        .map(|(&key, side)| {
            let kind = if side.iter().any(|e| public(e)) {
                Side::Public
            } else if side
                .iter()
                .all(|e| wiring.contains(&(e.from.as_str(), e.to.as_str())))
            {
                Side::Registry
            } else if side
                .iter()
                .all(|e| e.kind == EdgeKind::Reexport && barrels.contains(&e.from))
            {
                Side::Reexport
            } else {
                Side::Cut
            };
            (key, kind)
        })
        .collect();
    let mut cycles = Vec::new();

    let imports: BTreeMap<(&str, &str), usize> = module_edges
        .iter()
        .map(|m| ((m.from.as_str(), m.to.as_str()), m.imports))
        .collect();
    let weighted: Vec<_> = module_edges
        .iter()
        .map(|m| {
            let (a, b) = (m.from.as_str(), m.to.as_str());
            let extra = if sides[&(a, b)] == Side::Public {
                keep
            } else {
                0
            };
            (a, b, m.imports + extra)
        })
        .collect();
    for (members, removed, exact) in cyclic_parts(&weighted) {
        let removed: BTreeSet<(&str, &str)> = removed
            .iter()
            .map(|&i| (weighted[i].0, weighted[i].1))
            .collect();
        let break_set = edges
            .iter()
            .filter(|e| removed.contains(&(module_of[&e.from].as_str(), module_of[&e.to].as_str())))
            .map(|e| Link {
                from: e.from.clone(),
                to: e.to.clone(),
            })
            .collect();
        let pairs = two_way(module_of, edges, &imports, &sides, &members);
        // One registrar forming the other side of several pairs: moving it out of its module
        // opens them all.
        let mut by_registrar: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for p in &pairs {
            let back = (p.to.as_str(), p.from.as_str());
            let files: BTreeSet<&str> = by_side[&back].iter().map(|e| e.from.as_str()).collect();
            if let ([file], Side::Registry) = (
                files.into_iter().collect::<Vec<_>>().as_slice(),
                sides[&back],
            ) && imported.contains(file)
                && !is_index(file)
            {
                by_registrar.entry(file).or_default().push(p.from.clone());
            }
        }
        let registrar_moves = by_registrar
            .into_iter()
            .filter(|(_, with)| with.len() >= REGISTRAR_MOVE_MIN_PAIRS)
            .map(|(file, mut with)| {
                with.sort();
                RegistrarMove {
                    file: file.to_owned(),
                    module: module_of[file].clone(),
                    pairs: with.len(),
                    with,
                    to: None,
                    pairs_removed: None,
                    cycle_before: None,
                    cycle_after: None,
                }
            })
            .collect();
        cycles.push(Cycle {
            level: CycleLevel::Module,
            two_way: Some(pairs),
            registrar_moves,
            members: owned(&members),
            exact,
            break_set,
        });
    }

    let weighted: Vec<_> = edges
        .iter()
        .filter(|e| e.from != e.to && module_of[&e.from] == module_of[&e.to])
        .map(|e| {
            (
                e.from.as_str(),
                e.to.as_str(),
                if public(e) { keep } else { 1 },
            )
        })
        .collect();
    for (members, removed, exact) in cyclic_parts(&weighted) {
        let break_set = removed
            .iter()
            .map(|&i| Link {
                from: weighted[i].0.to_owned(),
                to: weighted[i].1.to_owned(),
            })
            .collect();
        cycles.push(Cycle {
            level: CycleLevel::File,
            members: owned(&members),
            exact,
            break_set,
            two_way: None,
            registrar_moves: Vec::new(),
        });
    }

    cycles.sort_by(|a, b| a.members.cmp(&b.members));
    cycles
}

/// A module's index (`index.ts`, `__init__.py`): its entry, which moves only with the module.
fn is_index(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    name == "__init__.py" || crate::modules::TS_INDEX.contains(&name)
}

/// What cutting one direction of a module pair would cut.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    /// Only an internal barrel's re-exports of the other module's files: the first to drop.
    Reexport,
    /// Ordinary imports.
    Cut,
    /// Only imports from composition roots and registrars (`wiring`).
    Registry,
    /// A re-export by a public entry.
    Public,
}

/// File imports that wire modules together rather than use them: every import of a composition
/// root (a file nobody imports that imports `ROOT_MIN_MODULES` or more other modules: `app.ts`,
/// `cli.ts`), a registrar's imports into a module it imports `REGISTRAR_MIN_IMPORTS` or more
/// files of, and a fan-out registrar's imports into `FANOUT_MIN_SIBLINGS` or more sibling modules
/// (code picking one of many drivers).
pub fn wiring<'a, 'm>(
    module_of: impl Fn(&str) -> &'m str,
    edges: &'a [Edge],
) -> BTreeSet<(&'a str, &'a str)> {
    let imported: BTreeSet<&str> = edges.iter().map(|e| e.to.as_str()).collect();
    let mut into: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for e in edges.iter().filter(|e| e.kind.is_runtime()) {
        let (mf, mt) = (module_of(&e.from), module_of(&e.to));
        if mf != mt {
            *into.entry(&e.from).or_default().entry(mt).or_default() += 1;
        }
    }
    let parent = |m: &str| m.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
    // Per file: the parents under which it imports `FANOUT_MIN_SIBLINGS` or more modules.
    let fans: BTreeMap<&str, BTreeSet<String>> = into
        .iter()
        .map(|(&file, counts)| {
            let mut siblings: BTreeMap<String, usize> = BTreeMap::new();
            // Top-level modules and root files share no parent folder.
            for m in counts
                .keys()
                .filter(|m| !parent(m).is_empty() && parent(m) != "..")
            {
                *siblings.entry(parent(m)).or_default() += 1;
            }
            let wide = siblings
                .into_iter()
                .filter(|(_, n)| *n >= FANOUT_MIN_SIBLINGS)
                .map(|(p, _)| p)
                .collect();
            (file, wide)
        })
        .collect();
    edges
        .iter()
        .filter(|e| {
            let Some(counts) = into.get(e.from.as_str()) else {
                return false;
            };
            let to = module_of(&e.to);
            let root = !imported.contains(e.from.as_str()) && counts.len() >= ROOT_MIN_MODULES;
            let registrar = counts.get(to).is_some_and(|&n| n >= REGISTRAR_MIN_IMPORTS);
            let fan_out = counts.contains_key(to) && fans[e.from.as_str()].contains(&parent(to));
            root || registrar || fan_out
        })
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect()
}

/// Pairs of `members` with module edges both ways, as the direction to cut: the lighter one,
/// unless it is only wiring (`registry`) or holds a public re-export (`entry`) and the other is
/// not; a near-even pair (`even`) names none and keeps the names' order. Sorted ordinary pairs
/// first, then even, registry and entry pairs, each by imports, then names.
fn two_way(
    module_of: &BTreeMap<String, String>,
    edges: &[Edge],
    imports: &BTreeMap<(&str, &str), usize>,
    sides: &BTreeMap<(&str, &str), Side>,
    members: &[&str],
) -> Vec<TwoWay> {
    let mut pairs = Vec::new();
    for (i, &a) in members.iter().enumerate() {
        for &b in &members[i + 1..] {
            let (Some(&ab), Some(&ba)) = (imports.get(&(a, b)), imports.get(&(b, a))) else {
                continue;
            };
            let (sa, sb) = (sides[&(a, b)], sides[&(b, a)]);
            let (lo, hi) = (ab.min(ba), ab.max(ba));
            let even = sa == Side::Cut
                && sb == Side::Cut
                && hi <= EVEN_MAX_IMPORTS
                && hi <= EVEN_MAX_RATIO * lo;
            // Ordinary and re-export sides rank alike, the lighter first, a re-export on a tie.
            let key = |s: Side, n: usize| match s {
                Side::Reexport => (0, n, 0),
                Side::Cut => (0, n, 1),
                Side::Registry => (1, n, 0),
                Side::Public => (2, n, 0),
            };
            // An even pair names no lighter side: the names' order.
            let (from, to, n, reverse, side) = if even || key(sa, ab) <= key(sb, ba) {
                (a, b, ab, ba, sa)
            } else {
                (b, a, ba, ab, sb)
            };
            pairs.push(TwoWay {
                from: from.to_owned(),
                to: to.to_owned(),
                imports: n,
                reverse_imports: reverse,
                even,
                registry: side == Side::Registry,
                entry: side == Side::Public,
                resolved_by_move: false,
                edges: Vec::new(),
            });
        }
    }
    // One pass over the file edges fills every pair, instead of one pass per pair.
    let slot: BTreeMap<(&str, &str), usize> = pairs
        .iter()
        .enumerate()
        .map(|(i, p): (usize, &TwoWay)| ((p.from.as_str(), p.to.as_str()), i))
        .collect();
    let mut links: Vec<Vec<Link>> = vec![Vec::new(); pairs.len()];
    for e in edges {
        let key = (module_of[&e.from].as_str(), module_of[&e.to].as_str());
        if let Some(&i) = slot.get(&key)
            && links[i].len() < TWO_WAY_EDGES
        {
            links[i].push(Link {
                from: e.from.clone(),
                to: e.to.clone(),
            });
        }
    }
    for (p, l) in pairs.iter_mut().zip(links) {
        p.edges = l;
    }
    pairs.sort_by(|x, y| {
        (x.entry, x.registry, x.even, x.imports, &x.from, &x.to)
            .cmp(&(y.entry, y.registry, y.even, y.imports, &y.from, &y.to))
    });
    pairs
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// Every SCC with two or more nodes, as (sorted members, indices into `edges` to remove, exact).
fn cyclic_parts<'a>(edges: &[(&'a str, &'a str, usize)]) -> Vec<(Vec<&'a str>, Vec<usize>, bool)> {
    let nodes: Vec<&str> = edges
        .iter()
        .flat_map(|e| [e.0, e.1])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let index: BTreeMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut adjacency = vec![Vec::new(); nodes.len()];
    for e in edges {
        adjacency[index[e.0]].push(index[e.1]);
    }
    let mut parts = Vec::new();
    for mut component in sccs(&adjacency).into_iter().filter(|c| c.len() >= 2) {
        component.sort();
        let members: Vec<&str> = component.iter().map(|&g| nodes[g]).collect();
        let local: BTreeMap<usize, usize> =
            component.iter().enumerate().map(|(l, &g)| (g, l)).collect();
        let inside: Vec<usize> = (0..edges.len())
            .filter(|&i| {
                local.contains_key(&index[edges[i].0]) && local.contains_key(&index[edges[i].1])
            })
            .collect();
        let local_edges: Vec<_> = inside
            .iter()
            .map(|&i| {
                (
                    local[&index[edges[i].0]],
                    local[&index[edges[i].1]],
                    edges[i].2,
                )
            })
            .collect();
        let (removed, exact) = break_set(component.len(), &local_edges);
        parts.push((
            members,
            removed.into_iter().map(|k| inside[k]).collect(),
            exact,
        ));
    }
    parts
}

/// Tarjan's algorithm with an explicit call stack, so import chains of any depth fit.
fn sccs(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNSEEN: usize = usize::MAX;
    let n = adjacency.len();
    let (mut index, mut low) = (vec![UNSEEN; n], vec![0; n]);
    let mut on_stack = vec![false; n];
    let (mut stack, mut out) = (Vec::new(), Vec::new());
    // (node, next neighbour to look at)
    let mut calls: Vec<(usize, usize)> = Vec::new();
    let mut next = 0;
    for root in 0..n {
        if index[root] != UNSEEN {
            continue;
        }
        calls.push((root, 0));
        while let Some(&(v, i)) = calls.last() {
            if i == 0 {
                index[v] = next;
                low[v] = next;
                next += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&w) = adjacency[v].get(i) {
                calls.last_mut().unwrap().1 += 1;
                if index[w] == UNSEEN {
                    calls.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(component);
            }
        }
    }
    out
}

/// Minimum-weight set of edges whose removal leaves the graph acyclic, as indices into `edges`,
/// and whether it is exact. Ties go to the first subset in bitmask order, so output is stable.
fn break_set(n: usize, edges: &[(usize, usize, usize)]) -> (Vec<usize>, bool) {
    if edges.len() > EXACT_LIMIT {
        return (eades_lin_smyth(n, edges), false);
    }
    let mut best: Option<(usize, u32)> = None;
    for mask in 0..(1u32 << edges.len()) {
        let weight: usize = (0..edges.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| edges[i].2)
            .sum();
        if best.is_none_or(|(w, _)| weight < w) && acyclic(n, edges, mask) {
            best = Some((weight, mask));
        }
    }
    let mask = best.map_or(0, |(_, m)| m);
    (
        (0..edges.len()).filter(|i| mask & (1 << i) != 0).collect(),
        true,
    )
}

fn acyclic(n: usize, edges: &[(usize, usize, usize)], removed: u32) -> bool {
    let mut indegree = vec![0; n];
    let mut adjacency = vec![Vec::new(); n];
    for (i, &(a, b, _)) in edges.iter().enumerate() {
        if removed & (1 << i) == 0 {
            indegree[b] += 1;
            adjacency[a].push(b);
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&v| indegree[v] == 0).collect();
    let mut seen = 0;
    while let Some(v) = ready.pop() {
        seen += 1;
        for &w in &adjacency[v] {
            indegree[w] -= 1;
            if indegree[w] == 0 {
                ready.push(w);
            }
        }
    }
    seen == n
}

/// Eades, Lin and Smyth (1993), weighted: peel sinks to the back and sources to the front,
/// otherwise move the node with the largest out-weight minus in-weight to the front; edges that
/// point backwards in the resulting order form the break set.
// ponytail: O(n^2 + m) node selection; a bucket queue makes it linear if SCCs get huge.
fn eades_lin_smyth(n: usize, edges: &[(usize, usize, usize)]) -> Vec<usize> {
    let (mut out_w, mut in_w) = (vec![0i64; n], vec![0i64; n]);
    let mut touching = vec![Vec::new(); n];
    for (i, &(a, b, w)) in edges.iter().enumerate() {
        out_w[a] += w as i64;
        in_w[b] += w as i64;
        touching[a].push(i);
        touching[b].push(i);
    }
    let mut alive = vec![true; n];
    let (mut front, mut back) = (Vec::new(), Vec::new());
    loop {
        let live = || (0..n).filter(|&v| alive[v]);
        let next = live()
            .find(|&v| out_w[v] == 0)
            .map(|v| (v, false))
            .or_else(|| live().find(|&v| in_w[v] == 0).map(|v| (v, true)))
            .or_else(|| {
                live()
                    .max_by_key(|&v| (out_w[v] - in_w[v], Reverse(v)))
                    .map(|v| (v, true))
            });
        let Some((v, to_front)) = next else { break };
        if to_front {
            front.push(v)
        } else {
            back.push(v)
        }
        alive[v] = false;
        for &i in &touching[v] {
            let (a, b, w) = edges[i];
            if a == v && alive[b] {
                in_w[b] -= w as i64;
            } else if b == v && alive[a] {
                out_w[a] -= w as i64;
            }
        }
    }
    back.reverse();
    front.extend(back);
    let mut position = vec![0; n];
    for (p, &v) in front.iter().enumerate() {
        position[v] = p;
    }
    (0..edges.len())
        .filter(|&i| position[edges[i].0] > position[edges[i].1])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sccs;

    #[test]
    fn sccs_survive_a_hundred_thousand_long_chain() {
        // 0 -> 1 -> ... -> 99_999 -> 0: one SCC holding every node, found depth-first.
        let n = 100_000;
        let adjacency: Vec<Vec<usize>> = (0..n).map(|v| vec![(v + 1) % n]).collect();
        let components = sccs(&adjacency);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].len(), n);

        // Without the back edge every node is its own SCC.
        let chain: Vec<Vec<usize>> = (0..n)
            .map(|v| if v + 1 < n { vec![v + 1] } else { vec![] })
            .collect();
        assert_eq!(sccs(&chain).len(), n);
    }
}
