//! layout.yaml: parsing, glob compilation and edge verdicts.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{Config, Edge, RulesRef, Verdict};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rules {
    pub version: u32,
    #[serde(default)]
    pub modules: Vec<ModuleRule>,
    #[serde(default)]
    pub layers: Vec<String>,
    #[serde(default)]
    pub public: Vec<PublicRule>,
    #[serde(default)]
    pub forbid: Vec<Forbid>,
    #[serde(default)]
    pub allow: Vec<Allow>,
    /// The verdict for an import no other step decides; absent means `unplanned`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Fallback>,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Forbid {
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub reachable: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Allow {
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    pub reason: Option<String>,
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
/// a document from a newer archgraph stays readable.
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
    })
}

/// Every key the rules format knows, as one serialized rule of each kind.
fn known_keys() -> serde_json::Value {
    serde_json::to_value(Rules {
        modules: vec![ModuleRule::default()],
        public: vec![PublicRule::default()],
        forbid: vec![Forbid::default()],
        allow: vec![Allow::default()],
        default: Some(Fallback::Forbidden),
        ..Rules::default()
    })
    .expect("rules serialize")
}

/// The first key of `given` that `known` lacks, with where it sits (`forbid[0]`); array items
/// are compared with `known`'s first item.
fn unknown_key(
    given: &serde_json::Value,
    known: &serde_json::Value,
    at: &str,
) -> Option<(String, String)> {
    use serde_json::Value::{Array, Object};
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

pub struct Compiled {
    modules: Vec<(Pattern, String)>,
    layers: Vec<GlobMatcher>,
    public: Vec<(Pattern, Vec<String>)>,
    /// (name, from, to, reachable); `to` may use only `from`'s capture, bound to the importer.
    forbid: Vec<(String, Pattern, String, bool)>,
    allow: Vec<(String, Pattern, String)>,
    /// Step 7 gives forbidden (rule `default`) instead of unplanned.
    default_forbidden: bool,
    /// Globs with a capture bound, compiled on first use.
    bound: RefCell<HashMap<String, GlobMatcher>>,
}

impl Compiled {
    pub fn new(rules: &Rules) -> Result<Compiled> {
        if rules.version != 1 {
            bail!("unsupported rules version {}; expected 1", rules.version);
        }
        // Ids, reasons, globs and module ids are printed into terminals, CI logs and job
        // summaries, where a line break could forge a workflow command.
        let strings = rules
            .modules
            .iter()
            .flat_map(|m| [&m.id, &m.path])
            .chain(&rules.layers)
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
            }));
        for s in strings {
            if s.chars().any(char::is_control) {
                bail!("{s:?} holds a control character; rules strings must be one plain line");
            }
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
            public.push((module, p.entries.clone()));
        }
        let name = |id: &Option<String>, kind: &str, i: usize| {
            id.clone().unwrap_or_else(|| format!("{kind}[{i}]"))
        };
        let pair = |from: &str, to: &str| -> Result<Pattern> {
            let from = Pattern::new(from)?;
            from.check_template(to)?;
            glob(&from.instance(to, Some("x")))?;
            Ok(from)
        };
        let mut layers = Vec::new();
        for layer in &rules.layers {
            if !captures(layer)?.is_empty() {
                bail!("layer `{layer}`: layers bind no capture; write `*` for one segment");
            }
            layers.push(glob(layer)?);
        }
        Ok(Compiled {
            modules,
            layers,
            public,
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
                    ))
                })
                .collect::<Result<_>>()?,
            allow: rules
                .allow
                .iter()
                .enumerate()
                .map(|(i, a)| Ok((name(&a.id, "allow", i), pair(&a.from, &a.to)?, a.to.clone())))
                .collect::<Result<_>>()?,
            default_forbidden: rules.default == Some(Fallback::Forbidden),
            bound: RefCell::default(),
        })
    }

    fn bound_match(&self, glob_text: &str, path: &str) -> bool {
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

    /// Sets `verdict`, `rule` and `reach` on every edge.
    pub fn decide(&self, edges: &mut [Edge], module_of: &BTreeMap<String, String>) {
        let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
        for e in edges.iter() {
            adjacency.entry(&e.from).or_default().push(&e.to);
        }
        let mut reached: Reached = HashMap::new();
        let decisions: Vec<_> = edges
            .iter()
            .map(|e| self.verdict(e, module_of, &adjacency, &mut reached))
            .collect();
        for (e, (verdict, rule, reach)) in edges.iter_mut().zip(decisions) {
            e.verdict = Some(verdict);
            e.rule = Some(rule);
            e.reach = reach;
        }
    }

    /// The first matching step of the seven in docs/plan.md.
    fn verdict<'a>(
        &self,
        e: &'a Edge,
        module_of: &BTreeMap<String, String>,
        adjacency: &HashMap<&'a str, Vec<&'a str>>,
        reached: &mut Reached<'a>,
    ) -> (Verdict, String, Option<Vec<String>>) {
        use Verdict::*;
        for (i, (name, from, to, reachable)) in self.forbid.iter().enumerate() {
            let Some(value) = self.bind(from, &e.from) else {
                continue;
            };
            let to = from.bound_glob(to, value.as_deref());
            if self.bound_match(&to, &e.to) {
                return (Forbidden, name.clone(), None);
            }
            if *reachable {
                let path = reached.entry((i, value, e.to.as_str())).or_insert_with(|| {
                    shortest_path(&e.to, adjacency, |f| self.bound_match(&to, f))
                });
                if let Some(path) = path {
                    return (Forbidden, name.clone(), Some(path.clone()));
                }
            }
        }
        for (name, from, to) in &self.allow {
            if let Some(value) = self.bind(from, &e.from)
                && self.bound_match(&from.bound_glob(to, value.as_deref()), &e.to)
            {
                return (Allowed, name.clone(), None);
            }
        }
        let entry_of = |module: &Pattern, entries: &[String], value: Option<&str>| {
            entries
                .iter()
                .any(|entry| self.bound_match(&module.bound_glob(entry, value), &e.to))
        };
        for (module, entries) in &self.public {
            if let Some(value) = self.bind(module, &e.to) {
                let value = value.as_deref();
                let inside = self.bound_match(&module.bound_glob(&module.raw, value), &e.from);
                if !inside && !entry_of(module, entries, value) {
                    return (Forbidden, "public".into(), None);
                }
            }
        }
        let layer = |path: &str| self.layers.iter().position(|l| l.is_match(path));
        if let (Some(importer), Some(target)) = (layer(&e.from), layer(&e.to)) {
            let verdict = if target <= importer {
                Allowed
            } else {
                Forbidden
            };
            return (verdict, "layers".into(), None);
        }
        if module_of[&e.from] == module_of[&e.to] {
            return (Allowed, "same-module".into(), None);
        }
        for (module, entries) in &self.public {
            if let Some(value) = self.bind(module, &e.to)
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

    /// The public entries that an import into `to` should use instead, when a `public` rule
    /// covers `to`.
    pub fn entries_for(&self, to: &str) -> Option<Vec<String>> {
        self.public.iter().find_map(|(module, entries)| {
            let value = self.bind(module, to)?;
            Some(
                entries
                    .iter()
                    .map(|entry| module.instance(entry, value.as_deref()))
                    .collect(),
            )
        })
    }
}

/// Breadth-first from `start` (inclusive) to the nearest file that `hit` accepts.
fn shortest_path(
    start: &str,
    adjacency: &HashMap<&str, Vec<&str>>,
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
        for &next in adjacency.get(file).into_iter().flatten() {
            if !parent.contains_key(next) {
                parent.insert(next, file);
                queue.push_back(next);
            }
        }
    }
    None
}
