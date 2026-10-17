//! Source-only symbol collection. Repository code is never imported or executed.
pub mod golang;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::model::{Document, Language};
use crate::surface::Symbols;

pub struct Options<'a> {
    pub depcruise: Option<&'a Path>,
    pub python: &'a str,
    pub go: &'a str,
}

pub fn collect(root: &Path, doc: &Document, options: Options<'_>) -> Result<Symbols> {
    let root = root.canonicalize().context("resolve symbol scan root")?;
    for file in &doc.files {
        let path = Path::new(&file.path);
        if path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || path.as_os_str().is_empty()
            || !root
                .join(path)
                .canonicalize()
                .with_context(|| format!("resolve symbol source {}", file.path))?
                .starts_with(&root)
        {
            bail!("symbol source must stay inside scan root: {}", file.path);
        }
    }
    let root = root.as_path();
    if doc.source.language == Language::Go {
        return golang::collect(root, doc, options.go);
    }
    let (program, flag, script) = match doc.source.language {
        Language::Typescript => ("node", "-e", include_str!("typescript.cjs")),
        Language::Python => (options.python, "-c", include_str!("python.py")),
        language => {
            return Ok(Symbols {
                complete: false,
                exports: Vec::new(),
                imports: Vec::new(),
                limitations: vec![format!("Symbol collection is unavailable for {language:?}")],
            });
        }
    };
    let input = serde_json::to_vec(&json!({
        "root": root.canonicalize().context("resolve symbol scan root")?,
        "files": doc.files.iter().map(|file| &file.path).collect::<Vec<_>>(),
        "depcruise": options.depcruise,
    }))?;
    let mut command = Command::new(program);
    if doc.source.language == Language::Python {
        command.args(["-I", "-B"]);
    }
    let mut child = command
        .args([flag, script])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("start {program} symbol collector"))?;
    child
        .stdin
        .take()
        .context("symbol collector stdin")?
        .write_all(&input)?;
    let output = child
        .wait_with_output()
        .context("wait for symbol collector")?;
    if !output.status.success() {
        bail!(
            "symbol collector failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("decode symbol collector output")
}
