//! Conservative source edits with a private Git index and verified rollback.
use crate::{
    model::{Document, Language, Move},
    recipes::{Recipe, StepKind, VerificationResult},
    rewrite::{self, Edit},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub struct Plan {
    pub schema: &'static str,
    pub kind: &'static str,
    pub finding_id: String,
    pub recipe: Recipe,
    pub can_apply: bool,
    pub pending: Vec<String>,
    pub changes: Vec<Change>,
    pub moves: Vec<Move>,
    #[serde(skip)]
    buffers: Vec<Buffer>,
    #[serde(skip)]
    root: PathBuf,
}
#[derive(Debug, Serialize)]
pub struct Change {
    pub file: String,
    pub to: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub before: String,
    pub after: String,
}
#[derive(Debug)]
struct Buffer {
    file: String,
    to: String,
    before: Vec<u8>,
    after: Vec<u8>,
    permissions: fs::Permissions,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub kind: &'static str,
    pub finding_id: String,
    pub applied: bool,
    pub verify: VerificationResult,
    pub hint: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    finding_id: String,
    before: Value,
    recipe: Recipe,
    fingerprints: BTreeMap<String, String>,
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn valid_id(id: &str) -> Result<()> {
    ensure!(
        id.starts_with("ag-")
            && id.len() == 67
            && id[3..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid finding_id: use an ag- ID returned by report"
    );
    Ok(())
}
fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(
        crate::collect::is_relative_path(relative, false)
            && !relative
                .split('/')
                .any(|p| p == ".git" || p == ".archgraph"),
        "unsafe source path {relative:?}: use a repository-relative file outside Git/archgraph state"
    );
    checked_path(root, relative)
}
fn checked_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        let Component::Normal(part) = part else {
            bail!("unsafe relative path {relative:?}")
        };
        path.push(part);
        crate::history::refuse_symlink(&path)?;
        if path.exists() {
            ensure!(
                path == root.join(relative) || path.is_dir(),
                "path parent is not a directory: {}",
                path.display()
            );
            ensure!(
                path.canonicalize()?.starts_with(root),
                "path escapes scan root: {relative}"
            );
        }
    }
    Ok(path)
}
fn read_buffer(root: &Path, file: &str, to: &str) -> Result<Buffer> {
    let path = safe_path(root, file)?;
    ensure!(
        path.is_file(),
        "{file}: source file missing; rescan before applying"
    );
    let before = fs::read(&path)?;
    std::str::from_utf8(&before)
        .with_context(|| format!("{file}: non-UTF-8 source; edit manually"))?;
    Ok(Buffer {
        file: file.into(),
        to: to.into(),
        after: before.clone(),
        before,
        permissions: fs::metadata(path)?.permissions(),
    })
}
fn relative(from: &str, to: &str) -> String {
    let a: Vec<_> = from
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let b: Vec<_> = to
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; a.len() - common];
    parts.extend_from_slice(&b[common..]);
    let result = parts.join("/");
    if result.starts_with('.') {
        result
    } else {
        format!("./{result}")
    }
}
fn parent(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(p, _)| p)
}
fn module_name(doc: &Document, path: &str) -> Result<String> {
    let package = doc
        .config
        .collection
        .as_ref()
        .and_then(|c| c.python_package.as_deref())
        .unwrap_or("")
        .replace('.', "/");
    let source = doc.config.source_root.trim_matches('/');
    let base = if package.is_empty() || package == "/" {
        source
    } else {
        source
            .strip_suffix(&package)
            .context(
                "Python package does not match the configured source root; rescan before rewriting",
            )?
            .trim_end_matches('/')
    };
    let path = if !base.is_empty() && base != "." {
        path.strip_prefix(&format!("{base}/"))
            .context("Python move escapes its import search root; edit imports manually")?
    } else {
        path
    };
    let path = path
        .strip_suffix(".py")
        .context("Python destination must end in .py")?;
    let module = path
        .strip_suffix("/__init__")
        .unwrap_or(path)
        .replace('/', ".");
    ensure!(
        !module.is_empty(),
        "cannot rewrite root Python package import; edit manually"
    );
    Ok(module)
}
fn python_target(
    doc: &Document,
    file: &str,
    target: &str,
    span: &rewrite::Specifier,
) -> Result<()> {
    let count = span.value.chars().take_while(|c| *c == '.').count();
    let suffix = span.value.trim_start_matches('.');
    let resolved = if count == 0 {
        span.value.clone()
    } else {
        ensure!(
            !suffix.is_empty(),
            "Python package-member import requires agent binding analysis"
        );
        let module = module_name(doc, file)?;
        let package = if file.ends_with("/__init__.py") {
            module.as_str()
        } else {
            module.rsplit_once('.').map_or("", |(p, _)| p)
        };
        let mut components: Vec<_> = package.split('.').filter(|c| !c.is_empty()).collect();
        ensure!(
            count <= components.len(),
            "Python relative import escapes the package"
        );
        components.truncate(components.len() + 1 - count);
        components.push(suffix);
        components.join(".")
    };
    ensure!(
        resolved == module_name(doc, target)?,
        "Python import selects a package member rather than its module; edit bindings manually"
    );
    Ok(())
}
fn replacement(
    doc: &Document,
    from: &str,
    target: &str,
    old: &rewrite::Specifier,
) -> Result<String> {
    match doc.source.language {
        Language::Typescript => {
            let mut target = target.to_owned();
            let extension = Path::new(&target)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let old_ext = Path::new(&old.value).extension().and_then(|s| s.to_str());
            if let Some(ext) = old_ext.filter(|ext| matches!(*ext, "js" | "jsx" | "mjs" | "cjs")) {
                if matches!(extension, "ts" | "tsx" | "mts" | "cts") {
                    target = Path::new(&target)
                        .with_extension(ext)
                        .to_string_lossy()
                        .into_owned();
                }
            } else if old_ext.is_none()
                && matches!(
                    extension,
                    "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"
                )
            {
                target.truncate(target.len() - extension.len() - 1);
            }
            Ok(relative(parent(from), &target))
        }
        Language::Python => {
            let target = module_name(doc, target)?;
            if old.form == "import" {
                ensure!(
                    old.binding.is_some(),
                    "Python import binding would change; add an alias manually"
                );
                return Ok(target);
            }
            if !old.value.starts_with('.') {
                return Ok(target);
            }
            let source = module_name(doc, from)?;
            let package = if from.ends_with("/__init__.py") {
                source.as_str()
            } else {
                source.rsplit_once('.').map_or("", |(p, _)| p)
            };
            let a: Vec<_> = package.split('.').filter(|s| !s.is_empty()).collect();
            let b: Vec<_> = target.split('.').collect();
            let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
            ensure!(
                common > 0,
                "Python relative import crosses top-level packages; edit manually"
            );
            let suffix = b[common..].join(".");
            ensure!(
                !suffix.is_empty(),
                "Python package-member import needs binding analysis; edit manually"
            );
            Ok(format!("{}{}", ".".repeat(a.len() - common + 1), suffix))
        }
        _ => bail!("language needs package/declaration changes; edit imports manually"),
    }
}
fn select_span(
    doc: &Document,
    file: &str,
    bytes: &[u8],
    specifier: &str,
    line: Option<usize>,
) -> Result<rewrite::Specifier> {
    let matches: Vec<_> = rewrite::specifiers(doc.source.language, file, bytes)?
        .into_iter()
        .filter(|s| s.value == specifier && line.is_none_or(|n| s.line == n))
        .collect();
    ensure!(
        matches.len() == 1 && matches[0].safe,
        "{file}: import has no unique safe source span; rescan or edit manually"
    );
    Ok(matches.into_iter().next().unwrap())
}

pub fn plan(root: &Path, before: &Document, finding_id: &str) -> Result<Plan> {
    valid_id(finding_id)?;
    let root = root
        .canonicalize()
        .context("scan root missing: run with cwd=<repo>")?;
    let mut recipe = crate::recipes::recipe_for(before, finding_id)?.clone();
    let moves: Vec<Move> = recipe
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::MoveFile)
        .map(|s| Move {
            file: s.from.clone(),
            to: s.to.clone(),
        })
        .collect();
    let mut buffers: BTreeMap<String, Buffer> = BTreeMap::new();
    let mut pending = Vec::new();
    if let Err(error) = crate::git::toplevel(&root) {
        pending.push(format!("not a git repo: run with cwd=<repo> ({error:#})"));
    }
    let mut destinations = BTreeSet::new();
    for m in &moves {
        safe_path(&root, &m.file)?;
        let target = safe_path(&root, &m.to)?;
        ensure!(
            !target.exists() && destinations.insert(m.to.clone()),
            "destination {} already exists or is used twice; choose an empty destination",
            m.to
        );
        ensure!(
            !buffers.contains_key(&m.file),
            "file moved twice: {}",
            m.file
        );
        buffers.insert(m.file.clone(), read_buffer(&root, &m.file, &m.to)?);
        if crate::git::run(&root, &["ls-files", "--error-unmatch", "--", &m.file]).is_err() {
            pending.push(format!(
                "{}: git mv requires a tracked file; add it to Git before applying",
                m.file
            ));
        }
        match before.source.language {
            Language::Rust|Language::Java=>pending.push(format!("{}: moving this file requires module/package declarations; ask the agent to perform the move",m.file)),
            Language::Go if parent(&m.file)!=parent(&m.to)=>pending.push(format!("{}: cross-package Go moves require package/binding analysis; move the complete package manually",m.file)),
            _=>{}
        }
    }
    for record in before
        .imports
        .iter()
        .flatten()
        .filter(|record| moves.iter().any(|m| m.file == record.from))
    {
        if record.unresolved.is_some()
            || record.targets.iter().any(|target| {
                !before.edges.iter().any(|edge| {
                    edge.from == record.from
                        && (edge.to == *target || edge.original_to.as_ref() == Some(target))
                })
            })
        {
            pending.push(format!("{}: unresolved or excluded import {:?}; include/resolve its target before moving the file",record.from,record.specifier));
        }
    }
    let moved = |path: &str| {
        moves
            .iter()
            .find(|m| m.file == path)
            .map_or_else(|| path.to_owned(), |m| m.to.clone())
    };
    let mut edits: BTreeMap<String, Vec<Edit>> = BTreeMap::new();
    // Every physical import into or out of a relocated file must be accounted for.
    for edge in before.edges.iter().chain(&before.external_edges) {
        let physical = edge.original_to.as_deref().unwrap_or(&edge.to);
        let from = moved(&edge.from);
        let to = moved(physical);
        if from == edge.from && to == physical {
            continue;
        }
        // A host global use has no import to rewrite.
        if before
            .external_nodes
            .iter()
            .any(|node| node.id == physical && node.global)
        {
            continue;
        }
        if matches!(before.source.language, Language::Elixir | Language::Go) {
            continue;
        }
        if !matches!(
            before.source.language,
            Language::Typescript | Language::Python
        ) {
            continue;
        }
        let result = (|| -> Result<()> {
            let buffer = if let Some(b) = buffers.get(&edge.from) {
                b
            } else {
                buffers
                    .entry(edge.from.clone())
                    .or_insert(read_buffer(&root, &edge.from, &edge.from)?)
            };
            let specifier = edge
                .specifier
                .as_deref()
                .context("import has no recorded specifier; rescan using the native engine")?;
            let span = select_span(before, &edge.from, &buffer.before, specifier, edge.line)?;
            // Absolute external imports are independent of source-file placement.
            if physical.starts_with("pkg:")
                || before.external_nodes.iter().any(|node| node.id == physical)
            {
                ensure!(
                    !specifier.starts_with('.'),
                    "unresolved relative external import; edit manually"
                );
                return Ok(());
            }
            if before.source.language == Language::Python {
                python_target(before, &edge.from, physical, &span)?;
            }
            let value = replacement(before, &from, &to, &span)?;
            if value != span.value {
                edits.entry(edge.from.clone()).or_default().push(Edit {
                    range: span.range.clone(),
                    expected: buffer.before[span.range].to_vec(),
                    replacement: value.into_bytes(),
                });
            }
            Ok(())
        })();
        if let Err(e) = result {
            pending.push(format!("{}: {e:#}", edge.from))
        }
    }
    for step in &mut recipe.steps {
        let result = (|| -> Result<()> {
            match step.kind {
                StepKind::MoveFile => {
                    ensure!(step.mechanical, "{}: move requires agent work", step.file)
                }
                StepKind::CutImport => {
                    ensure!(
                        step.detail["operation"] != "replace_global_use",
                        "replacing a host global use requires agent work"
                    );
                    let buffer = if let Some(b) = buffers.get(&step.file) {
                        b
                    } else {
                        buffers
                            .entry(step.file.clone())
                            .or_insert(read_buffer(&root, &step.file, &step.file)?)
                    };
                    let specifier = step.detail["specifier"]
                        .as_str()
                        .context("import specifier unavailable: rescan with the native engine")?;
                    let span =
                        select_span(before, &step.file, &buffer.before, specifier, step.line)?;
                    let edit = if step.detail["operation"] == "remove_import" {
                        let range=rewrite::removal_range(before.source.language,&step.file,&buffer.before,&span)?
                            .context("import has live or ambiguous bindings; remove references manually before cutting it")?;
                        let matching: Vec<_> = before
                            .edges
                            .iter()
                            .chain(&before.external_edges)
                            .filter(|edge| {
                                edge.from == step.file
                                    && edge.line == Some(span.line)
                                    && edge.specifier.as_deref() == Some(specifier)
                            })
                            .collect();
                        ensure!(
                            !matching.is_empty() && matching.iter().all(|edge| edge.to == step.to),
                            "import serves multiple targets or lacks exact graph evidence; rescan or edit manually"
                        );
                        step.line = Some(span.line);
                        step.mechanical = true;
                        Edit {
                            range: range.clone(),
                            expected: buffer.before[range].to_vec(),
                            replacement: Vec::new(),
                        }
                    } else if step.detail["operation"] == "rewrite_specifier" {
                        ensure!(
                            step.mechanical,
                            "import rewrite requires agent binding changes"
                        );
                        if before.source.language == Language::Python {
                            let old_target = step.detail["old_target"]
                                .as_str()
                                .or_else(|| step.detail["target"].as_str())
                                .context("Python rewrite lacks original target")?;
                            python_target(before, &step.file, old_target, &span)?;
                        }
                        let value = replacement(before, &moved(&step.file), &step.to, &span)?;
                        Edit {
                            range: span.range.clone(),
                            expected: buffer.before[span.range].to_vec(),
                            replacement: value.into_bytes(),
                        }
                    } else {
                        bail!("unsupported cut operation; apply the agent instructions manually")
                    };
                    edits.entry(step.file.clone()).or_default().push(edit);
                }
                StepKind::AddReexport => {
                    ensure!(step.mechanical, "re-export requires agent work");
                    let buffer = buffers
                        .entry(step.file.clone())
                        .or_insert(read_buffer(&root, &step.file, &step.file)?);
                    let names: Vec<String> = step.detail["symbols"]
                        .as_array()
                        .context("re-export lacks symbols")?
                        .iter()
                        .map(|s| {
                            s.as_str()
                                .map(String::from)
                                .context("invalid re-export symbol")
                        })
                        .collect::<Result<_>>()?;
                    let existing = std::str::from_utf8(&buffer.before)?;
                    ensure!(
                        names.iter().all(|n| !existing
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|word| word == n)),
                        "entrypoint already mentions an exported binding; resolve the ambiguity manually"
                    );
                    let dummy = rewrite::Specifier {
                        range: 0..0,
                        statement: 0..0,
                        value: ".".into(),
                        line: 1,
                        form: "from".into(),
                        binding: None,
                        safe: true,
                    };
                    let target = step.detail["target"]
                        .as_str()
                        .context("re-export lacks target")?;
                    let specifier = replacement(before, &step.file, target, &dummy)?;
                    buffer.after = rewrite::append_reexport(
                        before.source.language,
                        &step.file,
                        &buffer.after,
                        &specifier,
                        &names,
                    )?;
                }
                _ => bail!(
                    "{:?}: non-mechanical step; follow its instructions and run verify",
                    step.kind
                ),
            }
            Ok(())
        })();
        if let Err(e) = result {
            pending.push(format!("{}: {e:#}", step.file));
        }
    }
    for (file, mut changes) in edits {
        changes.sort_by_key(|e| e.range.start);
        changes.dedup_by(|a, b| a.range == b.range && a.replacement == b.replacement);
        let buffer = buffers.get_mut(&file).unwrap();
        match rewrite::replace(&buffer.before, &changes) {
            Ok(bytes) => {
                ensure!(
                    buffer.after == buffer.before,
                    "{file}: combined re-export/import edits need agent review"
                );
                buffer.after = bytes
            }
            Err(e) => pending.push(format!("{file}: {e:#}")),
        }
    }
    pending.sort();
    pending.dedup();
    if pending.is_empty() {
        crate::recipes::forecast(before, &mut recipe)?;
    }
    let buffers: Vec<_> = buffers
        .into_values()
        .filter(|b| b.file != b.to || b.before != b.after)
        .collect();
    let changes = buffers
        .iter()
        .map(|b| Change {
            file: b.file.clone(),
            to: b.to.clone(),
            before_sha256: hash(&b.before),
            after_sha256: hash(&b.after),
            before: String::from_utf8(b.before.clone()).unwrap(),
            after: String::from_utf8(b.after.clone()).unwrap(),
        })
        .collect();
    Ok(Plan {
        schema: crate::model::SCHEMA,
        kind: "apply-plan",
        finding_id: finding_id.into(),
        recipe,
        can_apply: pending.is_empty() && !buffers.is_empty(),
        pending,
        changes,
        moves,
        buffers,
        root,
    })
}

struct IndexGuard {
    lock: PathBuf,
    private: PathBuf,
    index: PathBuf,
    before: Vec<u8>,
    lock_file: Option<fs::File>,
}
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.created().ok() == b.created().ok() && a.is_file() == b.is_file()
    }
}
impl IndexGuard {
    fn new(root: &Path) -> Result<Self> {
        let index = PathBuf::from(
            crate::git::text(
                root,
                &["rev-parse", "--path-format=absolute", "--git-path", "index"],
            )
            .context("not a git repo: run with cwd=<repo>")?
            .trim_end_matches(['\n', '\r']),
        );
        crate::history::refuse_symlink(&index)?;
        let lock = index.with_file_name(format!(
            "{}.lock",
            index.file_name().unwrap().to_string_lossy()
        ));
        let lock_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .context("Git index is locked: wait for the other Git operation, then retry")?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let private =
            index.with_file_name(format!("archgraph-index-{}-{stamp}", std::process::id()));
        let mut guard = Self {
            lock,
            private,
            index,
            before: Vec::new(),
            lock_file: Some(lock_file),
        };
        guard
            .lock_file
            .as_ref()
            .unwrap()
            .set_permissions(fs::metadata(&guard.index)?.permissions())?;
        guard.before = fs::read(&guard.index)
            .context("Git index missing: add the affected files with git add before applying")?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&guard.private)?;
        f.write_all(&guard.before)?;
        f.sync_all()?;
        Ok(guard)
    }
    fn publish(&mut self) -> Result<()> {
        ensure!(
            fs::read(&self.index)? == self.before,
            "Git index changed despite its lock; refusing to overwrite concurrent staging"
        );
        crate::history::refuse_symlink(&self.lock)?;
        ensure!(
            same_file(
                &self
                    .lock_file
                    .as_ref()
                    .context("index lock lost")?
                    .metadata()?,
                &fs::metadata(&self.lock)?
            ),
            "Git index lock replaced concurrently; refusing to overwrite staging"
        );
        let bytes = fs::read(&self.private)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.lock)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&self.lock, &self.index)?;
        self.lock_file = None;
        Ok(())
    }
}
impl Drop for IndexGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.private);
        let _ = fs::remove_file(self.private.with_extension("lock"));
        if self
            .lock_file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .zip(fs::metadata(&self.lock).ok())
            .is_some_and(|(a, b)| same_file(&a, &b))
        {
            let _ = fs::remove_file(&self.lock);
        }
    }
}
fn write_preserving(path: &Path, bytes: &[u8], permissions: &fs::Permissions) -> Result<()> {
    crate::history::refuse_symlink(path)?;
    let temp = path.with_file_name(format!(
        ".archgraph-edit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.set_permissions(permissions.clone())?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn make_parents(root: &Path, path: &Path, created: &mut Vec<PathBuf>) -> Result<()> {
    let parent = path.parent().context("destination has no parent")?;
    let mut current = root.to_path_buf();
    for component in parent.strip_prefix(root)?.components() {
        current.push(component);
        crate::history::refuse_symlink(&current)?;
        if !current.exists() {
            fs::create_dir(&current)?;
            created.push(current.clone());
        }
        ensure!(
            current.is_dir(),
            "destination parent is not a directory: {}",
            current.display()
        );
    }
    Ok(())
}
fn rollback(
    root: &Path,
    plan: &Plan,
    touched: &BTreeSet<String>,
    created: &[PathBuf],
) -> Result<()> {
    let mut conflicts = Vec::new();
    for buffer in plan
        .buffers
        .iter()
        .rev()
        .filter(|b| touched.contains(&b.file))
    {
        let result = (|| -> Result<()> {
            let from = safe_path(root, &buffer.file)?;
            let to = safe_path(root, &buffer.to)?;
            if buffer.file != buffer.to {
                if !to.exists() {
                    ensure!(
                        fs::read(&from)? == buffer.before,
                        "{} changed concurrently; preserve it and restore manually",
                        buffer.file
                    );
                    return Ok(());
                }
                let actual = fs::read(&to)?;
                ensure!(
                    actual == buffer.after || actual == buffer.before,
                    "{} changed concurrently; refusing to overwrite it during rollback",
                    buffer.to
                );
                ensure!(
                    !from.exists(),
                    "{} was recreated concurrently; refusing rollback overwrite",
                    buffer.file
                );
                fs::rename(&to, &from)?;
                if actual != buffer.before {
                    write_preserving(&from, &buffer.before, &buffer.permissions)?;
                }
            } else {
                let actual = fs::read(&from)?;
                ensure!(
                    actual == buffer.after || actual == buffer.before,
                    "{} changed concurrently; refusing rollback overwrite",
                    buffer.file
                );
                if actual != buffer.before {
                    write_preserving(&from, &buffer.before, &buffer.permissions)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            conflicts.push(format!("{error:#}"));
        }
    }
    for directory in created.iter().rev() {
        let _ = fs::remove_dir(directory);
    }
    ensure!(
        conflicts.is_empty(),
        "rollback conflict: {}. User edits were preserved; inspect git diff and restore only the recipe files manually",
        conflicts.join("; ")
    );
    Ok(())
}
fn receipt_path(root: &Path, id: &str) -> Result<PathBuf> {
    valid_id(id)?;
    checked_path(root, &format!(".archgraph/applied/{id}.json"))
}

fn preserve_resolution(before: &Document, after: &Document, plan: &Plan) -> Result<()> {
    let moved = |path: &str| {
        plan.moves
            .iter()
            .find(|m| m.file == path)
            .map_or_else(|| path.to_owned(), |m| m.to.clone())
    };
    let mut counts = BTreeMap::new();
    for record in &before.unresolved {
        *counts
            .entry((
                moved(&record.from),
                record.specifier.clone(),
                format!("{:?}", record.unresolved),
            ))
            .or_insert(0usize) += 1;
    }
    for record in &after.unresolved {
        let count = counts
            .entry((
                record.from.clone(),
                record.specifier.clone(),
                format!("{:?}", record.unresolved),
            ))
            .or_insert(0);
        ensure!(
            *count > 0,
            "apply introduced unresolved import {:?} in {}; restoring recipe files",
            record.specifier,
            record.from
        );
        *count -= 1;
    }
    for edge in &before.external_edges {
        let intentionally_cut = plan.recipe.steps.iter().any(|step| {
            step.kind == StepKind::CutImport
                && step.detail["operation"] == "remove_import"
                && step.file == edge.from
                && step.to == edge.to
                && step.line.is_none_or(|line| edge.line == Some(line))
        });
        if intentionally_cut {
            continue;
        }
        ensure!(
            after
                .external_edges
                .iter()
                .any(|e| e.from == moved(&edge.from)
                    && e.to == edge.to
                    && e.kind == edge.kind
                    && e.specifier == edge.specifier),
            "apply lost or changed external import {:?} in {}; restoring recipe files",
            edge.specifier,
            edge.from
        );
    }
    Ok(())
}

pub fn execute(
    root: &Path,
    before: &Document,
    plan: &Plan,
    mut rescan: impl FnMut() -> Result<Document>,
) -> Result<Report> {
    ensure!(
        plan.can_apply && plan.pending.is_empty(),
        "recipe is not fully mechanical: {}; apply its agent instructions and run verify",
        plan.pending.join("; ")
    );
    let root = root.canonicalize()?;
    ensure!(
        root == plan.root,
        "plan belongs to another scan root; regenerate it"
    );
    let original = crate::recipes::recipe_for(before, &plan.finding_id)?;
    ensure!(
        original.steps.len() == plan.recipe.steps.len(),
        "recipe changed; regenerate the plan"
    );
    for buffer in &plan.buffers {
        ensure!(
            fs::read(safe_path(&root, &buffer.file)?)? == buffer.before,
            "{} changed since planning; rescan and regenerate the plan",
            buffer.file
        );
        if buffer.file != buffer.to {
            ensure!(
                !safe_path(&root, &buffer.to)?.exists(),
                "{} now exists; regenerate the plan",
                buffer.to
            );
        }
    }
    let receipt = receipt_path(&root, &plan.finding_id)?;
    let old_receipt = if receipt.exists() {
        read_receipt(&root, &receipt, &plan.finding_id)?;
        Some(fs::read(&receipt)?)
    } else {
        None
    };
    let mut guard = IndexGuard::new(&root)?;
    let mut touched = BTreeSet::new();
    let mut created = Vec::new();
    let result = (|| -> Result<VerificationResult> {
        for m in &plan.moves {
            let buffer = plan
                .buffers
                .iter()
                .find(|b| b.file == m.file)
                .context("move source missing from the plan")?;
            ensure!(
                fs::read(safe_path(&root, &m.file)?)? == buffer.before,
                "{} changed before move; regenerate the plan",
                m.file
            );
            let target = safe_path(&root, &m.to)?;
            ensure!(
                !target.exists(),
                "{} appeared before move; regenerate the plan",
                m.to
            );
            make_parents(&root, &target, &mut created)?;
            touched.insert(m.file.clone());
            crate::git::run_with_index(&root, &["mv", "--", &m.file, &m.to], Some(&guard.private))
                .with_context(|| {
                    format!(
                        "cannot git mv {}: ensure it is tracked and the destination is empty",
                        m.file
                    )
                })?;
        }
        for buffer in &plan.buffers {
            let path = safe_path(&root, &buffer.to)?;
            ensure!(
                fs::read(&path)? == buffer.before,
                "{} changed during apply; refusing to overwrite it",
                buffer.to
            );
            touched.insert(buffer.file.clone());
            if buffer.after != buffer.before {
                write_preserving(&path, &buffer.after, &buffer.permissions)?;
            }
        }
        let after = rescan().context("post-apply scan failed; rolling back recipe changes")?;
        ensure!(
            serde_json::to_value(&before.config)? == serde_json::to_value(&after.config)?,
            "scan configuration or layout rules changed during apply; regenerate the plan"
        );
        preserve_resolution(before, &after, plan)?;
        let verification = crate::recipes::check(before, &after, &plan.recipe);
        if !verification.pass {
            return Ok(verification);
        }
        for buffer in &plan.buffers {
            ensure!(
                fs::read(safe_path(&root, &buffer.to)?)? == buffer.after,
                "{} changed during verification; preserving concurrent edits",
                buffer.to
            );
        }
        let receipt_value = Receipt {
            schema: "archgraph/apply-receipt/v1".into(),
            finding_id: plan.finding_id.clone(),
            before: serde_json::to_value(before)?,
            recipe: plan.recipe.clone(),
            fingerprints: plan
                .buffers
                .iter()
                .map(|b| (b.to.clone(), hash(&b.after)))
                .collect(),
        };
        make_parents(&root, &receipt, &mut created)?;
        let encoded = serde_json::to_vec_pretty(&receipt_value)?;
        ensure!(
            fs::read(&receipt).ok() == old_receipt,
            "apply receipt changed concurrently; refusing to overwrite it"
        );
        write_state(&receipt, &encoded)?;
        if let Err(error) = guard.publish() {
            ensure!(
                fs::read(&receipt)? == encoded,
                "receipt changed concurrently after index publication failed: {error:#}; preserve it and inspect manually"
            );
            if let Some(previous) = &old_receipt {
                write_state(&receipt, previous)?;
            } else {
                fs::remove_file(&receipt)?;
            }
            return Err(error);
        }
        Ok(verification)
    })();
    match result {
        Ok(verify) if verify.pass=>Ok(Report{schema:crate::model::SCHEMA,kind:"apply",finding_id:plan.finding_id.clone(),applied:true,verify,hint:Some("Run the language build and tests; import-level verification does not prove runtime behavior.".into())}),
        Ok(verify)=>{
            rollback(&root,plan,&touched,&created)?;
            let failed=verify.checks.iter().filter(|c|!c.pass).map(|c|format!("{}: expected {}, got {}",c.predicate,c.expected,c.actual)).collect::<Vec<_>>().join("; ");
            Ok(Report{schema:crate::model::SCHEMA,kind:"apply",finding_id:plan.finding_id.clone(),applied:false,verify,hint:Some(format!("verify: FAILED ({failed}). Recipe changes were rolled back; rescan and review the actual resolved imports before retrying."))})
        }
        Err(error)=>{rollback(&root,plan,&touched,&created).with_context(||format!("apply failed: {error:#}"))?;Err(error.context("recipe changes rolled back; inspect the error, then rescan before retrying"))}
    }
}

pub fn verify_receipt(root: &Path, id: &str, after: &Document) -> Result<VerificationResult> {
    let root = root.canonicalize()?;
    let path = receipt_path(&root, id)?;
    let metadata = fs::metadata(&path)
        .context("no apply receipt: run apply --yes first, or verify using explicit predicates")?;
    ensure!(
        metadata.is_file() && metadata.len() <= 64 * 1024 * 1024,
        "invalid or oversized apply receipt; inspect it manually"
    );
    let receipt: Receipt = serde_json::from_slice(&fs::read(path)?)
        .context("invalid apply receipt: rescan and verify explicit predicates")?;
    ensure!(
        receipt.schema == "archgraph/apply-receipt/v1" && receipt.finding_id == id,
        "apply receipt identity mismatch; inspect it manually"
    );
    for (file, expected) in &receipt.fingerprints {
        ensure!(
            expected.len() == 64 && expected.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid receipt fingerprint"
        );
        ensure!(
            hash(&fs::read(safe_path(&root, file)?)?) == *expected,
            "{file} changed after apply; verify explicit current predicates instead of the old receipt"
        );
    }
    let before = crate::restore_scan(serde_json::from_value(receipt.before)?)?;
    ensure!(
        serde_json::to_value(&before.config)? == serde_json::to_value(&after.config)?,
        "scan configuration or layout rules changed since apply; verify explicit current predicates"
    );
    Ok(crate::recipes::check(&before, after, &receipt.recipe))
}

fn read_receipt(root: &Path, path: &Path, id: &str) -> Result<Receipt> {
    ensure!(path.starts_with(root), "receipt escapes scan root");
    let meta = fs::metadata(path).with_context(|| {
        format!("missing recipe baseline for {id}: call propose again before editing")
    })?;
    ensure!(
        meta.is_file() && meta.len() <= 64 * 1024 * 1024,
        "invalid or oversized recipe baseline; call propose again"
    );
    let receipt: Receipt = serde_json::from_slice(&fs::read(path)?)
        .context("invalid recipe baseline: call propose again")?;
    ensure!(
        receipt.schema == "archgraph/apply-receipt/v1" && receipt.finding_id == id,
        "recipe baseline identity mismatch: call propose again"
    );
    Ok(receipt)
}
fn write_state(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= 64 * 1024 * 1024,
        "recipe baseline exceeds 64 MiB; use explicit current predicates instead"
    );
    let temp = path.with_file_name(format!(
        ".archgraph-state-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        crate::history::refuse_symlink(path)?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
/// Preserve the scan against which an agent's manual recipe changes will be measured.
pub fn remember_proposal(root: &Path, before: &Document, id: &str, recipe: &Recipe) -> Result<()> {
    valid_id(id)?;
    crate::recipes::recipe_for(before, id)?;
    let root = root.canonicalize()?;
    let path = checked_path(&root, &format!(".archgraph/proposed/{id}.json"))?;
    let mut created = Vec::new();
    make_parents(&root, &path, &mut created)?;
    let receipt = Receipt {
        schema: "archgraph/apply-receipt/v1".into(),
        finding_id: id.into(),
        before: serde_json::to_value(before)?,
        recipe: recipe.clone(),
        fingerprints: BTreeMap::new(),
    };
    write_state(&path, &serde_json::to_vec_pretty(&receipt)?)
}
/// Numeric deltas require a saved before-scan; current-only predicates need no receipt.
pub fn verify_predicate(
    root: &Path,
    id: &str,
    after: &Document,
    predicate: &crate::recipes::Predicate,
) -> Result<VerificationResult> {
    valid_id(id)?;
    ensure!(
        predicate.baseline_id.as_deref() == Some(id),
        "measured verification requires baseline_id from propose: call propose before editing, then pass its structured predicate"
    );
    let root = root.canonicalize()?;
    let applied = receipt_path(&root, id)?;
    let proposed = checked_path(&root, &format!(".archgraph/proposed/{id}.json"))?;
    let mut candidate = None;
    for path in [applied, proposed] {
        if !path.exists() {
            continue;
        }
        let receipt = read_receipt(&root, &path, id)?;
        if predicate.expected.as_ref().is_none_or(|expected| {
            serde_json::to_value(expected).ok()
                == serde_json::to_value(&receipt.recipe.expected).ok()
        }) {
            candidate = Some(receipt);
            break;
        }
    }
    let receipt=candidate.context("no matching recipe baseline: call propose again and pass its complete structured predicate, including expected")?;
    let before = crate::restore_scan(serde_json::from_value(receipt.before)?)?;
    ensure!(
        serde_json::to_value(&before.config)? == serde_json::to_value(&after.config)?,
        "scan configuration or layout rules changed since propose: regenerate the proposal before comparing deltas"
    );
    let mut recipe = receipt.recipe;
    recipe.verify.structured = predicate.clone();
    Ok(crate::recipes::check(&before, after, &recipe))
}
