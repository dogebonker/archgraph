//! Source-only Elixir AST collector: the target project is never compiled or evaluated.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use super::{RawGraph, excludes::Policy};
use crate::model::{Collector, Edge, ImportRecord};

/// Parse `.ex` and `.exs` with a trusted helper in an isolated working directory.
pub fn run(root: &Path, elixir: &str, policy: &Policy) -> Result<Vec<u8>> {
    let root = fs::canonicalize(root)?;
    let elixir = super::resolve_executable("elixir", elixir)?;
    let mut files = super::native::source_files(&root, &["ex", "exs"])?;
    let selected = policy.select(&files)?;
    let kept: HashSet<_> = selected.iter().collect();
    let excluded: Vec<_> = files
        .iter()
        .filter(|file| !kept.contains(file))
        .cloned()
        .collect();
    files = selected;
    ensure!(!files.is_empty(), "no Elixir source files after excludes");
    let temp = super::TempDir::new("elixir")?;
    let dir = temp.path();
    fs::write(dir.join("elixir.exs"), include_str!("elixir.exs"))?;
    let mut command = Command::new(&elixir);
    command.args(["elixir.exs"]).arg(&root).current_dir(dir);
    sanitize(&mut command);
    let out =
        super::run_with_input(&mut command, files.join("\0").into_bytes()).with_context(|| {
            format!(
                "running {}: install Elixir 1.14 or later, or pass --elixir",
                elixir.display()
            )
        })?;
    if !out.status.success() {
        bail!(
            "Elixir helper failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    for line in String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| line.starts_with("warning:"))
    {
        eprintln!("{line}");
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("reading Elixir helper JSON")?;
    value
        .as_object_mut()
        .context("Elixir helper JSON must be an object")?
        .insert("excluded_files".into(), serde_json::to_value(excluded)?);
    Ok(serde_json::to_vec(&value)?)
}

/// Remove runtime startup flags and project code paths for collection and version probes.
pub fn sanitize(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("MIX_")
            || name.starts_with("ELIXIR_")
            || matches!(
                name.as_ref(),
                "ERL_AFLAGS"
                    | "ERL_FLAGS"
                    | "ERL_ZFLAGS"
                    | "ERL_LIBS"
                    | "ERL_PATH"
                    | "ERL_INETRC"
                    | "ERL_RC"
            )
        {
            command.env_remove(key);
        }
    }
}

#[derive(Deserialize)]
struct Dump {
    elixir_version: String,
    files: Vec<String>,
    edges: Vec<Edge>,
    imports: Vec<ImportRecord>,
    #[serde(default)]
    excluded_files: Vec<String>,
}

pub fn parse(bytes: &[u8]) -> Result<RawGraph> {
    let dump: Dump = serde_json::from_slice(bytes).context("reading Elixir helper JSON")?;
    let mut files = HashSet::new();
    for file in &dump.files {
        ensure!(
            super::is_relative_path(file, false),
            "Elixir helper path must be root-relative: {file:?}"
        );
        ensure!(
            files.insert(file.as_str()),
            "duplicate Elixir helper file: {file}"
        );
    }
    for edge in &dump.edges {
        ensure!(
            super::is_relative_path(&edge.from, false) && super::is_relative_path(&edge.to, false),
            "Elixir helper edge paths must be root-relative"
        );
        ensure!(
            files.contains(edge.from.as_str()) && files.contains(edge.to.as_str()),
            "Elixir helper edge references an unknown file"
        );
        ensure!(
            edge.line.is_some_and(|line| line > 0),
            "Elixir helper edge requires a positive source line"
        );
    }
    for import in &dump.imports {
        ensure!(
            super::is_relative_path(&import.from, false) && files.contains(import.from.as_str()),
            "Elixir helper import references an unknown file"
        );
        ensure!(
            import.line.is_some_and(|line| line > 0),
            "Elixir helper import requires a positive source line"
        );
        ensure!(
            !import.specifier.is_empty(),
            "Elixir helper import requires a specifier"
        );
        for target in &import.targets {
            ensure!(
                super::is_relative_path(target, false) && files.contains(target.as_str()),
                "Elixir helper import target must reference a root-relative known file"
            );
        }
        let outcomes = usize::from(!import.targets.is_empty())
            + usize::from(import.external.is_some())
            + usize::from(import.unresolved.is_some());
        ensure!(
            outcomes == 1,
            "Elixir helper import requires exactly one resolution outcome"
        );
        ensure!(
            import
                .external
                .as_ref()
                .is_none_or(|name| !name.is_empty() && !name.chars().any(char::is_control)),
            "Elixir helper external package name is invalid"
        );
    }
    for excluded in &dump.excluded_files {
        ensure!(
            super::is_relative_path(excluded, false),
            "Elixir helper excluded path must be root-relative"
        );
        ensure!(
            files.insert(excluded.as_str()),
            "duplicate Elixir helper excluded file: {excluded}"
        );
    }
    let unresolved = dump
        .imports
        .iter()
        .filter(|import| import.unresolved.is_some())
        .count();
    Ok(RawGraph {
        collector: Collector {
            name: "elixir-ast".into(),
            version: Some(dump.elixir_version),
            cruised: dump.files.len(),
            unresolved: Some(unresolved),
            ..Default::default()
        },
        files: dump.files.into_iter().chain(dump.excluded_files).collect(),
        edges: dump.edges,
        imports: Some(dump.imports),
        metadata: Default::default(),
    })
}
