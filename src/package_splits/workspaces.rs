//! Static package/project boundaries; never runs workspace tools or plugins.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use globset::GlobSet;
use serde_json::Value;

use super::{ExistingPackage, ExistingProject, WorkspaceInventory};

struct Workspace {
    root: PathBuf,
    include: GlobSet,
    exclude: GlobSet,
}

pub fn discover(root: &Path) -> Result<WorkspaceInventory> {
    let root = root
        .canonicalize()
        .context("resolving workspace inspection root")?;
    let repo = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "rev-parse",
            "--show-toplevel",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
                .canonicalize()
                .ok()
        })
        .filter(|repo| root.starts_with(repo))
        .unwrap_or_else(|| root.clone());
    let mut result = WorkspaceInventory {
        checked: true,
        packages: Vec::new(),
        projects: Vec::new(),
        limitations: Vec::new(),
        enclosing_package: None,
    };
    let mut workspaces = Vec::new();
    let mut explicit_projects = Vec::new();
    for ancestor in root.ancestors().take_while(|path| path.starts_with(&repo)) {
        let mut patterns = None;
        if let Some(package) = read(ancestor, "package.json", false)? {
            if result.enclosing_package.is_none() {
                result.enclosing_package = package
                    .get("name")
                    .map(|name| name.as_str().context("package.json name must be a string"))
                    .transpose()?
                    .map(str::to_owned);
            }
            if let Some(value) = package.get("workspaces") {
                let value = value
                    .as_array()
                    .or_else(|| value.get("packages").and_then(Value::as_array))
                    .context(
                        "package.json workspaces must be an array or an object containing packages",
                    )?;
                patterns = Some(strings(value, "package.json workspaces")?);
            }
        }
        if let Some(pnpm) = read(ancestor, "pnpm-workspace.yaml", true)? {
            patterns
                .get_or_insert_default()
                .extend(match pnpm.get("packages") {
                    None => Vec::new(),
                    Some(value) => strings(
                        value.as_array().context("pnpm packages must be an array")?,
                        "pnpm packages",
                    )?,
                });
        }
        if let Some(mut patterns) = patterns {
            for pattern in &patterns {
                safe_relative(pattern.strip_prefix('!').unwrap_or(pattern))?;
            }
            for pattern in &mut patterns {
                let (prefix, path) = pattern
                    .strip_prefix('!')
                    .map_or(("", pattern.as_str()), |path| ("!", path));
                *pattern = format!("{prefix}{}", path.trim_start_matches("./"));
            }
            workspaces.push(Workspace {
                root: ancestor.to_owned(),
                include: crate::collect::globset(
                    &patterns
                        .iter()
                        .filter(|p| !p.starts_with('!'))
                        .cloned()
                        .collect::<Vec<_>>(),
                )?,
                exclude: crate::collect::globset(
                    &patterns
                        .iter()
                        .filter_map(|p| p.strip_prefix('!').map(str::to_owned))
                        .collect::<Vec<_>>(),
                )?,
            });
        }
        if let Some(nx) = read(ancestor, "nx.json", false)? {
            if let Some(plugins) = nx.get("plugins") {
                let plugins = plugins
                    .as_array()
                    .context("nx.json plugins must be an array")?;
                if !plugins.is_empty() {
                    result.limitations.push("Nx plugin-inferred projects were not executed; explicit projects are listed only".into());
                }
            }
            if nx.get("projects").is_some() {
                result.limitations.push("nx.json projects is not treated as a project-root map; use project.json or workspace.json".into());
            }
        }
        if let Some(workspace) = read(ancestor, "workspace.json", false)?
            && let Some(projects) = workspace.get("projects")
        {
            for (name, value) in projects
                .as_object()
                .context("workspace.json projects must be an object")?
            {
                let path = value
                    .as_str()
                    .or_else(|| value.get("root").and_then(Value::as_str))
                    .context("workspace.json project must have a static root")?;
                safe_relative(path)?;
                let path: PathBuf = ancestor.join(path).components().collect();
                if path.starts_with(&root) {
                    if safe_directory(&root, &path)? {
                        explicit_projects.push((relative(&root, &path), Some(name.clone())));
                    } else {
                        result.limitations.push(
                            "A workspace.json project root is missing or uses a symlink".into(),
                        );
                    }
                }
            }
        }
    }
    let mut packages = BTreeMap::new();
    let mut projects: BTreeMap<_, _> = explicit_projects.into_iter().collect();
    let mut pending = vec![(root.clone(), 0)];
    let mut visited = 0;
    while let Some((directory, depth)) = pending.pop() {
        let path = relative(&root, &directory);
        if let Some(package) = read(&directory, "package.json", false)? {
            if directory != root && package.get("workspaces").is_some() {
                result.limitations.push(
                    "Nested workspace declarations were not expanded; inspect their package boundaries separately".into(),
                );
            }
            let name = package
                .get("name")
                .map(|name| name.as_str().context("package.json name must be a string"))
                .transpose()?
                .map(str::to_owned);
            let declared = workspaces.iter().any(|workspace| {
                let Ok(relative) = directory.strip_prefix(&workspace.root) else {
                    return false;
                };
                let relative = relative.to_string_lossy().replace('\\', "/");
                workspace.include.is_match(&relative) && !workspace.exclude.is_match(&relative)
            });
            if path == "." || declared || (workspaces.is_empty() && name.is_some()) {
                packages.insert(path.clone(), name.clone());
            } else if name.is_some() {
                projects.insert(path.clone(), name.clone());
                result.limitations.push(
                    "Named manifests outside declared workspace membership are existing projects, not proven workspace package dependencies".into(),
                );
            }
            if let Some(nx) = package.get("nx") {
                if !nx.is_object() {
                    bail!("package.json nx metadata must be an object");
                }
                projects.insert(
                    path.clone(),
                    nx.get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(name),
                );
            }
        }
        if let Some(project) = read(&directory, "project.json", false)? {
            let name = project
                .get("name")
                .map(|name| name.as_str().context("project.json name must be a string"))
                .transpose()?
                .map(str::to_owned);
            projects.insert(path.clone(), name);
        }
        if directory != root && directory.join("pnpm-workspace.yaml").exists() {
            result.limitations.push(
                "Nested workspace declarations were not expanded; inspect their package boundaries separately".into(),
            );
        }
        if directory != root && directory.join("nx.json").exists() {
            result.limitations.push(
                "Nested Nx workspaces were not expanded; explicit project.json files are listed only".into(),
            );
        }
        for entry in fs::read_dir(&directory).context("reading workspace directory")? {
            visited += 1;
            if visited > 100_000 {
                result
                    .limitations
                    .push("Workspace inspection stopped at 100000 directory entries".into());
                pending.clear();
                break;
            }
            let entry = entry.context("reading workspace directory entry")?;
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
            let kind = entry.file_type().context("reading workspace entry type")?;
            if kind.is_symlink() {
                result
                    .limitations
                    .push("Symlinked workspace paths were not inspected".into());
            } else if kind.is_dir() {
                if depth == 32 {
                    result
                        .limitations
                        .push("Workspace inspection stopped at depth 32".into());
                } else {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
    }
    result.packages = packages
        .into_iter()
        .map(|(root, name)| ExistingPackage { root, name })
        .collect();
    result.projects = projects
        .into_iter()
        .map(|(root, name)| ExistingProject { root, name })
        .collect();
    result.limitations.sort();
    result.limitations.dedup();
    result.checked = result.limitations.is_empty();
    Ok(result)
}

fn read(directory: &Path, name: &str, yaml: bool) -> Result<Option<Value>> {
    let path = directory.join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {name}")),
    };
    if metadata.file_type().is_symlink() {
        bail!("workspace metadata {name} must not be a symlink");
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {name}"))?;
    let value: Value = if yaml {
        serde_yaml_ng::from_slice(&bytes).with_context(|| format!("parsing {name}"))?
    } else {
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {name}"))?
    };
    if !value.is_object() {
        bail!("workspace metadata {name} must be an object");
    }
    Ok(Some(value))
}

fn strings(values: &[Value], source: &str) -> Result<Vec<String>> {
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("{source} entries must be strings"))
        })
        .collect()
}

fn safe_relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        bail!("workspace roots and globs must be relative and must not escape their workspace");
    }
    Ok(())
}

fn safe_directory(root: &Path, path: &Path) -> Result<bool> {
    let mut current = root.to_owned();
    for part in path.strip_prefix(root)?.components() {
        current.push(part);
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            return Ok(false);
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn relative(root: &Path, path: &Path) -> String {
    let path = path
        .strip_prefix(root)
        .expect("inventory roots stay inside scan root");
    if path.as_os_str().is_empty() {
        ".".into()
    } else {
        path.to_string_lossy().replace('\\', "/")
    }
}
