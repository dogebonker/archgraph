//! Recommendation analyses over a built document. Thresholds are tuning knobs, not truths.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Bound;

use crate::model::{
    Cluster, Component, Document, Edge, EdgeKind, Evidence, Finding, FindingKind, Language, Link,
    Move, PlacementOwnership, RegistrarMove, Suggestion, Verdict,
};
use crate::rules::Compiled;
use crate::simulate;
use crate::{graph, modules};

/// A module imported by at least this many others is a shared sink.
const SINK_CA: usize = 3;
/// A sink splits only when at least two of its groups hold this many files: a lone file used by
/// one consumer is a misplaced file, not a second module.
const SINK_MIN_GROUP_FILES: usize = 2;
/// A file looks misplaced only when at least this many files of one other module import it. A
/// file's own imports never pull it: a router importing `app.deps` uses `app`, it does not belong
/// there. A file nobody imports is an entry point a framework locates by path (`page.tsx`).
const MISPLACED_MIN_IMPORTS: usize = 2;
/// The target's imports of a file must exceed its edges within its own module this many times:
/// two imports against one own edge is a tie of habits, not a misplaced file.
const MISPLACED_OWN_FACTOR: usize = 2;
/// A file imported from more other modules than this is a shared provider, not misplaced.
const MISPLACED_MAX_IMPORTER_MODULES: usize = 1;
/// Folder names that say what kind of code a file is, singular and space-separated
/// (`kind_folder` also takes the plural). Moving a file between two such folders changes its
/// kind, not its boundary.
// ponytail: a fixed list; folders named by kind in other words (`dao`, `infra`) still count as
// domains. Make it configurable when a repository needs that.
const KIND_FOLDERS: &str = "component scene page screen view controller service repository \
    model entity dto schema serializer type interface constant util helper lib hook store reducer \
    action middleware guard pipe filter interceptor decorator handler route api ui migration \
    management command adapter port provider module plugin config test menu layout container \
    widget icon style exception error event";
/// Folders of shared helpers: a file there that imports nothing is used from anywhere by design.
const LEAF_UTILITY_FOLDERS: &str = "util utils helper helpers lib shared common storage";
/// A suggested move must raise the cohesion of each module receiving files by this much: smaller
/// gains are noise a reviewer would not act on.
const MIN_COHESION_GAIN: f64 = 0.02;
/// Stable dependencies principle: flag A -> B when B is this much less stable than A.
const SDP_MARGIN: f64 = 0.25;
/// Below this many runtime file imports from A to B a stable-dependencies violation is noise.
const SDP_MIN_IMPORTS: usize = 2;
/// Files with a degree above this percentile are hubs that would glue every cluster together.
const HUB_PERCENTILE: f64 = 0.9;
/// Label propagation usually settles in a handful of rounds; this bounds oscillation.
const LP_MAX_ITERATIONS: usize = 50;
/// The clustering must beat the folders' modularity by this much to be worth reporting.
const SEAM_MIN_GAIN: f64 = 0.05;
/// No seam when more than this share of the folder modules are named by kind (controllers,
/// services, repositories): the codebase is laid out by kind, and clusters across kinds restate it.
const SEAM_MAX_KIND_SHARE: f64 = 0.5;
/// No seam when more than this share of the non-barrel files sit in single-file modules: a flat
/// package's folders give the clusters nothing to compare with.
const SEAM_MAX_LOOSE_SHARE: f64 = 0.5;
/// Clusters must hold at least this share of the non-barrel files: a few clustered files among
/// many loose ones propose no boundary.
const SEAM_MIN_COVERAGE: f64 = 0.5;
/// Below either size, findings carry little signal.
const SMALL_FILES: usize = 20;
const SMALL_MODULES: usize = 3;
/// Without rules, a module's barrels are its public entry only when at least this share of the
/// cross-module imports into it go through one of them: a barrel most importers bypass is not
/// the API the codebase uses.
const BARREL_MIN_SHARE: f64 = 0.5;
/// An internals-reach finding lists at most this many of its imports; `imports` counts them all.
const REACH_EDGES: usize = 20;
/// Absorbs float error in threshold comparisons of rounded ratios.
const EPSILON: f64 = 1e-9;

struct Graph<'a> {
    doc: &'a Document,
    rules: Option<&'a Compiled>,
    owners: Option<&'a crate::codeowners::Compiled>,
    module_of: BTreeMap<&'a str, &'a str>,
    /// Barrel-attributed file pairs.
    pairs: &'a BTreeSet<(String, String)>,
    /// `pairs` by importing file and by imported file.
    outgoing: BTreeMap<&'a str, Vec<&'a (String, String)>>,
    incoming: BTreeMap<&'a str, Vec<&'a (String, String)>>,
    raw: HashSet<(&'a str, &'a str)>,
    /// Longest common directory of each module's files: where a file moved into it goes.
    dirs: BTreeMap<&'a str, String>,
    /// Modules named by their one file (`main.ts`, `../tool.ts`): files directly in a root.
    root_files: HashSet<&'a str>,
    /// Each importing file's imports.
    imports: BTreeMap<&'a str, Vec<&'a Edge>>,
    /// Each module's public entries: its barrels and the files a package.json publishes in it.
    entries: BTreeMap<&'a str, BTreeSet<&'a str>>,
    /// Each module's files its entries re-export, directly or through other re-exports.
    offered: BTreeMap<&'a str, HashSet<&'a str>>,
    /// Files that are public API, never proposed for a move: what a package.json publishes and
    /// re-exports; in Python, every module without a leading underscore below the package.
    public: HashSet<&'a str>,
    /// Go files bound to their package (`source.collector.bound_paths()`): never moved.
    bound: HashSet<&'a str>,
    /// Per file: the number of other modules it imports at runtime.
    fanout: BTreeMap<&'a str, usize>,
    /// Imports from composition roots and registrars (`graph::wiring`).
    wiring: BTreeSet<(&'a str, &'a str)>,
    /// `pairs` over runtime imports that are not wiring: who really consumes a file.
    consuming: &'a BTreeSet<(String, String)>,
}

impl Graph<'_> {
    fn via_barrel<'p>(&self, mut used: impl Iterator<Item = &'p (String, String)>) -> bool {
        used.any(|(a, b)| !self.raw.contains(&(a.as_str(), b.as_str())))
    }

    /// The move of `file` into `module`, unless a file of that name is there or the destination
    /// falls in another module (a file directly in the source root is a module of its own).
    fn move_into(&self, file: &str, module: &str) -> Option<Move> {
        let to = modules::join(
            self.dest_dir(file, module),
            file.rsplit('/').next().unwrap_or(file),
        );
        let config = &self.doc.config;
        let lands = modules::assigned(&to, config, self.doc.source.language, self.rules);
        (!self.module_of.contains_key(to.as_str())
            && lands == module
            && !self.crosses_module(file, &to))
        .then(|| Move {
            file: file.to_owned(),
            to,
        })
    }

    /// `move_into`, or when the name is taken, the same move under `moved-<name>` (repeated
    /// until free): what to simulate when there is no move to suggest.
    fn free_move(&self, file: &str, module: &str) -> Move {
        let dir = self.dest_dir(file, module);
        let mut name = file.rsplit('/').next().unwrap_or(file).to_owned();
        while self
            .module_of
            .contains_key(modules::join(dir, &name).as_str())
        {
            name = format!("moved-{name}");
        }
        Move {
            file: file.to_owned(),
            to: modules::join(dir, &name),
        }
    }

    /// `module`'s directory plus the folder `file` sits in within its own module
    /// (`chains/domain/hd.py` into signer is `signer/domain`), or `module`'s directory alone
    /// when that folder does not exist there.
    fn dest_dir(&self, file: &str, module: &str) -> &str {
        let own = self.dirs[self.module_of[file]].as_str();
        let dir = modules::dir_of(file);
        let sub = match own {
            "" => dir,
            _ => dir
                .strip_prefix(own)
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(""),
        };
        let root = self.dirs[module].as_str();
        if sub.is_empty() {
            return root;
        }
        let candidate = modules::join(root, sub);
        // The folder must hold a file of `module`: under nested modules it may be another one's.
        let folder = format!("{candidate}/");
        self.module_of
            .range::<str, _>((Bound::Included(folder.as_str()), Bound::Unbounded))
            .take_while(|(p, _)| p.starts_with(&folder))
            .find(|(_, m)| **m == module)
            .map_or(root, |(p, _)| &p[..candidate.len()])
    }

    /// Whether module `a` is a folder above module `b` (`db` above `db/backends`). The root ids
    /// `.` and `..` and a file's own module (`../tool.ts`) are no folder of another module.
    fn is_ancestor(&self, a: &str, b: &str) -> bool {
        let folder = |m: &str| m != "." && m != ".." && !self.root_files.contains(m);
        folder(a) && folder(b) && b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
    }

    /// Whether `file` is one of `module`'s entries or an index (`index.*`, `__init__.py`).
    fn is_entry_or_index(&self, module: &str, file: &str) -> bool {
        let name = file.rsplit('/').next().unwrap_or(file);
        self.entries.get(module).is_some_and(|e| e.contains(file)) || modules::is_index(name)
    }

    /// Whether moving `file` to `to` would take it into another Go module (go.mod directory).
    fn crosses_module(&self, file: &str, to: &str) -> bool {
        go_module(file, &self.doc.source.collector.go_modules)
            != go_module(to, &self.doc.source.collector.go_modules)
    }
}

pub fn find(
    doc: &Document,
    rules: Option<&Compiled>,
    owners: Option<&crate::codeowners::Compiled>,
) -> Vec<Finding> {
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
        let dir = modules::dir_of(&f.path);
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
    // Re-exports of each file, for what an entry offers.
    let mut reexports: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in doc
        .edges
        .iter()
        .filter(|e| e.is_reexport || matches!(e.kind, EdgeKind::Reexport | EdgeKind::TypeReexport))
    {
        reexports.entry(&e.from).or_default().push(&e.to);
    }
    let reach = |starts: &mut dyn Iterator<Item = &'_ str>| -> HashSet<&str> {
        let mut seen = HashSet::new();
        let mut stack: Vec<&str> = starts.collect();
        while let Some(file) = stack.pop() {
            for &next in reexports.get(file).into_iter().flatten() {
                if seen.insert(next) {
                    stack.push(next);
                }
            }
        }
        seen
    };
    let mut entries: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for f in doc.files.iter().filter(|f| f.barrel) {
        entries.entry(&f.module).or_default().insert(&f.path);
    }
    for p in &doc.package_entries {
        if let Some(&m) = module_of.get(p.as_str()) {
            entries.entry(m).or_default().insert(p);
        }
    }
    let offered = entries
        .iter()
        .map(|(&m, files)| (m, reach(&mut files.iter().copied())))
        .collect();
    let public = match doc.source.language {
        Language::Typescript => {
            let published = doc.package_entries.iter().map(String::as_str);
            let mut public = reach(&mut published.clone());
            public.extend(published);
            public
        }
        Language::Python => doc
            .files
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| python_public(p, &doc.config.source_root))
            .collect(),
        Language::Go => doc
            .files
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| go_public(p, &doc.source.collector.main_packages))
            .collect(),
        Language::Rust | Language::Java | Language::Elixir => Default::default(),
    };
    let wiring = graph::wiring(|p| module_of[p], &doc.edges);
    let consumed = doc
        .edges
        .iter()
        .filter(|e| e.kind.is_runtime() && !wiring.contains(&(e.from.as_str(), e.to.as_str())));
    let consuming = modules::attribute(consumed, &barrels);
    let pairs = modules::attribute(&doc.edges, &barrels);
    let (mut outgoing, mut incoming): (BTreeMap<&str, Vec<_>>, BTreeMap<&str, Vec<_>>) =
        Default::default();
    for pair in &pairs {
        outgoing.entry(pair.0.as_str()).or_default().push(pair);
        incoming.entry(pair.1.as_str()).or_default().push(pair);
    }
    let mut imports: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    for e in &doc.edges {
        imports.entry(&e.from).or_default().push(e);
    }
    let g = Graph {
        owners,
        doc,
        rules,
        pairs: &pairs,
        outgoing,
        incoming,
        raw: doc
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect(),
        dirs: dirs.into_iter().map(|(m, d)| (m, d.join("/"))).collect(),
        root_files: doc
            .files
            .iter()
            .filter(|f| f.module.rsplit('/').next() == f.path.rsplit('/').next())
            .map(|f| f.module.as_str())
            .collect(),
        imports,
        entries,
        offered,
        public,
        consuming: &consuming,
        bound: doc.source.collector.bound_paths().into_iter().collect(),
        fanout: {
            let mut into: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            for e in doc.edges.iter().filter(|e| e.kind.is_runtime()) {
                let (mf, mt) = (module_of[e.from.as_str()], module_of[e.to.as_str()]);
                if mf != mt {
                    into.entry(&e.from).or_default().insert(mt);
                }
            }
            into.into_iter().map(|(f, m)| (f, m.len())).collect()
        },
        wiring,
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
    for evidence in &doc.grabbags {
        findings.push(finding(
            FindingKind::CycleViaGrabbag,
            evidence.members.join(", "),
            Evidence::Grabbag(evidence.clone()),
        ));
    }
    internals_reach(&g, &mut findings);
    let reexports: Vec<_> = doc
        .edges
        .iter()
        .filter(|e| {
            !(doc.source.language == Language::Rust
                && e.specifier.as_ref().is_some_and(|s| s.starts_with("mod ")))
        })
        .filter(|e| {
            e.is_reexport
                || matches!(e.kind, EdgeKind::Reexport | EdgeKind::TypeReexport)
                    && e.confidence != Some(crate::model::Confidence::Low)
        })
        .map(|e| (e.from.as_str(), e.original_to.as_deref().unwrap_or(&e.to)))
        .collect();
    for members in graph::components(&reexports) {
        let edges = reexports
            .iter()
            .filter(|(a, b)| members.contains(a) && members.contains(b))
            .map(|(from, to)| crate::model::Link {
                from: (*from).into(),
                to: (*to).into(),
            })
            .collect();
        let members: Vec<String> = members.into_iter().map(str::to_owned).collect();
        findings.push(finding(
            FindingKind::ReexportCycle,
            members.join(" -> "),
            Evidence::ReexportCycle { members, edges },
        ));
    }
    hidden_coupling(&g, &mut findings);
    unstable_dependency(&g, &mut findings);
    // Go: clusters of files split packages, which Go cannot do: no seam.
    if doc.source.language != Language::Go {
        seam(&g, &mut findings);
    }
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
        finding_id: String::new(),
        recipe: None,
        kind,
        subject,
        evidence,
        suggestion: None,
        simulation: None,
        age_days: None,
        introduced_by: None,
        provenance: None,
        hotspot_priority: None,
    }
}

/// Components of the bipartite graph between a sink's files and the modules importing them.
fn sink_split(g: &Graph, out: &mut Vec<Finding>) {
    // A folder of one kind of code (services, errors) serves whoever needs that kind: separate
    // groups of importers are its purpose, not a split.
    for module in g
        .doc
        .modules
        .iter()
        .filter(|m| m.ca >= SINK_CA && !kind_folder(&m.id))
    {
        let sink = module.id.as_str();
        // Runtime imports only, and no wiring: a type import or a registrar consumes nothing.
        // Nor does the folder above the sink, or a file that imports several modules and only
        // the sink's entry or index of it: both register the sink rather than use its files.
        let registers = |file: &str| {
            g.fanout.get(file).copied().unwrap_or(0) >= graph::ROOT_MIN_MODULES
                && g.imports
                    .get(file)
                    .into_iter()
                    .flatten()
                    .filter(|e| g.module_of[e.to.as_str()] == sink)
                    .all(|e| g.is_entry_or_index(sink, &e.to))
        };
        let into: Vec<&(String, String)> = g
            .consuming
            .iter()
            .filter(|(from, to)| {
                let from_module = g.module_of[from.as_str()];
                g.module_of[to.as_str()] == sink
                    && !g.is_ancestor(from_module, sink)
                    && (from_module == sink || !registers(from))
            })
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
                // Public API stays where its importers find it, and nothing moves between two
                // folders named by kind.
                [only] if !(kind_folder(sink) && kind_folder(only)) => files
                    .iter()
                    .filter(|f| !g.public.contains(f.as_str()) && !g.bound.contains(f.as_str()))
                    .filter_map(|f| g.move_into(f, only))
                    .filter(|m| !ownership_conflict(g.owners, &m.file, &m.to))
                    .collect(),
                _ => Vec::new(),
            };
            components.push(Component {
                files,
                consumers,
                moves,
            });
        }
        // Groups that import each other at runtime would split into a cycle.
        let group: BTreeMap<&str, usize> = components
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.files.iter().map(move |f| (f.as_str(), i)))
            .collect();
        let entangled = g.doc.edges.iter().any(|e| {
            e.kind.is_runtime()
                && matches!((group.get(e.from.as_str()), group.get(e.to.as_str())),
                            (Some(a), Some(b)) if a != b)
        });
        if entangled
            || components
                .iter()
                .filter(|c| c.files.len() >= SINK_MIN_GROUP_FILES)
                .count()
                < 2
        {
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
        let mut moves: Vec<Move> = components.iter().flat_map(|c| c.moves.clone()).collect();
        // The moves must pay for themselves in every module receiving files.
        let mut after = None;
        if !moves.is_empty() {
            let receivers: Vec<&str> = components
                .iter()
                .filter(|c| !c.moves.is_empty())
                .flat_map(|c| c.consumers.iter().map(String::as_str))
                .collect();
            let affected: Vec<&str> = receivers.iter().copied().chain([sink]).collect();
            after = simulate::apply(g.doc, g.rules, &moves)
                .ok()
                .filter(|a| gains(g.doc, a, &receivers, &affected));
            if after.is_none() {
                moves.clear();
                for c in &mut components {
                    c.moves.clear();
                }
            }
        }
        let mut f = finding(
            FindingKind::SinkSplit,
            sink.to_owned(),
            Evidence::SinkSplit {
                ca: module.ca,
                via_barrel: g.via_barrel(into.iter().copied().filter(|(from, _)| outside(from))),
                components,
            },
        );
        f.simulation = after.map(|a| simulate::changes(g.doc, &a));
        out.push(f);
    }
}

/// A file imported by at least two files of one other module, more often than it has edges (in
/// and out) within its own, and from no third module, whose move would add no forbidden import or
/// cycle, give the target module no new dependency and raise the cohesion of the two modules
/// taken together.
fn misplaced_file(g: &Graph, out: &mut Vec<Finding>) {
    let sizes: BTreeMap<&str, usize> = g
        .doc
        .modules
        .iter()
        .map(|m| (m.id.as_str(), m.files))
        .collect();
    for file in &g.doc.files {
        // A package's __init__.py cannot move without the package, nor can a Go file bound to its
        // package by methods, and moving a module's only file just renames the module.
        // In Go a public file (outside internal/, not a command) may be imported by other
        // modules: moving it to another package would break them.
        if file.barrel
            || file.path.ends_with("__init__.py")
            || g.bound.contains(file.path.as_str())
            || (g.doc.source.language == Language::Go && g.public.contains(file.path.as_str()))
            || sizes[file.module.as_str()] == 1
        {
            continue;
        }
        let (path, module) = (file.path.as_str(), file.module.as_str());
        let (outgoing, incoming) = (
            g.outgoing.get(path).map_or(&[][..], Vec::as_slice),
            g.incoming.get(path).map_or(&[][..], Vec::as_slice),
        );
        let mut imports_from: BTreeMap<&str, Vec<&(String, String)>> = BTreeMap::new();
        for &pair in incoming {
            let from = g.module_of[pair.0.as_str()];
            if from != module {
                imports_from.entry(from).or_default().push(pair);
            }
        }
        if imports_from.len() > MISPLACED_MAX_IMPORTER_MODULES {
            continue;
        }
        let own = outgoing
            .iter()
            .filter(|(_, b)| g.module_of[b.as_str()] == module)
            .count()
            + incoming
                .iter()
                .filter(|(a, _)| g.module_of[a.as_str()] == module)
                .count();
        let Some((&target, used)) = imports_from
            .iter()
            .filter(|(_, used)| {
                used.len() >= MISPLACED_MIN_IMPORTS && used.len() > MISPLACED_OWN_FACTOR * own
            })
            .max_by_key(|(m, used)| (used.len(), Reverse(**m)))
        else {
            continue;
        };
        // In Go a file working with its own package is a piece of the library the package offers,
        // whoever uses it now: it is misplaced only when it uses the target itself or nothing
        // of its own package.
        if g.doc.source.language == Language::Go {
            let dir = modules::dir_of(path);
            let uses_target = outgoing
                .iter()
                .any(|(_, b)| g.module_of[b.as_str()] == target);
            let in_package = outgoing.iter().any(|(_, b)| modules::dir_of(b) == dir)
                || incoming.iter().any(|(a, _)| modules::dir_of(a) == dir);
            if !uses_target && in_package {
                continue;
            }
        }
        // Between a module and its own subfolder the move only changes the folder's depth.
        // A leaf utility (it imports nothing) belongs with the shared helpers, whoever uses it.
        let leaf_utility = !g.imports.contains_key(path)
            && LEAF_UTILITY_FOLDERS.split_whitespace().any(|f| {
                module
                    .rsplit('/')
                    .next()
                    .unwrap_or(module)
                    .eq_ignore_ascii_case(f)
            });
        if leaf_utility
            || (kind_folder(module) && kind_folder(target))
            || g.is_ancestor(module, target)
            || g.is_ancestor(target, module)
        {
            continue;
        }
        let mut f = finding(
            FindingKind::MisplacedFile,
            file.path.clone(),
            Evidence::MisplacedFile {
                own_module_edges: own,
                target_module: target.to_owned(),
                imports_from_target: used.len(),
                via_barrel: g.via_barrel(used.iter().copied()),
                ownership: None,
            },
        );
        // ponytail: every surviving move reruns the whole pipeline; batch or make it incremental
        // if scans of large repositories with many misplaced files get slow.
        //
        // When the file's name is taken in the target directory there is no move to suggest, and
        // the check runs on a free name there instead.
        let suggested = g.move_into(path, target);
        let probe = suggested
            .clone()
            .unwrap_or_else(|| g.free_move(path, target));
        if g.crosses_module(path, &probe.to) {
            continue;
        }
        let incident = outgoing
            .iter()
            .copied()
            .chain(incoming.iter().copied().filter(|(from, _)| from != path));
        if rejects_move_metrics(
            g.doc,
            g.rules,
            &g.module_of,
            file,
            &probe.to,
            target,
            incident,
        ) {
            continue;
        }
        let Ok(after) = simulate::apply(g.doc, g.rules, std::slice::from_ref(&probe)) else {
            continue;
        };
        let (was, is) = (&g.doc.summary, &after.summary);
        let modules = [module, target];
        // A file that only uses the target (a component calling into a library) would drag its
        // other dependencies into it.
        let ce = |s: &crate::model::Summary| s.modules.get(target).map_or(0, |m| m.ce);
        if is.forbidden > was.forbidden
            || is.cycles > was.cycles
            || ce(is) > ce(was)
            || !gains(g.doc, &after, &[target], &modules)
        {
            continue;
        }
        let ownership = g.owners.map(|owners| {
            let primary = |id: &str| {
                g.doc
                    .modules
                    .iter()
                    .find(|m| m.id == id)
                    .and_then(|m| m.ownership.as_ref())
                    .map(|o| o.primary.clone())
                    .unwrap_or_default()
            };
            let current = primary(module);
            let target = primary(target);
            let destination = owners.owners(&probe.to);
            let overlaps = |a: &[String], b: &[String]| a.iter().any(|owner| b.contains(owner));
            let conflict = !file.owners.is_empty()
                && ((!target.is_empty() && !overlaps(&file.owners, &target))
                    || (!destination.is_empty() && !overlaps(&file.owners, &destination)));
            let assessment = if conflict {
                "conflicts"
            } else if file.owners.is_empty() || target.is_empty() {
                "unknown"
            } else if !overlaps(&file.owners, &current) {
                "supports"
            } else {
                "shared"
            };
            PlacementOwnership {
                assessment,
                file_owners: file.owners.clone(),
                current_module_owners: current,
                target_module_owners: target,
                destination_owners: destination,
            }
        });
        let conflict = ownership
            .as_ref()
            .is_some_and(|o| o.assessment == "conflicts");
        if let Evidence::MisplacedFile {
            ownership: evidence,
            ..
        } = &mut f.evidence
        {
            *evidence = ownership;
        }
        if let Some(m) = suggested.filter(|_| !conflict) {
            f.simulation = Some(simulate::changes(g.doc, &after));
            f.suggestion = Some(Suggestion { move_to: m });
        }
        out.push(f);
    }
}

/// Whether a move raises the cohesion of every module receiving files by `MIN_COHESION_GAIN`,
/// and that of all the `modules` it touches, taken together, at all.
/// Forecast only the raw metrics that can reject a move; surviving probes still use `simulate`.
fn rejects_move_metrics<'a>(
    doc: &Document,
    rules: Option<&Compiled>,
    module_of: &BTreeMap<&str, &str>,
    file: &crate::model::File,
    to: &str,
    target: &str,
    incident: impl Iterator<Item = &'a (String, String)>,
) -> bool {
    if file.barrel
        || modules::is_test(&file.path, &doc.config)
        || file.class == Some(crate::model::ModuleClass::Tests)
        || modules::is_test(to, &doc.config)
        || file.module == target
        || doc.files.iter().any(|existing| existing.path == to)
        || modules::assigned(to, &doc.config, doc.source.language, rules) != target
    {
        return false;
    }
    let Some(own) = doc.modules.iter().find(|m| m.id == file.module) else {
        return false;
    };
    let Some(receiver) = doc.modules.iter().find(|m| m.id == target) else {
        return false;
    };
    let (mut own, mut receiver) = (own.clone(), receiver.clone());
    for (from, to) in incident {
        let (Some(&before_from), Some(&before_to)) =
            (module_of.get(from.as_str()), module_of.get(to.as_str()))
        else {
            return false;
        };
        let after_from = if from == &file.path {
            target
        } else {
            before_from
        };
        let after_to = if to == &file.path { target } else { before_to };
        for module in [&mut own, &mut receiver] {
            if !adjust_raw_metrics(module, before_from, before_to, false)
                || !adjust_raw_metrics(module, after_from, after_to, true)
            {
                return false;
            }
        }
    }
    let ratio = |internal: usize, outgoing: usize| {
        let total = internal + outgoing;
        if total == 0 {
            0.0
        } else {
            internal as f64 / total as f64
        }
    };
    let modules = [file.module.as_str(), target];
    receiver.outgoing_to.len() > receiver.ce
        || !(ratio(receiver.internal_edges, receiver.outgoing_edges)
            >= cohesion(doc, &[target]) + MIN_COHESION_GAIN - EPSILON
            && ratio(
                own.internal_edges + receiver.internal_edges,
                own.outgoing_edges + receiver.outgoing_edges,
            ) > cohesion(doc, &modules) + EPSILON)
}

/// A non-barrel rename preserves attributed file pairs; only the moved node's module changes.
fn adjust_raw_metrics(module: &mut crate::model::Module, from: &str, to: &str, add: bool) -> bool {
    if from != module.id {
        return true;
    }
    let bump = |count: &mut usize| {
        let next = if add {
            count.checked_add(1)
        } else {
            count.checked_sub(1)
        };
        if let Some(next) = next {
            *count = next;
            true
        } else {
            false
        }
    };
    if from == to {
        return bump(&mut module.internal_edges);
    }
    if !bump(&mut module.outgoing_edges) {
        return false;
    }
    let count = module.outgoing_to.entry(to.to_owned()).or_default();
    if !bump(count) {
        return false;
    }
    if *count == 0 {
        module.outgoing_to.remove(to);
    }
    true
}

fn gains(before: &Document, after: &Document, receivers: &[&str], modules: &[&str]) -> bool {
    receivers
        .iter()
        .all(|r| cohesion(after, &[r]) >= cohesion(before, &[r]) + MIN_COHESION_GAIN - EPSILON)
        && cohesion(after, modules) > cohesion(before, modules) + EPSILON
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

/// Whether `to` is a private module (`_internal`) of the package `from` sits in: a public facade
/// over its own implementation, as intended.
fn private_of(from: &str, to: &str) -> bool {
    let (parent, name) = to.rsplit_once('/').unwrap_or(("", to));
    name.starts_with('_')
        && (parent.is_empty()
            || from == parent
            || from
                .strip_prefix(parent)
                .is_some_and(|r| r.starts_with('/')))
}

/// Whether a Python file is public API: no path segment below the source root starts with `_`
/// (a package's `__init__.py` stands for the package).
fn python_public(path: &str, source_root: &str) -> bool {
    let rel = match source_root {
        "." => path,
        root => path
            .strip_prefix(root)
            .and_then(|r| r.strip_prefix('/'))
            .unwrap_or(path),
    };
    let module = rel.strip_suffix(".py").unwrap_or(rel);
    let module = module.strip_suffix("__init__").unwrap_or(module);
    module
        .split('/')
        .filter(|s| !s.is_empty())
        .all(|s| !s.starts_with('_'))
}

/// Whether a Go file is public API: no `internal` directory on its path (Go's own rule for what
/// other modules may import) and not in a `package main` (a command, which nothing imports).
fn go_public(path: &str, main_packages: &[String]) -> bool {
    let dir = path.rsplit_once('/').map_or(".", |(d, _)| d);
    !dir.split('/').any(|s| s == "internal") && !main_packages.iter().any(|m| m == dir)
}

/// The Go module (deepest of `go_modules`, go.mod directories) holding `path`; `None` outside
/// all of them, and for every path when there are none (other languages).
fn go_module<'m>(path: &str, go_modules: &'m [String]) -> Option<&'m str> {
    go_modules
        .iter()
        .filter(|m| {
            *m == "."
                || path
                    .strip_prefix(m.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
        .max_by_key(|m| if *m == "." { 0 } else { m.len() })
        .map(String::as_str)
}

/// Whether module `id`'s last path segment names a kind of code, in any case, singular or plural.
fn kind_folder(id: &str) -> bool {
    let name = id.rsplit('/').next().unwrap_or(id).to_lowercase();
    let singular = match name.strip_suffix("ies") {
        Some(stem) => format!("{stem}y"),
        None => name.strip_suffix('s').unwrap_or(&name).to_owned(),
    };
    KIND_FOLDERS
        .split_whitespace()
        .any(|kind| kind == name || kind == singular)
}

/// With rules: imports forbidden by a `public` rule, one finding per bound `public` module
/// (subject `src/domains/cart/**`), as `check` reports them. Without, TypeScript only: one
/// finding per module, over cross-module imports of a file its entries (barrels, package.json
/// entries) re-export, when at least `BARREL_MIN_SHARE` of the cross-module imports into that
/// module go through one of its entries; never from a module below the target's: importing its
/// entry, which re-exports the importer's own folder, would be a cycle. An import of a file no
/// entry offers uses something the module does not export: not a bypass.
fn internals_reach(g: &Graph, out: &mut Vec<Finding>) {
    let is_entry: HashSet<&str> = g.entries.values().flatten().copied().collect();
    // Per module: (cross-module imports into it, of which into one of its entries).
    let mut into: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for e in &g.doc.edges {
        let written = e.original_to.as_deref().unwrap_or(&e.to);
        let module = g
            .module_of
            .get(written)
            .copied()
            .unwrap_or(g.module_of[e.to.as_str()]);
        if g.module_of[e.from.as_str()] != module {
            let count = into.entry(module).or_default();
            count.0 += 1;
            count.1 += usize::from(is_entry.contains(written));
        }
    }
    // Per subject: the module, its entries, its barrel share, the imports that bypass them.
    type Reach<'e> = (&'e str, BTreeSet<String>, Option<f64>, Vec<&'e Edge>);
    let mut by_subject: BTreeMap<String, Reach> = BTreeMap::new();
    for e in &g.doc.edges {
        let written = e.original_to.as_deref().unwrap_or(&e.to);
        let module = g
            .module_of
            .get(written)
            .copied()
            .unwrap_or(g.module_of[e.to.as_str()]);
        let (subject, entries, share) = match g.rules {
            Some(rules)
                if e.verdict == Some(Verdict::Forbidden)
                    && matches!(e.rule.as_deref(), Some("public" | "entrypoints")) =>
            {
                let (public, entries) = match rules.public_for(written) {
                    Some(boundary) => boundary,
                    None => continue,
                };
                (public, entries, None)
            }
            None if g.doc.source.language == Language::Typescript
                && g.module_of[e.from.as_str()] != module
                && !is_entry.contains(written)
                && g.offered
                    .get(module)
                    .is_some_and(|o| o.contains(e.to.as_str()))
                && !g.is_ancestor(module, g.module_of[e.from.as_str()]) =>
            {
                let b: Vec<String> = g.entries[module].iter().map(|p| p.to_string()).collect();
                let (total, via) = into[module];
                let share = via as f64 / total as f64;
                if share < BARREL_MIN_SHARE - EPSILON {
                    continue;
                }
                (module.to_owned(), b, Some(crate::quality::round4(share)))
            }
            _ => continue,
        };
        let group = by_subject
            .entry(subject)
            .or_insert_with(|| (module, BTreeSet::new(), None, Vec::new()));
        group.1.extend(entries);
        group.2 = share;
        group.3.push(e);
    }
    for (subject, (module, entries, barrel_share, reaching)) in by_subject {
        let importers: BTreeSet<&str> = reaching
            .iter()
            .map(|e| g.module_of[e.from.as_str()])
            .collect();
        let files: BTreeSet<&str> = reaching.iter().map(|e| e.from.as_str()).collect();
        out.push(finding(
            FindingKind::InternalsReach,
            subject,
            Evidence::InternalsReach {
                module: module.to_owned(),
                entries: entries.into_iter().collect(),
                imports: reaching.len(),
                importers: importers.len(),
                importing_files: files.len(),
                barrel_share,
                edges: reaching
                    .iter()
                    .take(REACH_EDGES)
                    .map(|e| Link {
                        from: e.from.clone(),
                        to: e.to.clone(),
                    })
                    .collect(),
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
/// import of what it re-exports, with at least `SDP_MIN_IMPORTS` runtime file imports from A to
/// B (no barrel attribution: an import of a barrel elsewhere is not an import of B), and neither
/// module a folder above the other.
fn unstable_dependency(g: &Graph, out: &mut Vec<Finding>) {
    let instability: BTreeMap<&str, Option<f64>> = g
        .doc
        .modules
        .iter()
        .map(|m| (m.id.as_str(), m.instability))
        .collect();
    let mut runtime: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for e in
        g.doc.edges.iter().filter(|e| {
            e.kind.is_runtime() && !g.wiring.contains(&(e.from.as_str(), e.to.as_str()))
        })
    {
        *runtime
            .entry((g.module_of[e.from.as_str()], g.module_of[e.to.as_str()]))
            .or_default() += 1;
    }
    let pairs: BTreeSet<(&str, &str)> = g
        .pairs
        .iter()
        .map(|(a, b)| (g.module_of[a.as_str()], g.module_of[b.as_str()]))
        .filter(|(ma, mb)| ma != mb && !g.is_ancestor(ma, mb) && !g.is_ancestor(mb, ma))
        .filter(|(ma, mb)| !private_of(ma, mb))
        .collect();
    for (ma, mb) in pairs {
        let n = runtime.get(&(ma, mb)).copied().unwrap_or(0);
        let (Some(from), Some(to)) = (instability[ma], instability[mb]) else {
            continue;
        };
        if n >= SDP_MIN_IMPORTS && to - from >= SDP_MARGIN - EPSILON {
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
    for (a, b) in g.pairs {
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
    let single: HashSet<&str> = g
        .doc
        .modules
        .iter()
        .filter(|m| m.files == 1)
        .map(|m| m.id.as_str())
        .collect();
    let loose = g
        .doc
        .files
        .iter()
        .filter(|f| !f.barrel && single.contains(f.module.as_str()))
        .count();
    let folder_modules: Vec<&str> = g
        .doc
        .modules
        .iter()
        .map(|m| m.id.as_str())
        .filter(|m| !g.root_files.contains(m))
        .collect();
    let by_kind = folder_modules.iter().filter(|m| kind_folder(m)).count();
    if clusters.len() < 2
        || loose as f64 > SEAM_MAX_LOOSE_SHARE * non_barrel as f64 + EPSILON
        || by_kind as f64 > SEAM_MAX_KIND_SHARE * folder_modules.len() as f64 + EPSILON
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
            modularity: crate::quality::round4(clustered),
            folder_modularity: crate::quality::round4(folders),
            clusters,
            hubs: hubs.iter().map(|h| h.to_string()).collect(),
            via_barrel: g.via_barrel(used),
        },
    ));
}

/// Gives each registrar move of `doc`'s cycles a destination, simulated: a new folder named
/// after the file (`driver/DriverFactory.ts` to `driver/driver-factory/DriverFactory.ts`), in the
/// file's folder when that makes a module of its own at the scan's depth, else beside its
/// module's folder; and the module of its one importer, when it has exactly one. The best by
/// two-way pairs removed wins, among moves that add no new cycle; a move with none is dropped.
pub fn place_registrars(
    doc: &mut Document,
    rules: Option<&Compiled>,
    owners: Option<&crate::codeowners::Compiled>,
) {
    let module_of = |path: &str| modules::assigned(path, &doc.config, doc.source.language, rules);
    let exists: HashSet<&str> = doc.files.iter().map(|f| f.path.as_str()).collect();
    let modules: HashSet<&str> = doc.modules.iter().map(|m| m.id.as_str()).collect();
    let pairs = |d: &Document| -> i64 {
        d.cycles
            .iter()
            .filter_map(|c| c.two_way.as_ref())
            .map(|p| p.len() as i64)
            .sum()
    };
    let module_cycles = |d: &Document| -> Vec<BTreeSet<String>> {
        d.cycles
            .iter()
            .filter(|c| c.level == crate::model::CycleLevel::Module)
            .map(|c| c.members.iter().cloned().collect())
            .collect()
    };
    let before_pairs = pairs(doc);
    let before_cycles = module_cycles(doc);
    let (bound, go_modules) = (
        doc.source.collector.bound_paths(),
        &doc.source.collector.go_modules,
    );
    let other_module =
        |file: &str, to: &str| go_module(file, go_modules) != go_module(to, go_modules);
    let mut placed: Vec<Vec<RegistrarMove>> = Vec::new();
    for c in &doc.cycles {
        let mut kept = Vec::new();
        for r in c
            .registrar_moves
            .iter()
            .filter(|r| !bound.contains(r.file.as_str()))
        {
            let (dir, name) = r.file.rsplit_once('/').unwrap_or(("", &r.file));
            let stem = name.split('.').next().unwrap_or(name);
            let folder = kebab(stem);
            let mut candidates: Vec<String> = Vec::new();
            for base in [dir, modules::dir_of(dir)] {
                let to = modules::join(base, &format!("{folder}/{name}"));
                let lands = module_of(&to);
                if lands != r.module
                    && !modules.contains(lands.as_str())
                    && !exists.contains(to.as_str())
                    && !other_module(&r.file, &to)
                {
                    candidates.push(to);
                    break;
                }
            }
            let importers: BTreeSet<&str> = doc
                .edges
                .iter()
                .filter(|e| e.to == r.file)
                .map(|e| e.from.as_str())
                .collect();
            if let [only] = importers.into_iter().collect::<Vec<_>>().as_slice() {
                let to = modules::join(modules::dir_of(only), name);
                if module_of(&to) != r.module
                    && !exists.contains(to.as_str())
                    && !other_module(&r.file, &to)
                {
                    candidates.push(to);
                }
            }
            let mut best: Option<RegistrarMove> = None;
            for to in candidates {
                if ownership_conflict(owners, &r.file, &to) {
                    continue;
                }
                let moved = [Move {
                    file: r.file.clone(),
                    to: to.clone(),
                }];
                let Ok(after) = simulate::apply(doc, rules, &moved) else {
                    continue;
                };
                let landed = module_of(&to);
                // A cycle not inside one that existed (the new module aside) is new.
                let new_cycle = module_cycles(&after).iter().any(|m| {
                    let rest: BTreeSet<&String> = m.iter().filter(|x| **x != landed).collect();
                    !before_cycles
                        .iter()
                        .any(|b| rest.iter().all(|x| b.contains(*x)))
                });
                if new_cycle {
                    continue;
                }
                let removed = before_pairs - pairs(&after);
                // The registrar's module's cycle (0: it left every cycle); when the move empties
                // that module, the largest cycle the other members are left in.
                let after_cycles = module_cycles(&after);
                let cycle_after = if after.modules.iter().any(|m| m.id == r.module) {
                    after_cycles.iter().find(|m| m.contains(&r.module))
                } else {
                    after_cycles
                        .iter()
                        .filter(|m| c.members.iter().any(|x| m.contains(x)))
                        .max_by_key(|m| m.len())
                }
                .map_or(0, BTreeSet::len);
                if best
                    .as_ref()
                    .is_none_or(|b| removed > b.pairs_removed.unwrap_or(0))
                {
                    best = Some(RegistrarMove {
                        to: Some(to),
                        pairs_removed: Some(removed),
                        cycle_before: Some(c.members.len()),
                        cycle_after: Some(cycle_after),
                        ..r.clone()
                    });
                }
            }
            kept.extend(best);
        }
        placed.push(kept);
    }
    for (c, moves) in doc.cycles.iter_mut().zip(placed) {
        // A pair whose other side is only a moved registrar is resolved by the move, when the
        // move removes pairs at all.
        if let Some(pairs) = &mut c.two_way {
            for p in pairs.iter_mut() {
                p.resolved_by_move = moves.iter().any(|m| {
                    m.pairs_removed.is_some_and(|n| n > 0)
                        && m.module == p.to
                        && m.with.contains(&p.from)
                });
            }
            pairs.sort_by_key(|p| p.resolved_by_move);
        }
        c.registrar_moves = moves;
    }
}

fn ownership_conflict(owners: Option<&crate::codeowners::Compiled>, file: &str, to: &str) -> bool {
    owners.is_some_and(|owners| {
        let before = owners.owners(file);
        let after = owners.owners(to);
        !before.is_empty() && !after.is_empty() && !before.iter().any(|o| after.contains(o))
    })
}

/// `DriverFactory`, `LayerUI`, `HTTPServer` and `driver_factory` as `driver-factory`,
/// `layer-ui`, `http-server`, `driver-factory`: a hyphen where a word starts.
fn kebab(stem: &str) -> String {
    let chars: Vec<char> = stem.chars().collect();
    let mut out = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            out.push('-');
            continue;
        }
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1);
        let starts_word = ch.is_uppercase()
            && prev.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))
            });
        if starts_word && !out.ends_with('-') {
            out.push('-');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{go_public, python_public};

    #[test]
    fn kebab_starts_a_word_at_each_capital_run() {
        for (stem, want) in [
            ("DriverFactory", "driver-factory"),
            ("LayerUI", "layer-ui"),
            ("HTTPServer", "http-server"),
            ("database-connection", "database-connection"),
            ("use_cache", "use-cache"),
        ] {
            assert_eq!(super::kebab(stem), want, "{stem}");
        }
    }

    #[test]
    fn python_public_is_every_module_without_a_leading_underscore() {
        for (path, public) in [
            ("django/core/files/uploadedfile.py", true),
            ("django/__init__.py", true),
            ("django/core/__init__.py", true),
            ("django/_internal/x.py", false),
            ("django/utils/_os.py", false),
        ] {
            assert_eq!(python_public(path, "django"), public, "{path}");
        }
    }

    #[test]
    fn go_public_is_every_file_outside_internal_folders_and_commands() {
        let main = [".".to_owned(), "cmd/tool".to_owned()];
        for (path, public) in [
            ("store/db.go", true),
            ("internal/db.go", false),
            ("store/internal/x/db.go", false),
            ("internals/db.go", true),
            ("cmd/tool/main.go", false),
            ("cmd/tool/sub/x.go", true),
            ("main.go", false),
        ] {
            assert_eq!(go_public(path, &main), public, "{path}");
        }
    }
    fn metric_fixture(
        edges: Vec<crate::model::Edge>,
        custom: bool,
        barrel: bool,
        test: bool,
    ) -> crate::model::Document {
        use crate::model::{Collector, Language, NodeMetadata, Source};
        let mut config: crate::model::Config = serde_json::from_value(serde_json::json!({
            "depth": 1, "source_root": ".", "excludes": [], "include_tests": true, "rules": null
        }))
        .unwrap();
        let files: Vec<String> = [
            "old/a.ts",
            "old/z.ts",
            "target/b.ts",
            "target/c.ts",
            "other/d.ts",
            "other/index.ts",
            "third/e.ts",
            "tests/check.spec.ts",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for file in &files {
            config.file_metadata.insert(
                file.clone(),
                NodeMetadata {
                    barrel: file == "other/index.ts" || (file == "old/a.ts" && barrel),
                    test: file == "tests/check.spec.ts" || (file == "old/a.ts" && test),
                },
            );
        }
        if custom {
            config.rules = Some(crate::model::RulesRef {
                path: "layout.yaml".into(),
                sha256: String::new(),
                doc: serde_json::from_value(serde_json::json!({"version":1,
                    "modules":[{"id":"receiver","path":"target/**"}],
                    "edge_weights":{"runtime":0.8,"lazy":0.2,"type":0.05,"reexport":0.7}
                }))
                .unwrap(),
            });
        }
        let rules = crate::rules::compile(&config).unwrap();
        crate::build(
            Source {
                language: Language::Typescript,
                collector: Collector::default(),
                git: None,
                generated_at: "1970-01-01T00:00:00Z".into(),
                scan_root: None,
            },
            config,
            files,
            edges,
            Vec::new(),
            rules.as_ref(),
            None,
        )
    }

    fn metric_probe(doc: &crate::model::Document) -> (bool, bool) {
        let rules = crate::rules::compile(&doc.config).unwrap();
        let file = doc.files.iter().find(|f| f.path == "old/a.ts").unwrap();
        let target = &doc
            .files
            .iter()
            .find(|f| f.path == "target/b.ts")
            .unwrap()
            .module;
        let module_of = doc
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.module.as_str()))
            .collect();
        let barrels = doc
            .files
            .iter()
            .filter(|f| f.barrel)
            .map(|f| f.path.clone())
            .collect();
        let pairs = crate::modules::attribute(&doc.edges, &barrels);
        let incident = pairs
            .iter()
            .filter(|(from, to)| from == &file.path || to == &file.path);
        let rejected = super::rejects_move_metrics(
            doc,
            rules.as_ref(),
            &module_of,
            file,
            "target/a.ts",
            target,
            incident,
        );
        let after = crate::simulate::apply(
            doc,
            rules.as_ref(),
            &[crate::model::Move {
                file: file.path.clone(),
                to: "target/a.ts".into(),
            }],
        )
        .unwrap();
        let failed = after.summary.modules[target].ce > doc.summary.modules[target].ce
            || !super::gains(
                doc,
                &after,
                &[target.as_str()],
                &[file.module.as_str(), target.as_str()],
            );
        (rejected, failed)
    }

    #[test]
    fn misplaced_metric_precheck_retains_successes_and_rejects_extra_fanout() {
        let edge = |from: &str, to: &str| crate::model::Edge {
            from: from.into(),
            to: to.into(),
            ..Default::default()
        };
        let mut edges = vec![
            edge("target/b.ts", "old/a.ts"),
            edge("target/c.ts", "old/a.ts"),
        ];
        assert_eq!(
            metric_probe(&metric_fixture(edges.clone(), false, false, false)),
            (false, false)
        );
        edges.extend([
            edge("old/a.ts", "other/d.ts"),
            edge("old/a.ts", "third/e.ts"),
        ]);
        assert_eq!(
            metric_probe(&metric_fixture(edges, false, false, false)),
            (true, true)
        );
    }

    #[test]
    fn misplaced_metric_forecast_matches_simulation_across_kinds_barrels_rules_and_tests() {
        use crate::model::{Edge, EdgeKind};
        let names = [
            "old/a.ts",
            "old/z.ts",
            "target/b.ts",
            "target/c.ts",
            "other/d.ts",
            "other/index.ts",
            "third/e.ts",
            "tests/check.spec.ts",
        ];
        let kinds = [
            EdgeKind::Static,
            EdgeKind::Dynamic,
            EdgeKind::Type,
            EdgeKind::Reexport,
            EdgeKind::TypeReexport,
        ];
        let mut seed = 13u64;
        let mut rejected = 0;
        for turn in 0..240 {
            let mut edges = Vec::new();
            for from in names {
                for to in names {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    if from != to && (seed >> 32).is_multiple_of(5) {
                        edges.push(Edge {
                            from: from.into(),
                            to: to.into(),
                            kind: kinds[(seed >> 40) as usize % kinds.len()],
                            ..Default::default()
                        });
                    }
                }
            }
            let barrel = turn % 11 == 0;
            let test = turn % 13 == 0;
            let doc = metric_fixture(edges, turn % 2 == 0, barrel, test);
            let (skip, failed) = metric_probe(&doc);
            if barrel || test {
                assert!(!skip);
            } else {
                assert_eq!(skip, failed, "case {turn}");
            }
            rejected += usize::from(skip);
        }
        assert!(
            rejected > 50,
            "forecast must exercise actual rejections: {rejected}"
        );
    }
    #[test]
    fn misplaced_metric_forecast_defers_test_destinations_to_simulation() {
        let doc = metric_fixture(Vec::new(), false, false, false);
        let file = doc.files.iter().find(|f| f.path == "old/a.ts").unwrap();
        let module_of = doc
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.module.as_str()))
            .collect();
        assert!(!super::rejects_move_metrics(
            &doc,
            None,
            &module_of,
            file,
            "target/a.spec.ts",
            "target",
            std::iter::empty()
        ));
    }
}
