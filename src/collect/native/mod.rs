//! Native source parsing and resolution; graph analysis is shared with the reference collectors.
pub mod elixir;
pub mod go;
pub mod java;
pub mod python;
pub mod rust;
pub mod typescript;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::RawGraph;
use crate::model::ImportRecord;
use crate::surface::{SymbolExport, SymbolTarget, Symbols};

/// Parsed syntax stays private to each collector; explicit invalidation avoids reading unchanged files.
#[derive(Default)]
pub(super) struct ParseCache<T> {
    entries: std::collections::BTreeMap<String, T>,
    pub parsed: usize,
    pub reused: usize,
    fresh: std::collections::BTreeSet<String>,
    reused_files: std::collections::BTreeSet<String>,
}
impl<T: Clone> ParseCache<T> {
    pub fn get(&mut self, file: &str, parse: impl FnOnce() -> Result<T>) -> Result<T> {
        if let Some(value) = self.entries.get(file) {
            if !self.fresh.contains(file) {
                self.reused_files.insert(file.to_owned());
                self.reused = self.reused_files.len();
            }
            return Ok(value.clone());
        }
        let value = parse()?;
        self.parsed += 1;
        self.fresh.insert(file.to_owned());
        self.entries.insert(file.to_owned(), value.clone());
        Ok(value)
    }
    pub fn invalidate(&mut self, files: &[String]) {
        self.parsed = 0;
        self.reused = 0;
        self.fresh.clear();
        self.reused_files.clear();
        for file in files {
            self.entries.remove(file);
        }
    }
}

pub struct Output {
    pub raw: RawGraph,
    pub symbols: Symbols,
    pub source_root: String,
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
/// An export defined by the file that declares it.
fn self_export(file: &str, name: &str) -> SymbolExport {
    SymbolExport {
        file: file.to_owned(),
        name: name.to_owned(),
        target: SymbolTarget {
            file: file.to_owned(),
            name: name.to_owned(),
        },
    }
}
fn unresolved_count(records: &[ImportRecord]) -> usize {
    records.iter().filter(|r| r.unresolved.is_some()).count()
}

/// Inventory regular source files without following repository symlinks or package installations.
pub fn source_files(root: &Path, extensions: &[&str]) -> Result<Vec<String>> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        // Build tools can remove temporary directories while we inventory a working tree.
        let entries = match fs::read_dir(&dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            result => result.with_context(|| format!("reading {}", dir.display()))?,
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let kind = match entry.file_type() {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                result => result?,
            };
            if kind.is_dir() {
                if !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "node_modules" | ".venv" | "venv" | "__pycache__")
                ) {
                    pending.push(path);
                }
            } else if kind.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| extensions.contains(&ext))
            {
                files.push(
                    path.strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    #[test]
    fn inventory_tolerates_a_disappeared_directory() {
        let missing =
            std::env::temp_dir().join(format!("boundrail-native-vanished-{}", std::process::id()));
        assert!(!missing.exists());
        assert!(super::source_files(&missing, &["ts"]).unwrap().is_empty());
    }
}
