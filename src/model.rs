//! Schema `archgraph/v1`: the contract consumers read. See schema/archgraph.v1.schema.json.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rules::Rules;

pub const SCHEMA: &str = "archgraph/v1.1";
pub fn supported_schema(schema: &str) -> bool {
    matches!(schema, "archgraph/v1" | SCHEMA)
}

#[derive(Debug, Serialize)]
pub struct Document {
    pub schema: &'static str,
    pub kind: &'static str,
    pub tool: Tool,
    pub source: Source,
    pub config: Config,
    pub files: Vec<File>,
    pub edges: Vec<Edge>,
    /// Files a package.json publishes (`exports`, `main`, `module`, `types`), mapped back to
    /// their sources: public entries like barrels. TypeScript only.
    pub package_entries: Vec<String>,
    pub modules: Vec<Module>,
    pub module_edges: Vec<ModuleEdge>,
    pub cycles: Vec<Cycle>,
    pub type_only_cycles: Vec<DeferredCycle>,
    /// The counts and module metrics that finding simulations and `simulate` compare.
    pub summary: Summary,
    pub findings: Vec<Finding>,
    /// `null` unless scanned with `--cochange`.
    pub cochange: Option<Cochange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspots: Option<crate::hotspots::Hotspots>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbols: Option<crate::surface::Symbols>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<crate::surface::SurfaceReport>,
}

/// The parts of a scan document that every other subcommand recomputes the rest from.
#[derive(Debug, Deserialize)]
pub struct ScanInput {
    pub schema: String,
    pub kind: String,
    pub source: Source,
    pub config: Config,
    pub files: Vec<File>,
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub package_entries: Vec<String>,
    #[serde(default)]
    pub cochange: Option<Cochange>,
    #[serde(default)]
    pub hotspots: Option<crate::hotspots::Hotspots>,
    #[serde(default)]
    pub findings: Vec<FindingContext>,
    #[serde(default)]
    pub symbols: Option<crate::surface::Symbols>,
    #[serde(default)]
    pub surface: Option<crate::surface::SurfaceReport>,
}

#[derive(Debug, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub language: Language,
    pub collector: Collector,
    pub git: Option<Git>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Typescript,
    Python,
    Go,
    Rust,
    Java,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collector {
    pub name: String,
    pub version: Option<String>,
    pub cruised: usize,
    /// Only dependency-cruiser reports unresolved imports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved: Option<usize>,
    /// `unavailable` when dependency-cruiser could not tell imports of types only (written
    /// without `import type`) from value imports: they are `static`. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_marking: Option<String>,
    /// Go: the directories of `package main` packages (`.` for the root), sorted. Their files
    /// are commands, never public API. Absent otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub main_packages: Vec<String>,
    /// Go: files that cannot leave their package and still compile, sorted: a file declaring a
    /// method of a type declared in another file, or using an unexported name another file
    /// declares, and that other file, when both are scanned. None of them is proposed for a
    /// move. Absent otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bound_files: Vec<String>,
    /// Go: the directories holding a go.mod (`.` for the root), sorted. No move is proposed
    /// from one Go module into another. Absent otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub go_modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Git {
    pub commit: String,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub depth: usize,
    /// Go/Java without `--depth`: every package directory is a module; `depth` is unused.
    /// Absent otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub package_modules: bool,
    pub source_root: String,
    pub excludes: Vec<String>,
    pub include_tests: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_kinds: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_default_excludes: bool,
    pub rules: Option<RulesRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codeowners: Option<crate::codeowners::Codeowners>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionOptions {
    pub python_package: Option<String>,
    pub rust_features: Option<crate::collect::rust::Features>,
    pub rust_build_scripts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesRef {
    /// File name only, so no absolute or working-directory-dependent path enters the document.
    pub path: String,
    pub sha256: String,
    pub doc: Rules,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub module: String,
    pub barrel: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_reexport: bool,
    pub verdict: Option<Verdict>,
    pub rule: Option<String>,
    /// For a `reachable` forbid: the import path from `to` to the matching file. Printed by
    /// `check`, not part of the document.
    #[serde(skip)]
    pub reach: Option<Vec<String>>,
}

/// Declared in ascending precedence: when a pair is reported more than once the greatest wins,
/// so a pair is `type` only when every report of it is type-only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Type,
    /// `export type … from`: a re-export for barrels, erased at runtime.
    #[serde(rename = "type-reexport")]
    TypeReexport,
    #[serde(rename = "lazy", alias = "dynamic")]
    Dynamic,
    Reexport,
    #[default]
    #[serde(rename = "runtime", alias = "static")]
    Static,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "runtime",
            Self::Dynamic => "lazy",
            Self::Type | Self::TypeReexport => "type",
            Self::Reexport => "reexport",
        }
    }
    /// Structural re-exports are accounted separately from runtime dependencies.
    pub fn is_runtime(self) -> bool {
        self == EdgeKind::Static
    }
}

impl Serialize for EdgeKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Allowed,
    Forbidden,
    Unplanned,
}

#[derive(Debug, Serialize)]
pub struct Module {
    pub id: String,
    pub files: usize,
    pub ca: usize,
    pub ce: usize,
    pub instability: Option<f64>,
    pub internal_edges: usize,
    pub outgoing_edges: usize,
    pub cohesion: Option<f64>,
    /// `outgoing_edges` split by target module, counted the same way (barrel-attributed).
    pub outgoing_to: BTreeMap<String, usize>,
    pub weighted_ca: f64,
    pub weighted_ce: f64,
    pub weighted_internal_edges: f64,
    pub weighted_outgoing_edges: f64,
    pub weighted_outgoing_to: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership: Option<ModuleOwnership>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleOwnership {
    pub owned_files: usize,
    /// Shared files count once for every declared owner.
    pub owners: BTreeMap<String, usize>,
    /// Owners of a strict majority of all files, including unowned files in the denominator.
    pub primary: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PlacementOwnership {
    pub assessment: &'static str,
    pub file_owners: Vec<String>,
    pub current_module_owners: Vec<String>,
    pub target_module_owners: Vec<String>,
    pub destination_owners: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ModuleEdge {
    pub from: String,
    pub to: String,
    pub imports: usize,
    pub forbidden: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cycle {
    pub level: CycleLevel,
    pub members: Vec<String>,
    pub exact: bool,
    #[serde(rename = "break")]
    pub break_set: Vec<Link>,
    /// Module cycles only: every pair of members importing each other, as its lighter direction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub two_way: Option<Vec<TwoWay>>,
    /// Module cycles only: registrars that alone form the other side of several two-way pairs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub registrar_moves: Vec<RegistrarMove>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeferredCycle {
    pub level: CycleLevel,
    pub members: Vec<String>,
    pub kinds: Vec<String>,
    pub contains_runtime_cycle: bool,
}

/// A registrar (a file wiring many modules) whose imports alone form the other side of `pairs`
/// two-way pairs of a cycle, with the modules in `with`: moving it out of `module` to `to` opens
/// them. `pairs_removed` is the net change in two-way pairs over all module cycles, and the
/// cycle shrinks from `cycle_before` to `cycle_after` modules (0: gone), as simulated.
#[derive(Debug, Clone, Serialize)]
pub struct RegistrarMove {
    pub file: String,
    pub module: String,
    pub pairs: usize,
    pub with: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairs_removed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_before: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_after: Option<usize>,
}

impl RegistrarMove {
    /// Whether the move comes before the cycle's cut: it removes more two-way pairs than a
    /// single-import cut would (one, when the cycle has one), and the cycle shrinks, since
    /// removing pairs while the cycle keeps or grows its size only reroutes them into other rings.
    /// The page applies the same rule.
    pub fn leads(&self, single_cut: bool) -> bool {
        let (Some(removed), Some(before), Some(after)) =
            (self.pairs_removed, self.cycle_before, self.cycle_after)
        else {
            return false;
        };
        removed > i64::from(single_cut) && after < before
    }
}

/// The lighter direction of two modules importing each other (the names' order on a tie): the
/// cheapest cut a person can act on.
#[derive(Debug, Clone, Serialize)]
pub struct TwoWay {
    pub from: String,
    pub to: String,
    pub imports: usize,
    pub reverse_imports: usize,
    /// Both directions have at most `graph::EVEN_MAX_IMPORTS` imports, within a factor of 2: a
    /// mutual dependency with no lighter side, `from` and `to` in the names' order.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub even: bool,
    /// `from` -> `to` is only composition roots and registrars wiring modules together: not a
    /// cut to offer first.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub registry: bool,
    /// `from` -> `to` holds a public entry's re-export, and so does the other way.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub entry: bool,
    /// The other way is only a registrar the cycle's `registrar_moves` moves: that move resolves
    /// the pair, and cutting `from` -> `to` would point the wrong way.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolved_by_move: bool,
    /// The file imports from `from` to `to`, at most `graph::TWO_WAY_EDGES` of them.
    pub edges: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CycleLevel {
    Module,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
}

/// Declared alphabetically, so the derived `Ord` sorts findings by kind name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    Cycle,
    HiddenCoupling,
    InternalsReach,
    MisplacedFile,
    Seam,
    SinkSplit,
    SmallRepo,
    UnstableDependency,
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub subject: String,
    pub evidence: Evidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<Suggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation: Option<FindingSimulation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub introduced_by: Option<crate::bisect::Commit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspot_priority: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub status: String,
    pub method: String,
    pub from: Option<String>,
    pub to: String,
    pub previous_commit: Option<String>,
    pub closing_edge: Option<Link>,
    pub limitations: Vec<String>,
    pub scans: usize,
}

#[derive(Debug, Deserialize)]
pub struct FindingContext {
    pub kind: String,
    pub subject: String,
    #[serde(default)]
    pub age_days: Option<u64>,
    #[serde(default)]
    pub introduced_by: Option<crate::bisect::Commit>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

/// One shape per finding kind, each named in the schema's `$defs`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Evidence {
    SinkSplit {
        ca: usize,
        components: Vec<Component>,
        via_barrel: bool,
    },
    MisplacedFile {
        /// Edges in either direction between the file and the rest of its own module.
        own_module_edges: usize,
        target_module: String,
        /// Files of the target module that import the file.
        imports_from_target: usize,
        via_barrel: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        ownership: Option<PlacementOwnership>,
    },
    Cycle(Cycle),
    HiddenCoupling {
        files: [String; 2],
        modules: [String; 2],
        commits: usize,
    },
    InternalsReach {
        module: String,
        entries: Vec<String>,
        /// Imports that bypass the entries, from `importers` distinct modules and
        /// `importing_files` distinct files.
        imports: usize,
        importers: usize,
        importing_files: usize,
        /// Without rules: the share of the cross-module imports into `module` that go through
        /// one of its barrels.
        #[serde(skip_serializing_if = "Option::is_none")]
        barrel_share: Option<f64>,
        /// At most `findings::REACH_EDGES` of the bypassing imports.
        edges: Vec<Link>,
    },
    UnstableDependency {
        from: String,
        to: String,
        from_instability: f64,
        to_instability: f64,
        imports: usize,
    },
    Seam {
        modularity: f64,
        folder_modularity: f64,
        clusters: Vec<Cluster>,
        hubs: Vec<String>,
        via_barrel: bool,
    },
    SmallRepo {
        files: usize,
        modules: usize,
    },
}

#[derive(Debug, Serialize)]
pub struct Component {
    pub files: Vec<String>,
    pub consumers: Vec<String>,
    /// Each file into the consumer's directory when there is exactly one consumer.
    pub moves: Vec<Move>,
}

#[derive(Debug, Serialize)]
pub struct Cluster {
    pub files: Vec<String>,
    pub modules: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Suggestion {
    #[serde(rename = "move")]
    pub move_to: Move,
}

#[derive(Debug, Clone, Serialize)]
pub struct Move {
    pub file: String,
    pub to: String,
}

/// A proposed move's result; the state before it is the scan document's `summary`.
#[derive(Debug, Serialize)]
pub struct FindingSimulation {
    pub after: Changes,
}

/// A Summary after a move, listing only the modules whose metrics changed: `null` for a module
/// the move empties, a new entry for a module it creates.
#[derive(Debug, Serialize)]
pub struct Changes {
    pub forbidden: usize,
    pub unplanned: usize,
    pub cycles: usize,
    pub type_only_cycles: usize,
    pub modules: BTreeMap<String, Option<ModuleSummary>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub forbidden: usize,
    pub unplanned: usize,
    pub cycles: usize,
    pub type_only_cycles: usize,
    pub modules: BTreeMap<String, ModuleSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleSummary {
    pub cohesion: Option<f64>,
    pub ca: usize,
    pub ce: usize,
    pub weighted_ca: f64,
    pub weighted_ce: f64,
    pub instability: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Simulation {
    pub schema: &'static str,
    pub kind: &'static str,
    #[serde(rename = "move")]
    pub move_to: MovedFile,
    pub before: Summary,
    pub after: Summary,
    pub new_forbidden: Vec<Edge>,
    pub removed_forbidden: Vec<Edge>,
}

#[derive(Debug, Serialize)]
pub struct MovedFile {
    pub file: String,
    pub to: String,
    pub module: String,
}

/// Git history read by `scan --cochange`, kept in the document so every subcommand that
/// recomputes findings sees the same history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cochange {
    /// The number of most recent commits asked for.
    pub limit: usize,
    /// Commits read, not counting skipped ones.
    pub commits: usize,
    /// Commits left out because they touch too many files.
    pub skipped: usize,
    /// Pairs of scanned files changed together in enough commits, in any modules.
    pub pairs: Vec<CochangePair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CochangePair {
    pub files: [String; 2],
    pub commits: usize,
}

#[derive(Debug, Serialize)]
pub struct Diff {
    pub schema: &'static str,
    pub kind: &'static str,
    pub old: Snapshot,
    pub new: Snapshot,
    pub modules: BTreeMap<String, ModuleDelta>,
    pub added_modules: Vec<String>,
    pub removed_modules: Vec<String>,
    pub new_cycles: Vec<Cycle>,
    pub resolved_cycles: Vec<Cycle>,
    pub new_forbidden: Vec<Edge>,
    pub resolved_forbidden: Vec<Edge>,
    pub new_unplanned: Vec<Edge>,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub commit: Option<String>,
    pub generated_at: String,
}

/// `[old, new]` per metric.
#[derive(Debug, Serialize)]
pub struct ModuleDelta {
    pub cohesion: [Option<f64>; 2],
    pub ca: [usize; 2],
    pub ce: [usize; 2],
    pub weighted_ca: [f64; 2],
    pub weighted_ce: [f64; 2],
    pub instability: [Option<f64>; 2],
}

#[cfg(test)]
mod tests {
    use super::RegistrarMove;

    fn mv(removed: i64, before: usize, after: usize) -> RegistrarMove {
        RegistrarMove {
            file: "a/r.ts".into(),
            module: "a".into(),
            pairs: 3,
            with: vec![],
            to: Some("a/r/r.ts".into()),
            pairs_removed: Some(removed),
            cycle_before: Some(before),
            cycle_after: Some(after),
        }
    }

    #[test]
    fn a_registrar_move_leads_only_when_it_beats_the_cut_and_the_cycle_shrinks() {
        // Against a single-import cut (one pair) a move must remove more; with no such cut,
        // any. Either way the cycle must shrink.
        assert!(mv(10, 27, 5).leads(true));
        assert!(!mv(2, 5, 5).leads(true));
        assert!(mv(2, 5, 4).leads(true));
        assert!(!mv(1, 5, 0).leads(true));
        assert!(mv(1, 5, 0).leads(false));
        assert!(!mv(13, 32, 33).leads(true));
        assert!(!mv(13, 32, 33).leads(false));
        assert!(!mv(0, 5, 5).leads(false));
    }
}
