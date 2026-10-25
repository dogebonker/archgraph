//! layout.yaml: parsing, glob compilation and edge verdicts.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{Config, Edge, EdgeKind, Link, Move, RulesRef, Verdict};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rules {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_weights: Option<EdgeWeights>,
    #[serde(
        rename = "pkg-category",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub pkg_category: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_exposure: Option<crate::exposure::Thresholds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grabbag: Option<crate::grabbag::Thresholds>,
    #[serde(
        rename = "early-smells",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub early_smells: Option<crate::early_smells::Thresholds>,
    #[serde(default)]
    pub modules: Vec<ModuleRule>,
    #[serde(default)]
    pub layers: Vec<String>,
    #[serde(
        rename = "layer-reason",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub layer_reason: Option<String>,
    #[serde(default)]
    pub public: Vec<PublicRule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entrypoints: BTreeMap<String, String>,
    #[serde(default)]
    pub forbid: Vec<Forbid>,
    #[serde(default)]
    pub allow: Vec<Allow>,
    /// The verdict for an import no other step decides; absent means `unplanned`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Fallback>,
    /// Where files go in the target, first match wins; only for cost and progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<MoveRule>,
}

/// Relative contribution of each import kind to coupling and cohesion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeWeights {
    pub runtime: f64,
    pub lazy: f64,
    #[serde(rename = "type")]
    pub type_only: f64,
    pub reexport: f64,
}

impl Default for EdgeWeights {
    fn default() -> Self {
        Self {
            runtime: 1.0,
            lazy: 0.3,
            type_only: 0.1,
            reexport: 0.0,
        }
    }
}

impl EdgeWeights {
    pub fn weight(self, kind: EdgeKind) -> f64 {
        match kind {
            EdgeKind::Static => self.runtime,
            EdgeKind::Dynamic => self.lazy,
            EdgeKind::Type | EdgeKind::TypeReexport => self.type_only,
            EdgeKind::Reexport => self.reexport,
        }
    }
}

fn includes_kind(kinds: &Option<Vec<EdgeKind>>, kind: EdgeKind) -> bool {
    kinds.as_ref().is_none_or(|kinds| {
        kinds.iter().any(|selected| {
            *selected == kind
                || matches!(
                    (selected, kind),
                    (
                        EdgeKind::Type | EdgeKind::TypeReexport,
                        EdgeKind::Type | EdgeKind::TypeReexport
                    )
                )
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fallback {
    Unplanned,
    Forbidden,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleRule {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublicRule {
    pub module: String,
    pub entries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<EdgeKind>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Forbid {
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub reachable: bool,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<EdgeKind>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MoveRule {
    pub from: String,
    /// A folder ending in `/`, which may use `from`'s capture.
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Allow {
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<EdgeKind>>,
}

impl Rules {
    /// Whether a `forbid` or `allow` target is a `global:` selector.
    pub fn names_globals(&self) -> bool {
        self.forbid
            .iter()
            .map(|r| &r.to)
            .chain(self.allow.iter().map(|r| &r.to))
            .any(|to| to.starts_with("global:"))
    }

    /// Reasons for the decision label retained on an edge, including implicit layer order.
    pub fn reasons(&self, decision: &str) -> Vec<&str> {
        let mut reasons = Vec::new();
        if decision == "layers" {
            reasons.extend(self.layer_reason.as_deref());
        }
        for (index, rule) in self.forbid.iter().enumerate() {
            if rule.id.as_deref().map_or_else(
                || decision == format!("forbid[{index}]"),
                |id| id == decision,
            ) {
                reasons.extend(rule.reason.as_deref());
            }
        }
        for (index, rule) in self.allow.iter().enumerate() {
            if rule.id.as_deref().map_or_else(
                || decision == format!("allow[{index}]"),
                |id| id == decision,
            ) {
                reasons.extend(rule.reason.as_deref());
            }
        }
        // Rules sharing an id (one claim) often repeat its reason: print each once.
        let mut seen = HashSet::new();
        reasons.retain(|r| seen.insert(*r));
        reasons
    }

    /// ADR paths for the decision label retained on an edge.
    pub fn sources(&self, decision: &str) -> Vec<&str> {
        let mut sources = Vec::new();
        for (index, rule) in self.forbid.iter().enumerate() {
            if rule.id.as_deref().map_or_else(
                || decision == format!("forbid[{index}]"),
                |id| id == decision,
            ) {
                sources.extend(rule.source.as_deref());
            }
        }
        for (index, rule) in self.allow.iter().enumerate() {
            if rule.id.as_deref().map_or_else(
                || decision == format!("allow[{index}]"),
                |id| id == decision,
            ) {
                sources.extend(rule.source.as_deref());
            }
        }
        let mut seen = HashSet::new();
        sources.retain(|s| seen.insert(*s));
        sources
    }
}

/// The rules in effect for `config`, compiled.
pub fn compile(config: &Config) -> Result<Option<Compiled>> {
    config
        .rules
        .as_ref()
        .map(|r| Compiled::new(&r.doc))
        .transpose()
}

/// Reads, validates and hashes a rules file. Unknown keys are an error here, so a typo cannot
/// silently allow everything; rules embedded in a scan document are read without that check, so
/// a document from a newer boundrail stays readable.
pub fn load(path: &Path) -> Result<RulesRef> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let parse = || -> Result<Rules> {
        let given: serde_json::Value = serde_yaml_ng::from_slice(&bytes)?;
        if let Some((key, at)) = unknown_key(&given, &known_keys(), "") {
            match at.as_str() {
                "" => bail!("unknown field `{key}`"),
                at => bail!("unknown field `{key}` in {at}"),
            }
        }
        Ok(serde_yaml_ng::from_slice(&bytes)?)
    };
    let doc = parse().with_context(|| format!("parsing {}", path.display()))?;
    Compiled::new(&doc).with_context(|| format!("in {}", path.display()))?;
    Ok(RulesRef {
        path: path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        sha256: Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        doc,
        known: crate::known::load(path)?,
    })
}

/// Every key the rules format knows, as one serialized rule of each kind.
fn known_keys() -> serde_json::Value {
    serde_json::to_value(Rules {
        modules: vec![ModuleRule::default()],
        public: vec![PublicRule {
            kinds: Some(vec![EdgeKind::Static]),
            ..PublicRule::default()
        }],
        entrypoints: BTreeMap::from([("module".into(), "entry".into())]),
        forbid: vec![Forbid {
            source: Some("docs/adr/ADR-001.md".into()),
            kinds: Some(vec![EdgeKind::Static]),
            ..Forbid::default()
        }],
        allow: vec![Allow {
            source: Some("docs/adr/ADR-001.md".into()),
            kinds: Some(vec![EdgeKind::Static]),
            ..Allow::default()
        }],
        edge_weights: Some(EdgeWeights::default()),
        pkg_category: BTreeMap::from([("orm".into(), vec!["typeorm".into()])]),
        external_exposure: Some(crate::exposure::Thresholds::default()),
        grabbag: Some(crate::grabbag::Thresholds::default()),
        early_smells: Some(crate::early_smells::Thresholds::default()),
        layer_reason: Some("Keep foundational layers independent.".into()),
        exclude: vec!["excluded/**".into()],
        include: vec!["included/**".into()],
        default: Some(Fallback::Forbidden),
        moves: vec![MoveRule::default()],
        ..Rules::default()
    })
    .expect("rules serialize")
}

/// The first key of `given` that `known` lacks, with where it sits (`forbid[0]`); array items
/// are compared with `known`'s first item.
pub(crate) fn unknown_key(
    given: &serde_json::Value,
    known: &serde_json::Value,
    at: &str,
) -> Option<(String, String)> {
    use serde_json::Value::{Array, Object};
    if matches!(at, "entrypoints" | "pkg-category") {
        return None;
    }
    match (given, known) {
        (Object(given), Object(known)) => {
            given.iter().find_map(|(key, value)| match known.get(key) {
                Some(known) => {
                    let path = if at.is_empty() {
                        key.clone()
                    } else {
                        format!("{at}.{key}")
                    };
                    unknown_key(value, known, &path)
                }
                None => Some((key.clone(), at.to_owned())),
            })
        }
        (Array(items), Array(known)) => known.first().and_then(|known| {
            items
                .iter()
                .enumerate()
                .find_map(|(i, item)| unknown_key(item, known, &format!("{at}[{i}]")))
        }),
        _ => None,
    }
}

fn glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .with_context(|| format!("glob {pattern}"))?
        .compile_matcher())
}

/// The `<name>` tokens in `text`, each required to be a whole path segment.
fn captures(text: &str) -> Result<Vec<&str>> {
    let mut found = Vec::new();
    for segment in text.split('/') {
        if !segment.contains(['<', '>']) {
            continue;
        }
        let name = segment.strip_prefix('<').and_then(|s| s.strip_suffix('>'));
        if !name
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_'))
        {
            bail!("capture in `{text}` must be a whole path segment like `<name>`");
        }
        if !found.contains(&segment) {
            found.push(segment);
        }
    }
    Ok(found)
}

/// A glob where an optional `<name>` segment matches one segment and can be bound to it.
struct Pattern {
    raw: String,
    capture: Option<String>,
    any: GlobMatcher,
}

impl Pattern {
    fn new(raw: &str) -> Result<Pattern> {
        let capture = match captures(raw)?.as_slice() {
            [] => None,
            [one] => Some(one.to_string()),
            _ => bail!("`{raw}` has more than one capture name"),
        };
        let any = match &capture {
            Some(token) => glob(&raw.replace(token.as_str(), "*"))?,
            None => glob(raw)?,
        };
        Ok(Pattern {
            raw: raw.to_owned(),
            capture,
            any,
        })
    }

    /// `template` (this pattern or one that may use only its capture) with the capture bound.
    fn instance(&self, template: &str, value: Option<&str>) -> String {
        match (&self.capture, value) {
            (Some(token), Some(value)) => template.replace(token.as_str(), value),
            _ => template.to_owned(),
        }
    }

    /// Like `instance`, for use as a glob: the bound value matches literally.
    fn bound_glob(&self, template: &str, value: Option<&str>) -> String {
        self.instance(template, value.map(globset::escape).as_deref())
    }

    fn check_template(&self, template: &str) -> Result<()> {
        for token in captures(template)? {
            if self.capture.as_deref() != Some(token) {
                bail!(
                    "`{template}` uses {token}, which `{}` does not bind",
                    self.raw
                );
            }
        }
        Ok(())
    }
}

/// A reachable forbid's import path by (rule index, bound capture, start file).
type Reached<'a> = HashMap<(usize, Option<String>, &'a str), Option<Vec<String>>>;

type CompiledForbid = (String, Pattern, String, bool, Option<Vec<EdgeKind>>);

/// One move rule in one slice: files still matching `from` with their destinations, and the
/// number of files already below `to`.
#[derive(Debug, Default)]
pub struct MoveEvidence {
    pub pending: Vec<Move>,
    pub arrived: usize,
}

/// One forbid rule in one slice: files matching `from`, targets matching `to`, imports it forbids.
#[derive(Debug, Default)]
pub struct SliceEvidence {
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub hits: Vec<Link>,
}

impl SliceEvidence {
    /// Whether the rule speaks about this slice: files on both sides, or an import it forbids.
    pub fn bound(&self) -> bool {
        !self.hits.is_empty() || (!self.from.is_empty() && !self.to.is_empty())
    }
}

pub struct Compiled {
    categories: BTreeMap<String, Vec<GlobMatcher>>,
    modules: Vec<(Pattern, String)>,
    layers: Vec<GlobMatcher>,
    public: Vec<(Pattern, Vec<String>, Option<Vec<EdgeKind>>)>,
    entrypoints: Vec<(Pattern, String)>,
    /// (name, from, to, reachable); `to` may use only `from`'s capture, bound to the importer.
    forbid: Vec<CompiledForbid>,
    allow: Vec<(String, Pattern, String, Option<Vec<EdgeKind>>)>,
    /// (from, to, `to` + `**`).
    moves: Vec<(Pattern, String, Pattern)>,
    /// Step 7 gives forbidden (rule `default`) instead of unplanned.
    default_forbidden: bool,
    /// Globs with a capture bound, compiled on first use.
    bound: RefCell<HashMap<String, GlobMatcher>>,
}

impl Compiled {
    pub fn new(rules: &Rules) -> Result<Compiled> {
        if let Some(reason) = &rules.layer_reason
            && (reason.trim().is_empty() || reason.chars().any(char::is_control))
        {
            bail!("layer-reason must be nonempty and contain no control characters");
        }
        // Ids and reasons are printed by check, review and SARIF: a blank one says nothing.
        let named = rules
            .forbid
            .iter()
            .map(|r| ("forbid", r.id.as_deref(), r.reason.as_deref()));
        let allowed = rules
            .allow
            .iter()
            .map(|r| ("allow", r.id.as_deref(), r.reason.as_deref()));
        let mut index: BTreeMap<&str, usize> = BTreeMap::new();
        for (kind, id, reason) in named.chain(allowed) {
            let i = index.entry(kind).or_default();
            for (field, value) in [("id", id), ("reason", reason)] {
                if value.is_some_and(|v| v.trim().is_empty()) {
                    bail!("{kind}[{i}].{field} must be nonempty");
                }
            }
            *i += 1;
        }
        for source in rules
            .forbid
            .iter()
            .filter_map(|rule| rule.source.as_deref())
            .chain(rules.allow.iter().filter_map(|rule| rule.source.as_deref()))
        {
            if source.is_empty()
                || source.contains(['\\', ':'])
                || source.chars().any(char::is_control)
                || source
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                bail!("rule source must be a normalized repository-relative path: {source:?}");
            }
        }
        if rules.version != 1 {
            bail!("unsupported rules version {}; expected 1", rules.version);
        }
        crate::collect::globset(&rules.exclude).context("exclude patterns")?;
        crate::collect::globset(&rules.include).context("include patterns")?;
        if let Some(thresholds) = rules.grabbag {
            thresholds.validate()?;
        }
        if let Some(thresholds) = &rules.early_smells {
            thresholds.validate()?;
        }
        if let Some(weights) = rules.edge_weights {
            for (name, weight) in [
                ("runtime", weights.runtime),
                ("lazy", weights.lazy),
                ("type", weights.type_only),
                ("reexport", weights.reexport),
            ] {
                if !weight.is_finite() || weight < 0.0 {
                    bail!("edge_weights.{name} must be finite and non-negative");
                }
            }
        }
        // Ids, reasons, globs and module ids are printed into terminals, CI logs and job
        // summaries, where a line break could forge a workflow command.
        let strings = rules
            .modules
            .iter()
            .flat_map(|m| [&m.id, &m.path])
            .chain(&rules.layers)
            .chain(&rules.exclude)
            .chain(&rules.include)
            .chain(
                rules
                    .entrypoints
                    .iter()
                    .flat_map(|(module, entry)| [module, entry]),
            )
            .chain(
                rules
                    .public
                    .iter()
                    .flat_map(|p| [&p.module].into_iter().chain(&p.entries)),
            )
            .chain(rules.forbid.iter().flat_map(|f| {
                [&f.from, &f.to]
                    .into_iter()
                    .chain(f.id.iter())
                    .chain(f.reason.iter())
            }))
            .chain(rules.allow.iter().flat_map(|a| {
                [&a.from, &a.to]
                    .into_iter()
                    .chain(a.id.iter())
                    .chain(a.reason.iter())
            }))
            .chain(rules.moves.iter().flat_map(|m| [&m.from, &m.to]));
        for s in strings {
            if s.chars().any(char::is_control) {
                bail!("{s:?} holds a control character; rules strings must be one plain line");
            }
        }
        let mut categories = BTreeMap::new();
        for (name, packages) in &rules.pkg_category {
            if name.is_empty()
                || name
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || matches!(c, ':' | '<' | '>'))
                || packages.is_empty()
            {
                bail!("pkg-category names and package lists must be non-empty plain names");
            }
            for package in packages {
                if package.is_empty()
                    || package.chars().any(char::is_control)
                    || !captures(package)?.is_empty()
                {
                    bail!("pkg-category packages must be non-empty package globs without captures");
                }
            }
            categories.insert(
                name.clone(),
                packages
                    .iter()
                    .map(|p| glob(p))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        if let Some(thresholds) = rules.external_exposure {
            thresholds.validate()?;
        }
        let mut modules = Vec::new();
        for m in &rules.modules {
            let path = Pattern::new(&m.path)?;
            path.check_template(&m.id)?;
            modules.push((path, m.id.clone()));
        }
        let mut public = Vec::new();
        for p in &rules.public {
            let module = Pattern::new(&p.module)?;
            for entry in &p.entries {
                module.check_template(entry)?;
                glob(&module.instance(entry, Some("x")))?;
            }
            public.push((module, p.entries.clone(), p.kinds.clone()));
        }
        let mut entrypoints = Vec::new();
        for (raw, entry) in &rules.entrypoints {
            let module = Pattern::new(raw)?;
            module.check_template(entry)?;
            let path = module.instance(entry, Some("x"));
            if path.is_empty()
                || path.contains(['\\', ':', '*', '?', '[', ']', '{', '}'])
                || path.split('/').any(|part| matches!(part, "" | "." | ".."))
            {
                bail!("entrypoint `{entry}` must be an exact relative file path");
            }
            entrypoints.push((module, entry.clone()));
        }
        let name = |id: &Option<String>, kind: &str, i: usize| {
            id.clone().unwrap_or_else(|| format!("{kind}[{i}]"))
        };
        let pair = |from: &str, to: &str| -> Result<Pattern> {
            if ["pkg:", "category:", "global:"]
                .iter()
                .any(|prefix| from.starts_with(prefix))
            {
                bail!("package and global selectors are supported only as rule targets (`to`)");
            }
            let from = Pattern::new(from)?;
            from.check_template(to)?;
            if let Some(name) = to.strip_prefix("category:") {
                if !categories.contains_key(name) {
                    bail!("unknown or empty package category `{name}`");
                }
                if !captures(to)?.is_empty() {
                    bail!("package categories do not support captures");
                }
            } else if let Some(package) = to.strip_prefix("pkg:") {
                if package.is_empty() {
                    bail!("pkg: requires a package glob");
                }
                glob(&from.instance(package, Some("x")))?;
            } else if let Some(name) = to.strip_prefix("global:") {
                if name.is_empty() || !captures(to)?.is_empty() {
                    bail!("global: requires a name glob without captures");
                }
                glob(name)?;
            } else {
                glob(&from.instance(to, Some("x")))?;
            }
            Ok(from)
        };
        let mut layers = Vec::new();
        for layer in &rules.layers {
            if !captures(layer)?.is_empty() {
                bail!("layer `{layer}`: layers bind no capture; write `*` for one segment");
            }
            layers.push(glob(layer)?);
        }
        let mut moves = Vec::new();
        for (i, m) in rules.moves.iter().enumerate() {
            if ["pkg:", "category:", "global:"]
                .iter()
                .any(|prefix| m.from.starts_with(prefix))
            {
                bail!("moves[{i}].from must be a file glob, not a package or global selector");
            }
            let from = Pattern::new(&m.from).with_context(|| format!("moves[{i}].from"))?;
            from.check_template(&m.to)
                .with_context(|| format!("moves[{i}].to"))?;
            if m.to.contains(['*', '?', '[', ']', '{', '}'])
                || !m
                    .to
                    .strip_suffix('/')
                    .is_some_and(|dir| crate::collect::is_relative_path(dir, false))
            {
                bail!(
                    "moves[{i}].to must be a folder ending in `/` without globs: {:?}",
                    m.to
                );
            }
            let arrived = Pattern::new(&format!("{}**", m.to))?;
            moves.push((from, m.to.clone(), arrived));
        }
        Ok(Compiled {
            modules,
            layers,
            public,
            entrypoints,
            forbid: rules
                .forbid
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    Ok((
                        name(&f.id, "forbid", i),
                        pair(&f.from, &f.to)?,
                        f.to.clone(),
                        f.reachable,
                        f.kinds.clone(),
                    ))
                })
                .collect::<Result<_>>()?,
            allow: rules
                .allow
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    Ok((
                        name(&a.id, "allow", i),
                        pair(&a.from, &a.to)?,
                        a.to.clone(),
                        a.kinds.clone(),
                    ))
                })
                .collect::<Result<_>>()?,
            moves,
            categories,
            default_forbidden: rules.default == Some(Fallback::Forbidden),
            bound: RefCell::default(),
        })
    }

    /// Whether `path` matches `glob_text`, compiled once per text.
    pub(crate) fn bound_match(&self, glob_text: &str, path: &str) -> bool {
        let mut bound = self.bound.borrow_mut();
        bound
            .entry(glob_text.to_owned())
            .or_insert_with(|| glob(glob_text).expect("validated when the rules were compiled"))
            .is_match(path)
    }

    /// `Some(value)` when `path` matches `pattern`; the value binds its capture, trying the
    /// path's segments left to right.
    fn bind(&self, pattern: &Pattern, path: &str) -> Option<Option<String>> {
        if !pattern.any.is_match(path) {
            return None;
        }
        if pattern.capture.is_none() {
            return Some(None);
        }
        path.split('/')
            .find(|value| self.bound_match(&pattern.bound_glob(&pattern.raw, Some(value)), path))
            .map(|value| Some(value.to_owned()))
    }

    /// The `modules:` id for `path`, when a rule matches it.
    pub fn module_of(&self, path: &str) -> Option<String> {
        self.modules.iter().find_map(|(pattern, id)| {
            let value = self.bind(pattern, path)?;
            Some(pattern.instance(id, value.as_deref()))
        })
    }

    /// The first matching layer, retaining the policy's lowest-first index.
    pub fn layer_for(&self, path: &str) -> Option<(usize, String)> {
        self.layers.iter().enumerate().find_map(|(index, layer)| {
            layer
                .is_match(path)
                .then(|| (index, layer.glob().glob().to_owned()))
        })
    }

    /// Constraint evidence for edits to these paths. Source order is policy precedence.
    /// Reachability needs a graph, so reachable forbids are retained conservatively.
    pub fn relevant(&self, doc: &Rules, paths: &[String]) -> Result<serde_json::Value> {
        use serde_json::json;
        for path in paths {
            if !crate::collect::is_relative_path(path, false)
                || path.contains(['*', '?', '[', ']', '{', '}', '<', '>'])
            {
                bail!(
                    "invalid rule path {path:?}: supply an exact path relative to the scanned root, without traversal or globs"
                );
            }
        }
        let mut paths = paths.to_vec();
        paths.sort();
        paths.dedup();
        let mut modules = Vec::new();
        for (order, (pattern, id)) in self.modules.iter().enumerate() {
            for (capture, matched) in self.relevant_instances(pattern, None, &paths)? {
                modules.push(json!({"id": pattern.instance(id, capture.as_deref()),
                    "path": pattern.bound_glob(&pattern.raw, capture.as_deref()),
                    "order": order, "matched_paths": matched}));
            }
        }
        // All layer boundaries matter to a possible new import, not just its edited end.
        let layers: Vec<_> = doc
            .layers
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let matched: Vec<_> = paths
                    .iter()
                    .filter(|p| self.layers[index].is_match(p))
                    .collect();
                json!({"index": index, "path": path, "matched_paths": matched})
            })
            .collect();
        let mut public = Vec::new();
        for (order, (pattern, entries, kinds)) in self.public.iter().enumerate() {
            let mut matches = self.relevant_instances(pattern, None, &paths)?;
            for entry in entries {
                for (capture, matched) in self.relevant_instances(pattern, Some(entry), &paths)? {
                    matches.entry(capture).or_default().extend(matched);
                }
            }
            for (capture, mut matched) in matches {
                matched.sort();
                matched.dedup();
                let entries: Vec<_> = entries
                    .iter()
                    .map(|entry| pattern.bound_glob(entry, capture.as_deref()))
                    .collect();
                public.push(
                    json!({"module": pattern.bound_glob(&pattern.raw, capture.as_deref()),
                    "entries": entries, "kinds": kinds, "order": order, "matched_paths": matched}),
                );
            }
        }
        let mut entrypoints = Vec::new();
        for (pattern, entry) in &self.entrypoints {
            for (capture, matched) in self.relevant_instances(pattern, Some(entry), &paths)? {
                entrypoints.push(json!({"module": pattern.bound_glob(&pattern.raw, capture.as_deref()),
                    "entry": pattern.instance(entry, capture.as_deref()), "matched_paths": matched}));
            }
        }
        let mut forbid = Vec::new();
        for (order, ((_, pattern, to, reachable, _), rule)) in
            self.forbid.iter().zip(&doc.forbid).enumerate()
        {
            let mut matches = self.relevant_instances(pattern, Some(to), &paths)?;
            if *reachable {
                matches.entry(None).or_default();
            }
            for (capture, matched) in matches {
                let mut value = serde_json::to_value(rule)?;
                value["from"] = json!(pattern.bound_glob(&pattern.raw, capture.as_deref()));
                value["to"] = json!(pattern.bound_glob(to, capture.as_deref()));
                value["order"] = json!(order);
                value["matched_paths"] = json!(matched);
                if *reachable {
                    value["selection"] =
                        json!("conservative: indirect reachability may cross the edited paths");
                }
                forbid.push(value);
            }
        }
        let mut allow = Vec::new();
        for (order, ((_, pattern, to, _), rule)) in self.allow.iter().zip(&doc.allow).enumerate() {
            for (capture, matched) in self.relevant_instances(pattern, Some(to), &paths)? {
                let mut value = serde_json::to_value(rule)?;
                value["from"] = json!(pattern.bound_glob(&pattern.raw, capture.as_deref()));
                value["to"] = json!(pattern.bound_glob(to, capture.as_deref()));
                value["order"] = json!(order);
                value["matched_paths"] = json!(matched);
                allow.push(value);
            }
        }
        let categories: BTreeMap<_, _> = doc
            .pkg_category
            .iter()
            .filter(|(name, _)| {
                let selector = format!("category:{name}");
                forbid.iter().chain(&allow).any(|r| r["to"] == selector)
            })
            .collect();
        let relevant_globs = |patterns: &[String]| -> Vec<String> {
            patterns
                .iter()
                .filter(|pattern| paths.iter().any(|p| self.bound_match(pattern, p)))
                .cloned()
                .collect()
        };
        Ok(json!({"version": doc.version, "paths": paths,
            "default": doc.default.unwrap_or(Fallback::Unplanned),
            "edge_weights": doc.edge_weights.unwrap_or_default(),
            "modules": modules, "layers": layers, "public": public,
            "entrypoints": entrypoints, "forbid": forbid, "allow": allow,
            "pkg-category": categories, "exclude": relevant_globs(&doc.exclude),
            "include": relevant_globs(&doc.include), "external_exposure": doc.external_exposure,
            "early-smells": doc.early_smells,
            "layer-reason": doc.layer_reason,
            "precedence": "forbid, entrypoints, allow, public, layers, same-module, public entry, default"}))
    }

    fn relevant_instances(
        &self,
        from: &Pattern,
        to: Option<&str>,
        paths: &[String],
    ) -> Result<BTreeMap<Option<String>, Vec<String>>> {
        let target = to
            .filter(|to| {
                !to.starts_with("pkg:")
                    && !to.starts_with("category:")
                    && !to.starts_with("global:")
            })
            .map(Pattern::new)
            .transpose()?;
        let mut matches: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for path in paths {
            if let Some(capture) = self.bind(from, path) {
                matches.entry(capture).or_default().push(path.clone());
            }
            if let Some(capture) = target.as_ref().and_then(|target| self.bind(target, path)) {
                let matched = matches.entry(capture).or_default();
                if !matched.contains(path) {
                    matched.push(path.clone());
                }
            }
        }
        Ok(matches)
    }

    /// Sets `verdict`, `rule` and `reach` on every edge.
    pub fn decide(&self, edges: &mut [Edge], module_of: &BTreeMap<String, String>) {
        self.decide_with_externals(edges, module_of, &BTreeMap::new());
    }

    pub fn decide_with_externals(
        &self,
        edges: &mut [Edge],
        module_of: &BTreeMap<String, String>,
        externals: &BTreeMap<String, String>,
    ) {
        let mut adjacency: HashMap<&str, Vec<(&str, EdgeKind)>> = HashMap::new();
        for e in edges.iter() {
            adjacency.entry(&e.from).or_default().push((&e.to, e.kind));
        }
        let mut reached: Reached = HashMap::new();
        let decisions: Vec<_> = edges
            .iter()
            .map(|e| self.verdict(e, module_of, externals, &adjacency, &mut reached))
            .collect();
        for (e, (verdict, rule, reach)) in edges.iter_mut().zip(decisions) {
            e.verdict = Some(verdict);
            e.rule = Some(rule);
            e.reach = reach;
        }
    }

    /// Per forbid rule, the files and targets on each side and the imports it forbids, by bound
    /// capture. Every rule sees every import, whichever rule `verdict` gives it to.
    pub fn forbid_slices(
        &self,
        files: &[&str],
        edges: &[&Edge],
        externals: &BTreeMap<String, String>,
    ) -> Vec<BTreeMap<Option<String>, SliceEvidence>> {
        let mut adjacency: HashMap<&str, Vec<(&str, EdgeKind)>> = HashMap::new();
        for e in edges {
            adjacency.entry(&e.from).or_default().push((&e.to, e.kind));
        }
        let mut reached: Reached = HashMap::new();
        let targets: Vec<&str> = files
            .iter()
            .copied()
            .chain(externals.keys().map(String::as_str))
            .collect();
        (0..self.forbid.len())
            .map(|i| {
                let (_, from, to, _, _) = &self.forbid[i];
                let mut slices: BTreeMap<Option<String>, SliceEvidence> = BTreeMap::new();
                for file in files {
                    if let Some(value) = self.bind(from, file) {
                        slices.entry(value).or_default().from.push(file.to_string());
                    }
                }
                for (value, slice) in &mut slices {
                    let to = from.bound_glob(to, value.as_deref());
                    slice.to = targets
                        .iter()
                        .filter(|t| self.matches_target(externals, &to, t))
                        .map(|t| t.to_string())
                        .collect();
                }
                for e in edges {
                    if let Some((value, _)) =
                        self.forbids(i, e, externals, &adjacency, &mut reached)
                    {
                        slices.entry(value).or_default().hits.push(Link {
                            from: e.from.clone(),
                            to: e.original_to.as_deref().unwrap_or(&e.to).to_owned(),
                        });
                    }
                }
                slices
            })
            .collect()
    }

    /// Per move rule, by bound capture: the files still matching `from` (first matching rule
    /// wins) with their destinations, and the files already below the bound `to`. A destination
    /// is the bound `to` plus the file's path below the literal leading folders of `from`.
    pub(crate) fn resolve_moves(
        &self,
        files: &[&str],
    ) -> Vec<BTreeMap<Option<String>, MoveEvidence>> {
        let mut out: Vec<BTreeMap<Option<String>, MoveEvidence>> =
            self.moves.iter().map(|_| BTreeMap::new()).collect();
        for file in files {
            let first = self
                .moves
                .iter()
                .enumerate()
                .find_map(|(i, (from, _, _))| Some((i, self.bind(from, file)?)));
            if let Some((i, value)) = first {
                let (from, to, _) = &self.moves[i];
                let bound = from.instance(&from.raw, value.as_deref());
                let segments: Vec<&str> = bound.split('/').collect();
                let literal = segments[..segments.len() - 1]
                    .iter()
                    .take_while(|s| !s.contains(['*', '?', '[', ']', '{', '}']))
                    .count();
                let below = file.split('/').skip(literal).collect::<Vec<_>>().join("/");
                out[i].entry(value.clone()).or_default().pending.push(Move {
                    file: file.to_string(),
                    to: from.instance(to, value.as_deref()) + &below,
                });
            }
            for (i, (_, _, arrived)) in self.moves.iter().enumerate() {
                if let Some(value) = self.bind(arrived, file) {
                    out[i].entry(value).or_default().arrived += 1;
                }
            }
        }
        out
    }

    /// Whether a `global:` target of `forbid` or `allow` names the host global `name`.
    pub fn names_global(&self, name: &str) -> bool {
        self.forbid
            .iter()
            .map(|f| &f.2)
            .chain(self.allow.iter().map(|a| &a.2))
            .filter_map(|to| to.strip_prefix("global:"))
            .any(|pattern| self.bound_match(pattern, name))
    }

    /// Whether `target` (a file, or an external node by its policy name, `pkg:<package>` or
    /// `global:<name>`) matches `selector`.
    fn matches_target(
        &self,
        externals: &BTreeMap<String, String>,
        selector: &str,
        target: &str,
    ) -> bool {
        match externals.get(target) {
            Some(name) => {
                let package = name.strip_prefix("pkg:");
                if let Some(pattern) = selector.strip_prefix("pkg:") {
                    package.is_some_and(|package| self.bound_match(pattern, package))
                } else if let Some(category) = selector.strip_prefix("category:") {
                    package.is_some_and(|package| {
                        self.categories
                            .get(category)
                            .is_some_and(|patterns| patterns.iter().any(|p| p.is_match(package)))
                    })
                } else if let Some(pattern) = selector.strip_prefix("global:") {
                    name.strip_prefix("global:")
                        .is_some_and(|global| self.bound_match(pattern, global))
                } else {
                    false
                }
            }
            None => {
                !selector.starts_with("pkg:")
                    && !selector.starts_with("category:")
                    && !selector.starts_with("global:")
                    && self.bound_match(selector, target)
            }
        }
    }

    /// `Some((capture, reach))` when forbid rule `i` matches `e`; `reach` is the import path
    /// for a reachable hit.
    fn forbids<'a>(
        &self,
        i: usize,
        e: &'a Edge,
        externals: &BTreeMap<String, String>,
        adjacency: &HashMap<&'a str, Vec<(&'a str, EdgeKind)>>,
        reached: &mut Reached<'a>,
    ) -> Option<(Option<String>, Option<Vec<String>>)> {
        let (_, from, to, reachable, kinds) = &self.forbid[i];
        if !includes_kind(kinds, e.kind) {
            return None;
        }
        let value = self.bind(from, &e.from)?;
        let to = from.bound_glob(to, value.as_deref());
        if self.matches_target(externals, &to, e.original_to.as_deref().unwrap_or(&e.to)) {
            return Some((value, None));
        }
        if !*reachable {
            return None;
        }
        let path = reached
            .entry((i, value.clone(), e.to.as_str()))
            .or_insert_with(|| {
                shortest_path(&e.to, adjacency, kinds, |f| {
                    self.matches_target(externals, &to, f)
                })
            })
            .clone()?;
        Some((value, Some(path)))
    }

    /// The first matching rule; strict entrypoint boundaries precede permissive allows.
    fn verdict<'a>(
        &self,
        e: &'a Edge,
        module_of: &BTreeMap<String, String>,
        externals: &BTreeMap<String, String>,
        adjacency: &HashMap<&'a str, Vec<(&'a str, EdgeKind)>>,
        reached: &mut Reached<'a>,
    ) -> (Verdict, String, Option<Vec<String>>) {
        use Verdict::*;
        let written = e.original_to.as_deref().unwrap_or(&e.to);
        let matches_target =
            |selector: &str, target: &str| self.matches_target(externals, selector, target);
        for i in 0..self.forbid.len() {
            if let Some((_, reach)) = self.forbids(i, e, externals, adjacency, reached) {
                return (Forbidden, self.forbid[i].0.clone(), reach);
            }
        }
        for (module, entry) in &self.entrypoints {
            if !externals.contains_key(written)
                && let Some(value) = self.bind(module, written)
            {
                let value = value.as_deref();
                let inside = self.bound_match(&module.bound_glob(&module.raw, value), &e.from);
                if !inside && written != module.instance(entry, value) {
                    return (Forbidden, "entrypoints".into(), None);
                }
            }
        }
        for (name, from, to, kinds) in &self.allow {
            if !includes_kind(kinds, e.kind) {
                continue;
            }
            if let Some(value) = self.bind(from, &e.from)
                && matches_target(&from.bound_glob(to, value.as_deref()), written)
            {
                return (Allowed, name.clone(), None);
            }
        }
        if externals.contains_key(written) {
            return if self.default_forbidden {
                (Forbidden, "default".into(), None)
            } else {
                (Unplanned, "none".into(), None)
            };
        }
        let entry_of = |module: &Pattern, entries: &[String], value: Option<&str>| {
            entries
                .iter()
                .any(|entry| self.bound_match(&module.bound_glob(entry, value), written))
        };
        for (module, entries, kinds) in &self.public {
            if !includes_kind(kinds, e.kind) {
                continue;
            }
            if let Some(value) = self.bind(module, written) {
                let value = value.as_deref();
                let inside = self.bound_match(&module.bound_glob(&module.raw, value), &e.from);
                if !inside && !entry_of(module, entries, value) {
                    return (Forbidden, "public".into(), None);
                }
            }
        }
        let layer = |path: &str| self.layers.iter().position(|l| l.is_match(path));
        if let (Some(importer), Some(target)) = (layer(&e.from), layer(written)) {
            let verdict = if target <= importer {
                Allowed
            } else {
                Forbidden
            };
            return (verdict, "layers".into(), None);
        }
        if module_of
            .get(&e.from)
            .is_some_and(|module| Some(module) == module_of.get(written))
        {
            return (Allowed, "same-module".into(), None);
        }
        for (module, entry) in &self.entrypoints {
            if let Some(value) = self.bind(module, written)
                && written == module.instance(entry, value.as_deref())
            {
                return (Allowed, "entrypoints".into(), None);
            }
        }
        for (module, entries, kinds) in &self.public {
            if !includes_kind(kinds, e.kind) {
                continue;
            }
            if let Some(value) = self.bind(module, written)
                && entry_of(module, entries, value.as_deref())
            {
                return (Allowed, "public".into(), None);
            }
        }
        if self.default_forbidden {
            (Forbidden, "default".into(), None)
        } else {
            (Unplanned, "none".into(), None)
        }
    }

    /// The entrypoint or `public` module covering `to`, with captures bound, and the entries an
    /// import into `to` should use instead. Explicit entrypoints take precedence.
    pub fn public_for(&self, to: &str) -> Option<(String, Vec<String>)> {
        self.boundary(to, Pattern::instance)
    }

    /// `public_for` as globs for `bound_match`: the bound capture matches literally.
    pub(crate) fn public_globs(&self, to: &str) -> Option<(String, Vec<String>)> {
        self.boundary(to, Pattern::bound_glob)
    }

    fn boundary(
        &self,
        to: &str,
        bound: fn(&Pattern, &str, Option<&str>) -> String,
    ) -> Option<(String, Vec<String>)> {
        let entrypoints = self
            .entrypoints
            .iter()
            .map(|(module, entry)| (module, std::slice::from_ref(entry)));
        let public = self
            .public
            .iter()
            .map(|(module, entries, _)| (module, entries.as_slice()));
        entrypoints.chain(public).find_map(|(module, entries)| {
            let value = self.bind(module, to)?;
            let value = value.as_deref();
            Some((
                bound(module, &module.raw, value),
                entries
                    .iter()
                    .map(|entry| bound(module, entry, value))
                    .collect(),
            ))
        })
    }

    /// The explicit entrypoint for `to`, with module captures bound to its path.
    pub fn entrypoints_for(&self, to: &str) -> Option<(String, Vec<String>)> {
        self.entrypoints.iter().find_map(|(module, entry)| {
            let value = self.bind(module, to)?;
            Some((
                module.instance(&module.raw, value.as_deref()),
                vec![module.instance(entry, value.as_deref())],
            ))
        })
    }
}

/// Breadth-first from `start` (inclusive) to the nearest file that `hit` accepts.
fn shortest_path(
    start: &str,
    adjacency: &HashMap<&str, Vec<(&str, EdgeKind)>>,
    kinds: &Option<Vec<EdgeKind>>,
    hit: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let mut parent: HashMap<&str, &str> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    parent.insert(start, start);
    while let Some(file) = queue.pop_front() {
        if hit(file) {
            let mut path = vec![file.to_owned()];
            let mut at = file;
            while at != start {
                at = parent[at];
                path.push(at.to_owned());
            }
            path.reverse();
            return Some(path);
        }
        for &(next, kind) in adjacency.get(file).into_iter().flatten() {
            if !includes_kind(kinds, kind) {
                continue;
            }
            if !parent.contains_key(next) {
                parent.insert(next, file);
                queue.push_back(next);
            }
        }
    }
    None
}
