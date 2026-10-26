//! Local, syntax-based application facts and service inference.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FactKind {
    Database,
    Cache,
    Queue,
    Http,
    Route,
    Publish,
    Subscribe,
}

#[derive(Debug, Clone)]
pub(crate) struct Fact {
    pub kind: FactKind,
    pub package: String,
    /// Original URL for HTTP calls, path for routes, name for topics.
    pub operation: Option<String>,
    pub method: Option<String>,
    pub line: usize,
    pub dynamic: bool,
}

#[derive(Debug, Default)]
pub(crate) struct AppFacts {
    pub facts: Vec<Fact>,
    pub entrypoint: bool,
    pub incomplete: bool,
    pub imports: Vec<String>,
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::collect::{self, excludes, native};
use crate::model::Language;
use crate::services::{
    self, Call, Confidence, Manifest, Protocol, Report, Resource, ResourceKind, SourceLocation,
    Verdict,
};

/// Infer local application evidence without executing the scanned application's code.
pub fn infer(root: &Path, manifest: Option<Manifest>) -> Result<Report> {
    let root = root.canonicalize().context("resolving service root")?;
    if !root.is_dir() {
        bail!("service inference requires a directory");
    }
    let root_id = root
        .file_name()
        .and_then(|name| name.to_str())
        .context("service root must have a UTF-8 directory name")?
        .to_owned();
    let policy = excludes::Policy::new(&root, Language::Typescript, excludes::Options::default())?;
    let files = policy.select(&inventory(&root, &policy)?)?;
    if files
        .iter()
        .any(|file| !collect::is_relative_path(file, false))
    {
        bail!(
            "application source paths must be supported relative paths without control characters"
        );
    }
    let eligible: BTreeSet<_> = files.iter().cloned().collect();
    let mut boundaries = BTreeSet::new();
    let mut package_dirs = BTreeSet::new();
    let mut startups = BTreeSet::new();
    let mut relocated_root = false;
    let mut orchestrated_root = false;
    let mut modules = Vec::new();
    for file in &files {
        let path = Path::new(file);
        let dir = parent(file);
        match path.file_name().and_then(|name| name.to_str()) {
            Some("Dockerfile") => {
                boundaries.insert(dir);
            }
            Some("package.json" | "project.json") => {
                let text = fs::read_to_string(root.join(file))?;
                let value: serde_json::Value = serde_json::from_str(&text)
                    .with_context(|| format!("parsing service marker {file}"))?;
                package_dirs.insert(dir.clone());
                let scripts = &value["scripts"];
                if dir.is_empty() {
                    orchestrated_root |= scripts.as_object().is_some_and(|scripts| {
                        scripts.values().any(|script| {
                            script.as_str().is_some_and(|script| {
                                script
                                    .split_whitespace()
                                    .any(|token| matches!(token, "turbo" | "nx" | "concurrently"))
                            })
                        })
                    });
                }
                let start = scripts["start"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| {
                        scripts["serve"]
                            .as_str()
                            .filter(|script| runtime_script(script))
                    });
                if let Some(script) = start {
                    let target = script_boundary(&root, &dir, script);
                    relocated_root |= dir.is_empty() && !target.is_empty();
                    boundaries.insert(target.clone());
                    if !dir.is_empty() || !script.contains(['&', '|', ';']) {
                        startups.insert(target);
                    }
                }
                let targets = &value["targets"];
                if targets.get("serve").is_some()
                    || targets.get("start").is_some()
                    || value["projectType"] == "application"
                        && targets.as_object().is_some_and(|t| !t.is_empty())
                {
                    boundaries.insert(dir.clone());
                    startups.insert(dir);
                }
            }
            Some("go.mod") => {
                let text = fs::read_to_string(root.join(file))?;
                if let Some(module) = text
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("module "))
                {
                    modules.push((dir, module.trim().trim_matches('"').to_owned()));
                }
            }
            _ => {}
        }
    }
    let ts = native::typescript::collect(&root, ".", &policy, false)?;
    let mut packages: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    for import in ts.raw.imports.iter().flatten() {
        if let Some(package) = &import.external {
            packages
                .entry(&import.from)
                .or_default()
                .insert(import.specifier.clone(), package.clone());
        }
    }
    let mut parsed = BTreeMap::new();
    let mut relations: BTreeSet<(String, String)> = ts
        .raw
        .edges
        .iter()
        .filter(|edge| {
            edge.kind.is_runtime() && eligible.contains(&edge.from) && eligible.contains(&edge.to)
        })
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect();
    for file in files.iter().filter(|file| is_source(file)) {
        let source = fs::read_to_string(root.join(file))
            .with_context(|| format!("reading application source {file}"))?;
        let mut facts = if file.ends_with(".go") {
            native::go::app_facts(file, &source)?
        } else if file.ends_with(".py") {
            native::python::app_facts(file, &source)?
        } else {
            native::typescript::app_facts(
                file,
                &source,
                packages.get(file.as_str()).unwrap_or(&BTreeMap::new()),
            )?
        };
        for fact in &mut facts.facts {
            if fact
                .operation
                .as_ref()
                .is_some_and(|value| value.chars().any(char::is_control))
            {
                fact.operation = None;
                facts.incomplete = true;
            }
        }
        if facts.entrypoint && !file.ends_with(".py") {
            let dir = if file.ends_with(".go") {
                parent(file)
            } else {
                startup_boundary(file, &package_dirs, &boundaries)
            };
            boundaries.insert(dir.clone());
            startups.insert(dir);
        }
        parsed.insert(file.clone(), facts);
    }
    // Root source clients can belong to an image even when its startup is indirect.
    let root_facts = parsed.iter().any(|(file, facts)| {
        !facts.facts.is_empty()
            && !package_dirs
                .iter()
                .any(|dir| !dir.is_empty() && Path::new(file).starts_with(dir))
            && !boundaries
                .iter()
                .any(|dir| !dir.is_empty() && Path::new(file).starts_with(dir))
    });
    if boundaries.iter().any(|dir| !dir.is_empty())
        && !startups.contains("")
        && (relocated_root || orchestrated_root || !root_facts)
    {
        boundaries.remove("");
    }
    for (file, facts) in &parsed {
        if !file.ends_with(".go") {
            continue;
        }
        for imported in &facts.imports {
            for (dir, module) in &modules {
                let suffix = if imported == module {
                    Some("")
                } else {
                    imported.strip_prefix(&format!("{module}/"))
                };
                if let Some(suffix) = suffix {
                    let target = join(dir, suffix);
                    for other in parsed
                        .keys()
                        .filter(|other| other.ends_with(".go") && parent(other) == target)
                    {
                        relations.insert((file.clone(), other.clone()));
                    }
                }
            }
        }
    }
    let declared = manifest.is_some();
    let mut manifest = manifest.unwrap_or(Manifest {
        version: 1,
        services: Vec::new(),
        edges: Vec::new(),
        forbid: Vec::new(),
    });
    let mut overrides = BTreeSet::new();
    for id in &manifest.services {
        let dir = root.join(id);
        if dir.is_dir() {
            if !dir.canonicalize()?.starts_with(&root) {
                bail!("service boundary {id:?} must stay inside the root");
            }
            overrides.insert(id.clone());
        } else if id == &root_id {
            overrides.insert(String::new());
        }
    }
    if !overrides.is_empty() {
        boundaries = overrides;
    }
    if boundaries.contains("") && boundaries.contains(&root_id) {
        bail!("root service ID {root_id:?} collides with a nested service directory");
    }
    for dir in &boundaries {
        let id = if dir.is_empty() { &root_id } else { dir };
        if !manifest.services.contains(id) {
            manifest.services.push(id.clone());
        }
    }
    let local_python = python_relations(&parsed, &boundaries, &mut relations);
    for (file, packages) in local_python {
        if let Some(facts) = parsed.get_mut(&file) {
            facts.facts.retain(|fact| {
                !packages.contains(fact.package.split('.').next().unwrap_or(&fact.package))
            });
        }
    }
    let owner = |file: &str| -> Option<String> {
        boundaries
            .iter()
            .filter(|dir| dir.is_empty() || Path::new(file).starts_with(dir))
            .max_by_key(|dir| dir.len())
            .map(|dir| {
                if dir.is_empty() {
                    root_id.clone()
                } else {
                    dir.clone()
                }
            })
    };
    let mut attributed = Vec::new();
    let mut unattributed = 0;
    let mut incomplete = 0;
    for (file, facts) in &parsed {
        incomplete += usize::from(facts.incomplete);
        let direct = owner(file);
        let owners: BTreeSet<_> = direct
            .iter()
            .cloned()
            .chain(
                relations
                    .iter()
                    .filter(|(_, to)| to == file)
                    .filter_map(|(from, _)| direct.is_none().then(|| owner(from)).flatten()),
            )
            .collect();
        if owners.is_empty() {
            unattributed += facts.facts.len();
        }
        for service in owners {
            for fact in &facts.facts {
                attributed.push((
                    service.clone(),
                    file.clone(),
                    fact.clone(),
                    if direct.is_some() {
                        Confidence::High
                    } else {
                        Confidence::Medium
                    },
                ));
            }
        }
    }
    let mut resources = Vec::new();
    let mut ambiguous = 0;
    for (service, file, fact, confidence) in &attributed {
        let mut matched = false;
        if matches!(fact.kind, FactKind::Http | FactKind::Publish) && fact.operation.is_some() {
            let mut candidates: BTreeMap<String, (String, Confidence)> = BTreeMap::new();
            for (target, _, endpoint, target_confidence) in &attributed {
                if target == service {
                    continue;
                }
                let result = if fact.kind == FactKind::Http && endpoint.kind == FactKind::Route {
                    match_http(fact, endpoint, target)
                } else if fact.kind == FactKind::Publish
                    && endpoint.kind == FactKind::Subscribe
                    && fact.operation == endpoint.operation
                {
                    Some((
                        fact.operation.clone().unwrap(),
                        if fact.dynamic || endpoint.dynamic {
                            Confidence::Medium
                        } else {
                            Confidence::High
                        },
                    ))
                } else {
                    None
                };
                if let Some((operation, found)) = result {
                    let found = found.min(*confidence).min(*target_confidence);
                    candidates
                        .entry(target.clone())
                        .and_modify(|previous| {
                            if found > previous.1 {
                                *previous = (operation.clone(), found);
                            }
                        })
                        .or_insert((operation, found));
                }
            }
            if candidates.len() == 1 {
                let (to, (operation, confidence)) = candidates.into_iter().next().unwrap();
                manifest.edges.push(Call {
                    from: service.clone(),
                    to,
                    kind: if fact.kind == FactKind::Http {
                        Protocol::Http
                    } else {
                        Protocol::Queue
                    },
                    confidence,
                    source: SourceLocation {
                        file: file.clone(),
                        line: fact.line,
                    },
                    operation: Some(operation),
                });
                matched = true;
            } else if candidates.len() > 1 {
                ambiguous += 1;
            }
        }
        let kind = match fact.kind {
            FactKind::Database => ResourceKind::Database,
            FactKind::Cache => ResourceKind::Cache,
            FactKind::Queue => ResourceKind::Queue,
            FactKind::Http if !matched => ResourceKind::Http,
            _ => continue,
        };
        resources.push(Resource {
            service: service.clone(),
            kind,
            package: fact.package.clone(),
            operation: if kind == ResourceKind::Http {
                fact.operation.as_deref().map(http_path)
            } else {
                fact.operation.clone()
            },
            confidence: *confidence,
            source: SourceLocation {
                file: file.clone(),
                line: fact.line,
            },
            verdict: Verdict::Allowed,
            rule: None,
        });
    }
    let mut seen = BTreeSet::new();
    manifest.edges.retain(|edge| seen.insert(edge.clone()));
    let mut report = services::analyze(manifest, &root_id)?;
    report.source.collector = if declared {
        "inferred+declarative"
    } else {
        "inferred"
    }
    .into();
    report.caveats = vec![
        "Inference is syntax-based and incomplete; configuration, indirect wiring and dynamic values may hide clients and calls.".into(),
        "Only literal routes, URLs and topics are matched; shared clients use direct runtime import evidence.".into(),
        "Service boundaries use deployment, application metadata and runtime startup evidence; a process with several runtime roles remains one service.".into(),
        "A service cycle is a review signal, not proof of deadlock or deploy-order failure; queue calls may be asynchronous.".into(),
    ];
    if declared {
        report.caveats.push("Declared source locations and confidence are supplied by the manifest and are not verified.".into());
    }
    if unattributed > 0 {
        report.caveats.push(format!(
            "{unattributed} application facts are unattributed to any service."
        ));
    }
    if ambiguous > 0 {
        report.caveats.push(format!(
            "{ambiguous} calls match multiple services; no service edge was inferred for them."
        ));
    }
    if incomplete > 0 {
        report.caveats.push(format!(
            "{incomplete} source files have incomplete syntax facts."
        ));
    }
    services::attach_resources(&mut report, resources)?;
    Ok(report)
}

fn parent(file: &str) -> String {
    Path::new(file)
        .parent()
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .replace('\\', "/")
}
fn join(dir: &str, suffix: &str) -> String {
    if dir.is_empty() {
        suffix.into()
    } else if suffix.is_empty() {
        dir.into()
    } else {
        format!("{dir}/{suffix}")
    }
}
fn is_source(file: &str) -> bool {
    Path::new(file)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "go" | "py"
            )
        })
}
fn inventory(root: &Path, policy: &excludes::Policy) -> Result<Vec<String>> {
    let defaults = collect::globset(policy.patterns())?;
    let mut pending = vec![PathBuf::new()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(root.join(&dir))
            .with_context(|| format!("reading service directory {}", root.join(&dir).display()))?
        {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_str().is_some_and(|name| name.starts_with('.')) {
                continue;
            }
            let path = dir.join(name);
            let relative = path.to_string_lossy().replace('\\', "/");
            let kind = entry.file_type()?;
            if kind.is_dir() {
                if !defaults.is_match(format!("{relative}/__boundrail.ts"))
                    && !matches!(relative.as_str(), "target" | "venv")
                {
                    pending.push(path);
                }
            } else if kind.is_file()
                && (is_source(&relative)
                    || matches!(
                        entry.file_name().to_str(),
                        Some("Dockerfile" | "package.json" | "project.json" | "go.mod")
                    ))
            {
                files.push(relative);
            }
        }
    }
    files.sort();
    Ok(files)
}
fn script_boundary(root: &Path, dir: &str, script: &str) -> String {
    if !dir.is_empty() || script.contains(['&', '|', ';']) {
        return dir.into();
    }
    for token in script
        .split_whitespace()
        .map(|t| t.trim_matches(['\'', '"']).trim_start_matches("./"))
    {
        let Some(source) = token
            .strip_prefix("build/")
            .or_else(|| token.strip_prefix("dist/"))
        else {
            continue;
        };
        if let Some(top) = source
            .split('/')
            .next()
            .filter(|part| !matches!(*part, "src" | "node_modules" | "." | ".."))
            && source.contains('/')
            && root.join(top).is_dir()
        {
            return top.into();
        }
    }
    dir.into()
}
fn startup_boundary(
    file: &str,
    packages: &BTreeSet<String>,
    boundaries: &BTreeSet<String>,
) -> String {
    if let Some(dir) = packages
        .iter()
        .filter(|dir| !dir.is_empty() && Path::new(file).starts_with(dir))
        .max_by_key(|dir| dir.len())
    {
        return dir.clone();
    }
    if let Some(dir) = boundaries
        .iter()
        .filter(|dir| !dir.is_empty() && Path::new(file).starts_with(dir))
        .max_by_key(|dir| dir.len())
    {
        return dir.clone();
    }
    file.split('/')
        .next()
        .filter(|top| *top != "src" && file.contains('/'))
        .unwrap_or("")
        .into()
}
fn match_http(call: &Fact, route: &Fact, target: &str) -> Option<(String, Confidence)> {
    let method = call.method.as_deref();
    if route
        .method
        .as_deref()
        .is_some_and(|route_method| Some(route_method) != method)
    {
        return None;
    }
    let raw = call.operation.as_deref()?;
    if raw.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https")
    }) {
        return None;
    }
    let (authority, path) = http_parts(raw);
    if let Some(authority) = authority {
        let host = authority.rsplit('@').next()?.split(':').next()?;
        // A remote API with an identical path is not evidence for a local service call.
        if host != target && host != target.rsplit('/').next()? {
            return None;
        }
    } else if !path.starts_with('/') {
        return None;
    }
    let endpoint = route.operation.as_deref()?;
    let exact = path.trim_end_matches('/') == endpoint.trim_end_matches('/');
    let parameter = |segment: &str| {
        segment.starts_with(':')
            || segment.starts_with('{') && segment.ends_with('}')
            || segment.starts_with("${") && segment.ends_with('}')
            || !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
    };
    let left: Vec<_> = path.trim_matches('/').split('/').collect();
    let right: Vec<_> = endpoint.trim_matches('/').split('/').collect();
    if !exact
        && (left.len() != right.len()
            || !left
                .iter()
                .zip(&right)
                .all(|(a, b)| a == b || parameter(a) || parameter(b)))
    {
        return None;
    }
    Some((
        format!("{} {endpoint}", method.unwrap_or("ANY")),
        if exact && !call.dynamic && !route.dynamic && route.method.is_some() {
            Confidence::High
        } else {
            Confidence::Medium
        },
    ))
}

fn runtime_script(script: &str) -> bool {
    let tokens: Vec<_> = script.split_whitespace().collect();
    let framework = tokens
        .iter()
        .any(|token| matches!(*token, "vite" | "next" | "nest" | "nuxt" | "docusaurus"))
        && !tokens.contains(&"build");
    let executor = tokens.iter().any(|token| {
        matches!(
            *token,
            "node" | "tsx" | "ts-node" | "ts-node-dev" | "bun" | "deno"
        )
    });
    let source = tokens.iter().any(|token| {
        (token.starts_with("src/") || token.contains("/src/"))
            && Path::new(token.trim_matches(['\'', '"']))
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "ts" | "tsx" | "js" | "mjs" | "cjs"))
    });
    framework || executor && source
}

fn http_parts(raw: &str) -> (Option<&str>, &str) {
    let raw = raw.split(['?', '#']).next().unwrap_or("");
    let authority = raw
        .split_once("://")
        .map(|(_, rest)| rest)
        .or_else(|| raw.strip_prefix("//"));
    match authority {
        Some(rest) => {
            let offset = rest.find('/').unwrap_or(rest.len());
            (
                Some(&rest[..offset]),
                if offset == rest.len() {
                    "/"
                } else {
                    &rest[offset..]
                },
            )
        }
        None => (None, raw),
    }
}
fn http_path(raw: &str) -> String {
    http_parts(raw).1.to_owned()
}

fn python_relations(
    parsed: &BTreeMap<String, AppFacts>,
    boundaries: &BTreeSet<String>,
    relations: &mut BTreeSet<(String, String)>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut local = BTreeMap::<String, BTreeSet<String>>::new();
    for (file, facts) in parsed.iter().filter(|(file, _)| file.ends_with(".py")) {
        let service = boundaries
            .iter()
            .filter(|dir| dir.is_empty() || Path::new(file).starts_with(dir))
            .max_by_key(|dir| dir.len())
            .map(String::as_str)
            .unwrap_or("");
        for module in &facts.imports {
            let relative = module.chars().take_while(|ch| *ch == '.').count();
            let suffix = module[relative..].replace('.', "/");
            let candidates = if relative > 0 {
                let mut base = parent(file);
                let package_depth = base.split('/').filter(|part| !part.is_empty()).count()
                    + usize::from(parsed.contains_key("__init__.py"));
                if relative > package_depth {
                    continue;
                }
                for _ in 1..relative {
                    base = parent(&base);
                }
                vec![join(&base, &suffix)]
            } else {
                [
                    service.to_owned(),
                    join(service, "src"),
                    String::new(),
                    "src".into(),
                ]
                .iter()
                .map(|base| join(base, &suffix))
                .collect()
            };
            for candidate in candidates {
                let targets: Vec<_> = [join(&candidate, "__init__.py"), format!("{candidate}.py")]
                    .into_iter()
                    .filter(|target| parsed.contains_key(target))
                    .collect();
                if !targets.is_empty() && relative == 0 {
                    local
                        .entry(file.clone())
                        .or_default()
                        .insert(module.split('.').next().unwrap_or(module).to_owned());
                }
                if targets.len() == 1 {
                    relations.insert((file.clone(), targets.into_iter().next().unwrap()));
                    break;
                }
                if targets.len() > 1 {
                    break;
                }
            }
        }
    }
    local
}
