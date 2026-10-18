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

use super::RawGraph;
use crate::surface::Symbols;

pub struct Output {
    pub raw: RawGraph,
    pub symbols: Symbols,
    pub source_root: String,
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
            std::env::temp_dir().join(format!("archgraph-native-vanished-{}", std::process::id()));
        assert!(!missing.exists());
        assert!(super::source_files(&missing, &["ts"]).unwrap().is_empty());
    }
}
