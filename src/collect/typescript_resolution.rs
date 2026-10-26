//! Complete import-statement accounting and bounded source/config resolution.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{TS_EXTENSIONS, imports};
use crate::model::{EdgeKind, ImportReason, ImportRecord};

#[derive(Clone)]
struct Package {
    dir: PathBuf,
    json: Value,
    maps: PackageMaps,
}
#[derive(Clone, Default)]
struct Config {
    base: Option<PathBuf>,
    paths_base: Option<PathBuf>,
    paths: BTreeMap<String, Vec<String>>,
}
#[derive(Clone)]
enum Resolution {
    Local(String),
    External(String),
    Unresolved(ImportReason),
}
struct Resolver {
    root: PathBuf,
    repo: PathBuf,
    packages: BTreeMap<PathBuf, Option<Package>>,
    workspaces: BTreeMap<String, Package>,
    configs: BTreeMap<PathBuf, Option<Config>>,
}

/// Preserve unknown occurrence counts when recorded collector files are unavailable.
pub fn enrich(
    json_bytes: &[u8],
    sources: &mut imports::Sources,
) -> Result<(Vec<u8>, Option<Vec<ImportRecord>>)> {
    let mut json: Value =
        serde_json::from_slice(json_bytes).context("reading TypeScript collector JSON")?;
    let mut resolver = Resolver::new(&sources.root)?;
    let mut modules: BTreeMap<String, Value> = json["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|module| Some((module["source"].as_str()?.to_owned(), module.clone())))
        .collect();
    let original_files: BTreeSet<_> = modules.keys().cloned().collect();
    let original_cruised = json["summary"]["totalCruised"]
        .as_u64()
        .unwrap_or(original_files.len() as u64);
    let mut queue: VecDeque<_> = modules
        .keys()
        .filter(|file| {
            code_file(file)
                && !file.split('/').any(|part| part == "node_modules")
                && super::is_relative_path(file, false)
        })
        .cloned()
        .collect();
    let mut visited = BTreeSet::new();
    let mut ledger = Vec::new();
    let mut complete = true;
    while let Some(file) = queue.pop_front() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let mut module = modules
            .remove(&file)
            .unwrap_or_else(|| json!({"source":file,"dependencies":[]}));
        if module["coreModule"].as_bool() == Some(true)
            || module["couldNotResolve"].as_bool() == Some(true)
        {
            modules.insert(file, module);
            continue;
        }
        let dependencies = module["dependencies"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let source = super::is_relative_path(&file, false)
            .then(|| sources.get(&file))
            .flatten();
        if source.is_none() {
            complete = false;
        }
        let statements = source.as_ref().map_or(&[][..], |s| &s.statements);
        let mut enriched = dependencies.clone();
        for statement in statements {
            let Some(specifier) = &statement.specifier else {
                ledger.push(ImportRecord {
                    from: file.clone(),
                    specifier: "<computed>".into(),
                    line: Some(statement.line),
                    kind: statement.kind,
                    targets: Vec::new(),
                    external: None,
                    unresolved: Some(ImportReason::Dynamic),
                    ..ImportRecord::default()
                });
                continue;
            };
            let known = dependencies
                .iter()
                .find(|dependency| dependency["module"].as_str() == Some(specifier));
            let resolution = resolver.resolve(&file, specifier, statement.require, known, 0);
            let (targets, external, unresolved) = match &resolution {
                Resolution::Local(target) => {
                    let mut dependency = known
                        .cloned()
                        .unwrap_or_else(|| json!({"module":specifier}));
                    dependency["resolved"] = target.clone().into();
                    dependency["couldNotResolve"] = false.into();
                    dependency["coreModule"] = false.into();
                    dependency["_boundrailLine"] = statement.line.into();
                    dependency["_boundrailRequire"] = statement.require.into();
                    if dependency.get("dependencyTypes").is_none() {
                        dependency["dependencyTypes"] = dependency_types(statement.kind);
                    }
                    enriched.retain(|dependency| {
                        dependency["module"].as_str() != Some(specifier)
                            || dependency.get("_boundrailRequire").is_some()
                    });
                    enriched.push(dependency);
                    if code_file(target) && !visited.contains(target) {
                        queue.push_back(target.clone());
                    }
                    (vec![target.clone()], None, None)
                }
                Resolution::External(package) => (Vec::new(), Some(package.clone()), None),
                Resolution::Unresolved(reason) => {
                    for dependency in &mut enriched {
                        if dependency["module"].as_str() == Some(specifier) {
                            dependency["couldNotResolve"] = true.into();
                        }
                    }
                    (Vec::new(), None, Some(*reason))
                }
            };
            ledger.push(ImportRecord {
                from: file.clone(),
                specifier: specifier.clone(),
                line: Some(statement.line),
                kind: statement.kind,
                targets,
                external,
                unresolved,
                ..ImportRecord::default()
            });
        }
        // Source-less recordings can still benefit from literal alias recovery without claiming a ledger.
        if source.is_none() {
            for dependency in &mut enriched {
                let Some(specifier) = dependency["module"].as_str().map(str::to_owned) else {
                    continue;
                };
                if let Resolution::Local(target) =
                    resolver.resolve(&file, &specifier, false, Some(dependency), 0)
                {
                    dependency["resolved"] = target.into();
                    dependency["couldNotResolve"] = false.into();
                    dependency["coreModule"] = false.into();
                }
            }
        }
        module["dependencies"] = enriched.into();
        modules.insert(file, module);
    }
    json["modules"] = modules.into_values().collect::<Vec<_>>().into();
    if let Some(summary) = json["summary"].as_object_mut() {
        summary.insert(
            "totalCruised".into(),
            (original_cruised
                + visited
                    .iter()
                    .filter(|file| !original_files.contains(*file))
                    .count() as u64)
                .into(),
        );
    }
    ledger.sort_by(|a, b| (&a.from, a.line, &a.specifier).cmp(&(&b.from, b.line, &b.specifier)));
    Ok((serde_json::to_vec(&json)?, complete.then_some(ledger)))
}

fn dependency_types(kind: EdgeKind) -> Value {
    match kind {
        EdgeKind::Type | EdgeKind::TypeReexport => json!(["local", "import", "type-only"]),
        EdgeKind::Dynamic => json!(["local", "dynamic-import"]),
        EdgeKind::Reexport => json!(["local", "export"]),
        EdgeKind::Static => json!(["local", "import"]),
    }
}
fn code_file(file: &str) -> bool {
    file.rsplit_once('.')
        .is_some_and(|(_, extension)| TS_EXTENSIONS.contains(&extension))
}
fn normalized(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            part => result.push(part.as_os_str()),
        }
    }
    Some(result)
}
pub(crate) fn package_name(specifier: &str) -> Option<String> {
    if specifier.starts_with(['.', '/', '#', '~']) || specifier.is_empty() {
        return None;
    }
    let mut segments = specifier.split('/');
    let first = segments.next()?;
    if first.starts_with('@') {
        Some(format!("{first}/{}", segments.next()?))
    } else {
        Some(first.into())
    }
}

impl Resolver {
    fn new(root: &Path) -> Result<Self> {
        let root = root.canonicalize().context("resolving TypeScript root")?;
        let repo = crate::git::toplevel(&root)
            .ok()
            .filter(|repo| root.starts_with(repo))
            .unwrap_or_else(|| root.clone());
        let mut resolver = Self {
            root,
            repo,
            packages: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            configs: BTreeMap::new(),
        };
        resolver.discover_workspaces()?;
        Ok(resolver)
    }
    fn read_json(&self, path: &Path, jsonc: bool) -> Option<Value> {
        let path = path.canonicalize().ok()?;
        if !path.starts_with(&self.repo) {
            return None;
        }
        let bytes = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&if jsonc { strip_jsonc(&bytes) } else { bytes }).ok()
    }
    fn package(&mut self, dir: &Path) -> Option<Package> {
        if let Some(package) = self.packages.get(dir) {
            return package.clone();
        }
        let package = dir
            .join("package.json")
            .canonicalize()
            .ok()
            .filter(|path| path.starts_with(&self.repo))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| {
                Some(Package {
                    dir: dir.to_owned(),
                    json: serde_json::from_str(&text).ok()?,
                    maps: serde_json::from_str(&text).ok()?,
                })
            });
        self.packages.insert(dir.to_owned(), package.clone());
        package
    }
    fn nearest_package(&mut self, file: &Path) -> Option<Package> {
        let mut directory = file.parent()?;
        while directory.starts_with(&self.repo) {
            if let Some(package) = self.package(directory) {
                return Some(package);
            }
            directory = directory.parent()?;
        }
        None
    }
    fn discover_workspaces(&mut self) -> Result<()> {
        let mut roots = Vec::new();
        let mut ancestor = Some(self.root.clone());
        while let Some(directory) = ancestor.filter(|directory| directory.starts_with(&self.repo)) {
            let mut patterns = Vec::new();
            if let Some(package) = self.package(&directory) {
                let list = package.json["workspaces"]
                    .as_array()
                    .or_else(|| package.json["workspaces"]["packages"].as_array());
                patterns.extend(
                    list.into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
                if let Some(name) = package.json["name"].as_str() {
                    self.workspaces.insert(name.into(), package.clone());
                }
            }
            if let Some(path) = directory
                .join("pnpm-workspace.yaml")
                .canonicalize()
                .ok()
                .filter(|path| path.starts_with(&self.repo))
                && let Ok(bytes) = std::fs::read(path)
                && let Ok(value) = serde_yaml_ng::from_slice::<Value>(&bytes)
            {
                patterns.extend(
                    value["packages"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
            }
            if !patterns.is_empty() {
                roots.push((directory.clone(), patterns));
            }
            ancestor = directory.parent().map(Path::to_owned);
        }
        for (base, patterns) in roots {
            let includes = super::globset(
                &patterns
                    .iter()
                    .filter(|pattern| !pattern.starts_with('!'))
                    .cloned()
                    .collect::<Vec<_>>(),
            )?;
            let excludes = super::globset(
                &patterns
                    .iter()
                    .filter_map(|pattern| pattern.strip_prefix('!').map(str::to_owned))
                    .collect::<Vec<_>>(),
            )?;
            let mut pending = vec![base.clone()];
            while let Some(directory) = pending.pop() {
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                        continue;
                    }
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with('.')
                        || matches!(
                            name.as_ref(),
                            "node_modules" | "vendor" | "target" | "dist" | "build"
                        )
                    {
                        continue;
                    }
                    let directory = entry.path();
                    let relative = directory
                        .strip_prefix(&base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    if includes.is_match(&relative)
                        && !excludes.is_match(&relative)
                        && let Some(package) = self.package(&directory)
                        && let Some(name) = package.json["name"].as_str()
                    {
                        self.workspaces.insert(name.into(), package.clone());
                    }
                    pending.push(directory);
                }
            }
        }
        Ok(())
    }
    fn resolve(
        &mut self,
        from: &str,
        specifier: &str,
        require: bool,
        known: Option<&Value>,
        depth: usize,
    ) -> Resolution {
        if depth > 10 {
            return Resolution::Unresolved(ImportReason::Alias);
        }
        let importer = self.root.join(from);
        let mut alias = false;
        if !specifier.starts_with(['.', '/'])
            && !native(specifier)
            && known.is_none_or(|dependency| dependency["coreModule"].as_bool() != Some(true))
            && let Some(config) = self.config_for(&importer)
        {
            for (pattern, targets) in matches(&config.paths, specifier) {
                let Some(capture) = capture(pattern, specifier) else {
                    continue;
                };
                let base = config
                    .base
                    .as_ref()
                    .or(config.paths_base.as_ref())
                    .unwrap_or(&self.repo)
                    .clone();
                for target in targets {
                    if let Some(resolution) =
                        self.candidate(&base.join(target.replace('*', &capture)), specifier)
                    {
                        return resolution;
                    }
                }
                // TypeScript falls back to baseUrl and package resolution when no target exists.
                alias = true;
                break;
            }
            if let Some(base) = config.base
                && let Some(resolution) = self.candidate(&base.join(specifier), specifier)
            {
                return resolution;
            }
        }
        match self.resolve_unaliased(from, specifier, require, known, depth) {
            Resolution::Unresolved(_) if alias => Resolution::Unresolved(ImportReason::Alias),
            resolution => resolution,
        }
    }
    fn resolve_unaliased(
        &mut self,
        from: &str,
        specifier: &str,
        require: bool,
        known: Option<&Value>,
        depth: usize,
    ) -> Resolution {
        let importer = self.root.join(from);
        if let Some(known) =
            known.filter(|dependency| dependency["couldNotResolve"].as_bool() != Some(true))
        {
            if known["coreModule"].as_bool() == Some(true) {
                return Resolution::External(format!(
                    "node:{}",
                    specifier.strip_prefix("node:").unwrap_or(specifier)
                ));
            }
            if let Some(resolved) = known["resolved"].as_str() {
                if resolved.split('/').any(|part| part == "node_modules")
                    && let Some(name) = package_name(specifier)
                {
                    return Resolution::External(name);
                }
                if super::is_relative_path(resolved, false) && code_file(resolved) {
                    let path = self.root.join(resolved);
                    return if path.exists() {
                        self.classify(&path, specifier)
                    } else {
                        Resolution::Local(resolved.into())
                    };
                }
                if let Some(path) = normalized(&self.root.join(resolved))
                    && path.is_file()
                {
                    return self.classify(&path, specifier);
                }
            }
        }
        if native(specifier) {
            return Resolution::External(format!(
                "node:{}",
                specifier.strip_prefix("node:").unwrap_or(specifier)
            ));
        }
        if specifier.starts_with("node:") {
            return Resolution::Unresolved(ImportReason::Native);
        }
        if specifier.starts_with('.') {
            return self
                .candidate(
                    &importer.parent().unwrap_or(&self.root).join(specifier),
                    specifier,
                )
                .unwrap_or(Resolution::Unresolved(ImportReason::Missing));
        }
        if specifier.starts_with('/') {
            return Resolution::Unresolved(ImportReason::Alias);
        }
        let nearest = self.nearest_package(&importer);
        if specifier.starts_with('#') {
            if let Some(package) = nearest
                && let Some(targets) = package
                    .maps
                    .imports
                    .as_ref()
                    .and_then(|map| mapped(map, specifier, require))
            {
                for target in targets {
                    if target.starts_with('.') {
                        if let Some(resolution) =
                            self.candidate(&package.dir.join(target), specifier)
                        {
                            return resolution;
                        }
                    } else {
                        let resolution = self.resolve(from, &target, require, None, depth + 1);
                        if !matches!(resolution, Resolution::Unresolved(_)) {
                            return resolution;
                        }
                    }
                }
            }
            return Resolution::Unresolved(ImportReason::Alias);
        }
        let Some(name) = package_name(specifier) else {
            return Resolution::Unresolved(ImportReason::Alias);
        };
        let local = nearest
            .as_ref()
            .filter(|package| package.json["name"].as_str() == Some(&name))
            .cloned()
            .or_else(|| self.workspaces.get(&name).cloned());
        if let Some(package) = local {
            let subpath = specifier
                .strip_prefix(&name)
                .unwrap_or_default()
                .trim_start_matches('/');
            let key = if subpath.is_empty() {
                ".".into()
            } else {
                format!("./{subpath}")
            };
            if package.json.get("exports").is_some() {
                if let Some(targets) = package
                    .maps
                    .exports
                    .as_ref()
                    .and_then(|map| mapped(map, &key, require))
                {
                    for target in targets {
                        if let Some(resolution) =
                            self.package_candidate(&package, &target, specifier)
                        {
                            return resolution;
                        }
                    }
                }
                return Resolution::Unresolved(ImportReason::Alias);
            }
            if !subpath.is_empty() {
                if let Some(resolution) = self.candidate(&package.dir.join(subpath), specifier) {
                    return resolution;
                }
            } else {
                for field in ["main", "module"] {
                    if let Some(target) = package.json[field].as_str()
                        && let Some(resolution) =
                            self.package_candidate(&package, target, specifier)
                    {
                        return resolution;
                    }
                }
                if let Some(resolution) = self.candidate(&package.dir.join("index"), specifier) {
                    return resolution;
                }
                if let Some(resolution) = self.candidate(&package.dir.join("src/index"), specifier)
                {
                    return resolution;
                }
            }
            return Resolution::Unresolved(ImportReason::Missing);
        }
        let repo = self.repo.clone();
        let root_package = self.package(&repo);
        let scan_root = self.root.clone();
        let scan_package = self.package(&scan_root);
        if nearest
            .iter()
            .chain(root_package.iter())
            .chain(scan_package.iter())
            .any(|package| {
                [
                    "dependencies",
                    "devDependencies",
                    "peerDependencies",
                    "optionalDependencies",
                ]
                .iter()
                .any(|field| package.json[*field].get(&name).is_some())
            })
        {
            return Resolution::External(name);
        }
        let mut directory = importer.parent();
        while let Some(parent) = directory.filter(|parent| parent.starts_with(&self.repo)) {
            if self
                .read_json(
                    &parent.join("node_modules").join(&name).join("package.json"),
                    false,
                )
                .is_some()
            {
                return Resolution::External(name);
            }
            directory = parent.parent();
        }
        Resolution::Unresolved(
            if specifier.starts_with("@/") || specifier.starts_with("~/") {
                ImportReason::Alias
            } else if specifier.ends_with(".node") || specifier.contains("://") {
                ImportReason::Native
            } else {
                ImportReason::Missing
            },
        )
    }
    fn package_candidate(
        &mut self,
        package: &Package,
        target: &str,
        specifier: &str,
    ) -> Option<Resolution> {
        if let Some(resolution) = self.candidate(&package.dir.join(target), specifier) {
            return Some(resolution);
        }
        let target = target.trim_start_matches("./");
        let (directory, rest) = target.split_once('/')?;
        if !super::depcruise::OUTPUT_DIRS.contains(&directory) {
            return None;
        }
        for prefix in ["src/", ""] {
            if let Some(resolution) =
                self.candidate(&package.dir.join(format!("{prefix}{rest}")), specifier)
            {
                return Some(resolution);
            }
        }
        None
    }
    fn candidate(&mut self, candidate: &Path, specifier: &str) -> Option<Resolution> {
        let candidate = normalized(candidate)?;
        if !candidate.starts_with(&self.repo) {
            return None;
        }
        let mut candidates = Vec::new();
        let extension = candidate
            .extension()
            .and_then(|extension| extension.to_str());
        if matches!(extension, Some("js" | "jsx" | "mjs" | "cjs")) {
            for extension in ["ts", "tsx", "d.ts"] {
                candidates.push(candidate.with_extension(extension));
            }
        }
        candidates.push(candidate.clone());
        // `./user.service` names `user.service.ts`: append, never replace, the last extension.
        for extension in ["ts", "tsx", "js", "jsx", "mjs", "cjs", "d.ts"] {
            let mut path = candidate.clone().into_os_string();
            path.push(".");
            path.push(extension);
            candidates.push(path.into());
        }
        if candidate.is_dir() {
            if let Some(package) = self.package(&candidate) {
                for field in ["main", "module"] {
                    if let Some(target) = package.json[field].as_str() {
                        let target = candidate.join(target);
                        if target.is_file() {
                            candidates.push(target);
                        }
                    }
                }
            }
            for extension in TS_EXTENSIONS {
                candidates.push(candidate.join(format!("index.{extension}")));
            }
        }
        candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .map(|candidate| self.classify(&candidate, specifier))
    }
    fn classify(&mut self, path: &Path, _specifier: &str) -> Resolution {
        let Ok(path) = path.canonicalize() else {
            return Resolution::Unresolved(ImportReason::Missing);
        };
        if !path.starts_with(&self.repo) {
            return Resolution::Unresolved(ImportReason::Alias);
        }
        if let Ok(relative) = path.strip_prefix(&self.root) {
            return Resolution::Local(relative.to_string_lossy().replace('\\', "/"));
        }
        if let Some(package) = self.nearest_package(&path)
            && let Some(name) = package.json["name"].as_str()
        {
            return Resolution::External(name.into());
        }
        Resolution::Unresolved(ImportReason::Alias)
    }
    fn config_for(&mut self, importer: &Path) -> Option<Config> {
        let mut directory = importer.parent()?;
        while directory.starts_with(&self.repo) {
            let path = directory.join("tsconfig.json");
            if path.is_file() {
                return self.load_config(&path, &mut BTreeSet::new(), 0);
            }
            directory = directory.parent()?;
        }
        None
    }
    fn load_config(
        &mut self,
        path: &Path,
        seen: &mut BTreeSet<PathBuf>,
        depth: usize,
    ) -> Option<Config> {
        let path = path.canonicalize().ok()?;
        if depth >= 10 || !path.starts_with(&self.repo) || !seen.insert(path.clone()) {
            return None;
        }
        if let Some(config) = self.configs.get(&path) {
            seen.remove(&path);
            return config.clone();
        }
        let json = self.read_json(&path, true)?;
        let directory = path.parent()?;
        let mut config = Config::default();
        let bases: Vec<_> = json["extends"]
            .as_str()
            .into_iter()
            .chain(
                json["extends"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            )
            .collect();
        for base in bases {
            let mut base_path = directory.join(base);
            if !base_path.is_file() && base.starts_with('.') {
                base_path.set_extension("json");
            }
            if !base.starts_with('.') && !Path::new(base).is_absolute() {
                let mut ancestor = Some(directory);
                while let Some(parent) = ancestor.filter(|parent| parent.starts_with(&self.repo)) {
                    let candidate = parent.join("node_modules").join(base);
                    if candidate.is_file() {
                        base_path = candidate;
                        break;
                    }
                    if candidate.with_extension("json").is_file() {
                        base_path = candidate.with_extension("json");
                        break;
                    }
                    ancestor = parent.parent();
                }
            }
            if let Some(base) = self.load_config(&base_path, seen, depth + 1) {
                if base.base.is_some() {
                    config.base = base.base;
                }
                if base.paths_base.is_some() {
                    config.paths_base = base.paths_base;
                    config.paths = base.paths;
                }
            }
        }
        if let Some(base) = json["compilerOptions"]["baseUrl"].as_str() {
            config.base = normalized(&directory.join(base));
        }
        if let Some(paths) = json["compilerOptions"]["paths"].as_object() {
            config.paths = paths
                .iter()
                .map(|(pattern, values)| {
                    (
                        pattern.clone(),
                        values
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                    )
                })
                .collect();
            config.paths_base = Some(config.base.clone().unwrap_or_else(|| directory.to_owned()));
        }
        seen.remove(&path);
        self.configs.insert(path, Some(config.clone()));
        Some(config)
    }
}

fn capture(pattern: &str, specifier: &str) -> Option<String> {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => specifier
            .strip_prefix(prefix)?
            .strip_suffix(suffix)
            .map(str::to_owned),
        None => (pattern == specifier).then(String::new),
    }
}
fn matches<'a>(
    paths: &'a BTreeMap<String, Vec<String>>,
    specifier: &str,
) -> Vec<(&'a str, &'a Vec<String>)> {
    let mut matching: Vec<_> = paths
        .iter()
        .filter(|(pattern, _)| capture(pattern, specifier).is_some())
        .map(|(pattern, values)| (pattern.as_str(), values))
        .collect();
    matching.sort_by_key(|(pattern, _)| {
        (
            pattern.contains('*'),
            std::cmp::Reverse(pattern.split('*').next().unwrap_or_default().len()),
            std::cmp::Reverse(pattern.len()),
        )
    });
    matching
}
#[derive(Clone, Default, Deserialize)]
struct PackageMaps {
    exports: Option<Target>,
    imports: Option<Target>,
}
#[derive(Clone)]
enum Target {
    Text(String),
    List(Vec<Target>),
    Map(Vec<(String, Target)>),
    Blocked,
}
impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Target;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a package target")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Target, E> {
                Ok(Target::Text(value.into()))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Target, E> {
                Ok(Target::Text(value))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Target, E> {
                Ok(Target::Blocked)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> std::result::Result<Target, E> {
                Ok(Target::Blocked)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<Target, E> {
                Ok(Target::Blocked)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> std::result::Result<Target, E> {
                Ok(Target::Blocked)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Target, E> {
                Ok(Target::Blocked)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Target, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = access.next_element()? {
                    values.push(value);
                }
                Ok(Target::List(values))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Target, A::Error> {
                let mut values = Vec::new();
                while let Some(entry) = access.next_entry()? {
                    values.push(entry);
                }
                Ok(Target::Map(values))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
fn mapped(value: &Target, key: &str, require: bool) -> Option<Vec<String>> {
    if let Target::Map(object) = value
        && object
            .iter()
            .any(|(key, _)| key.starts_with('.') || key.starts_with('#'))
    {
        let patterns: BTreeMap<_, _> = object
            .iter()
            .map(|(key, _)| (key.clone(), Vec::new()))
            .collect();
        let (pattern, _) = matches(&patterns, key).into_iter().next()?;
        let capture = capture(pattern, key)?;
        let target = &object.iter().find(|(key, _)| key == pattern)?.1;
        return conditions(target, require).map(|targets| {
            targets
                .into_iter()
                .map(|target| target.replace('*', &capture))
                .collect()
        });
    }
    if key == "." {
        conditions(value, require)
    } else {
        None
    }
}
fn conditions(value: &Target, require: bool) -> Option<Vec<String>> {
    match value {
        Target::Text(target) => Some(vec![target.clone()]),
        Target::Blocked => Some(Vec::new()),
        Target::List(values) => values.iter().find_map(|value| conditions(value, require)),
        Target::Map(object) => object
            .iter()
            .filter(|(condition, _)| {
                condition == "default"
                    || condition == "node"
                    || condition == if require { "require" } else { "import" }
            })
            .find_map(|(_, target)| conditions(target, require)),
    }
}
fn native(specifier: &str) -> bool {
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    let node_only = matches!(name, "test" | "test/reporters" | "sea" | "sqlite");
    if node_only {
        return specifier.starts_with("node:");
    }
    matches!(
        name,
        "assert"
            | "assert/strict"
            | "async_hooks"
            | "buffer"
            | "child_process"
            | "cluster"
            | "console"
            | "constants"
            | "crypto"
            | "dgram"
            | "diagnostics_channel"
            | "dns"
            | "dns/promises"
            | "domain"
            | "events"
            | "fs"
            | "fs/promises"
            | "http"
            | "http2"
            | "https"
            | "inspector"
            | "inspector/promises"
            | "module"
            | "net"
            | "os"
            | "path"
            | "path/posix"
            | "path/win32"
            | "perf_hooks"
            | "process"
            | "punycode"
            | "querystring"
            | "readline"
            | "readline/promises"
            | "repl"
            | "stream"
            | "stream/consumers"
            | "stream/promises"
            | "stream/web"
            | "string_decoder"
            | "sys"
            | "timers"
            | "timers/promises"
            | "tls"
            | "trace_events"
            | "tty"
            | "url"
            | "util"
            | "util/types"
            | "v8"
            | "vm"
            | "wasi"
            | "worker_threads"
            | "zlib"
    )
}

/// JSONC comments/trailing commas, preserving quoted strings and line structure.
fn strip_jsonc(source: &str) -> String {
    let chars: Vec<_> = source.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut quoted = false;
    while i < chars.len() {
        let c = chars[i];
        if quoted {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                i += 1;
                out.push(chars[i]);
            } else if c == '"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            quoted = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        } else if c == ','
            && chars
                .iter()
                .skip(i + 1)
                .find(|c| !c.is_whitespace())
                .is_some_and(|c| matches!(c, ']' | '}'))
        {
            i += 1;
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }
    // Comments can occur between a trailing comma and its closing delimiter.
    let mut clean = String::new();
    let chars: Vec<_> = out.chars().collect();
    let mut quoted = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if quoted {
            clean.push(c);
            if c == '\\' && i + 1 < chars.len() {
                i += 1;
                clean.push(chars[i]);
            } else if c == '"' {
                quoted = false;
            }
        } else if c == '"' {
            quoted = true;
            clean.push(c);
        } else if c != ','
            || !chars
                .iter()
                .skip(i + 1)
                .find(|c| !c.is_whitespace())
                .is_some_and(|c| matches!(c, ']' | '}'))
        {
            clean.push(c);
        }
        i += 1;
    }
    clean
}
