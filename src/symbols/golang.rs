//! Source-only Go public symbols and cross-package references.
use std::{fs, path::Path, process::Command};

use crate::{model::Document, surface::Symbols};
use anyhow::{Context, Result, bail};

pub fn collect(root: &Path, doc: &Document, go: &str) -> Result<Symbols> {
    let root = fs::canonicalize(root)?;
    let go = if go.contains('/') {
        fs::canonicalize(go)?.to_string_lossy().into_owned()
    } else {
        go.to_owned()
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "archgraph-go-symbols-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&dir)?;
    let result = (|| -> Result<Symbols> {
        fs::write(dir.join("go_dump.go"), include_str!("go_dump.go"))?;
        let files: Vec<&str> = doc
            .files
            .iter()
            .filter(|f| f.path.ends_with(".go"))
            .map(|f| f.path.as_str())
            .collect();
        let output = crate::collect::run_with_input(
            Command::new(&go)
                .args(["run", "go_dump.go"])
                .arg(&root)
                .current_dir(&dir)
                .env("PWD", &dir)
                .envs(crate::collect::go::ENV.iter().copied())
                .env("GOENV", "off")
                .env("GOCACHEPROG", ""),
            serde_json::to_vec(&files)?,
        )
        .with_context(|| format!("running {go} for Go surface symbols"))?;
        if !output.status.success() {
            bail!(
                "Go symbol helper failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        serde_json::from_slice(&output.stdout).context("reading Go symbol helper JSON")
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}
