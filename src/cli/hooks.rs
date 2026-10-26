//! Persisted pre-edit baselines; an unresolved block remains debt through repair edits.
use super::*;
use anyhow::ensure;
use boundrail::hook_agents::{self, Agent, Event};
use boundrail::{edit_audit, edit_gate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

#[derive(Args)]
pub(super) struct Hook {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Install project-local agent hooks; prints the files and capabilities written
    Install(Install),
    /// Native hook protocol; reads one JSON payload from stdin
    Run(Native),
    /// Capture a pre-edit graph for scripted or MCP checks
    Capture(Capture),
    /// Compare current sources with an immutable pre-edit snapshot
    Check(Check),
    /// Release one crashed transaction, recording the reason
    Reset(Reset),
}
#[derive(Args, Clone, Serialize, Deserialize)]
struct Setup {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    language: Option<String>,
    #[arg(long)]
    package: Option<String>,
    #[arg(long)]
    depth: Option<usize>,
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Allowed repository-relative paths/globs (repeatable)
    #[arg(long)]
    scope: Vec<String>,
}
#[derive(Args)]
struct Install {
    #[command(flatten)]
    setup: Setup,
    #[arg(long, value_enum)]
    agent: Agent,
    /// Explicitly accept feedback-only hosts that cannot enforce completion
    #[arg(long)]
    feedback_only: bool,
}
#[derive(Args)]
struct Native {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, value_enum)]
    agent: Agent,
    #[arg(long, value_enum)]
    event: Event,
    #[arg(long)]
    feedback_only: bool,
}
#[derive(Args)]
struct Capture {
    #[command(flatten)]
    setup: Setup,
    #[arg(long, value_enum)]
    agent: Agent,
    #[arg(long, default_value = "manual")]
    session: String,
    #[arg(long, default_value = "manual")]
    tool_id: String,
    #[arg(long)]
    files: Vec<String>,
}
#[derive(Args)]
struct Check {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    baseline: String,
    #[arg(long)]
    allow_once: Vec<String>,
    #[arg(long)]
    reason: Option<String>,
}
#[derive(Args)]
struct Reset {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    baseline: String,
    #[arg(long)]
    reason: String,
}
#[derive(Args)]
pub(super) struct Fires {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Count blocks by rule, most frequent first
    #[arg(long)]
    top: bool,
}
#[derive(Serialize, Deserialize)]
struct Installed {
    setup: Setup,
    feedback_only: bool,
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    schema: String,
    root: PathBuf,
    agent: Agent,
    session: String,
    setup: Setup,
    policy: String,
    files: BTreeMap<String, String>,
    graph_hash: String,
}
#[derive(Serialize, Deserialize)]
struct Session {
    baseline: String,
    pending: bool,
}
#[derive(Serialize, Deserialize)]
struct Lease {
    session: String,
    tool: String,
    baseline: String,
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Cursor => "cursor",
        Agent::Opencode => "opencode",
    }
}
fn identity(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)),
        "unsafe correlation identity"
    );
    Ok(())
}
fn session_key(agent: Agent, session: &str) -> String {
    hash(format!("{}:{session}", agent_name(agent)).as_bytes())
}
fn token(id: &str) -> Result<()> {
    ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid snapshot ID"
    );
    Ok(())
}
fn checked(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        ensure!(
            matches!(part, std::path::Component::Normal(_)),
            "unsafe state path"
        );
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(
                !meta.is_symlink() && (meta.is_dir() || meta.is_file()),
                "unsafe hook state path: {}",
                path.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}
fn directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        path.push(part);
        checked(
            root,
            path.strip_prefix(root)?
                .to_str()
                .context("non-UTF8 state path")?,
        )?;
        if !path.exists() {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        ensure!(
            fs::symlink_metadata(&path)?.is_dir(),
            "hook state parent is not a directory"
        );
    }
    Ok(path)
}
fn save(root: &Path, relative: &str, value: &impl Serialize) -> Result<()> {
    let path = checked(root, relative)?;
    let temp = checked(root, &format!("{relative}.tmp"))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).with_context(|| {
        format!(
            "creating {} (remove only after confirming no hook is running)",
            temp.display()
        )
    })?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(&temp, &path)?;
    Ok(())
}
fn load<T: serde::de::DeserializeOwned>(root: &Path, relative: &str) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(checked(
        root, relative,
    )?)?)?)
}
fn optional<T: serde::de::DeserializeOwned>(root: &Path, relative: &str) -> Result<Option<T>> {
    if !checked(root, relative)?.exists() {
        return Ok(None);
    }
    load(root, relative).map(Some)
}
fn remove(root: &Path, relative: &str) -> Result<()> {
    let path = checked(root, relative)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn locked(root: &Path) -> Result<File> {
    directory(root, ".archgraph/hooks/snapshots")?;
    directory(root, ".archgraph/hooks/sessions")?;
    directory(root, ".archgraph/hooks/configs")?;
    let path = checked(root, ".archgraph/hooks/lock")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.lock()?;
    Ok(file)
}
fn canonical(mut setup: Setup) -> Result<Setup> {
    setup.root = setup.root.canonicalize()?;
    if setup.rules.is_none() && setup.root.join("layout.yaml").is_file() {
        setup.rules = Some(setup.root.join("layout.yaml"));
    }
    if let Some(path) = &mut setup.rules {
        if path.is_relative() {
            *path = setup.root.join(&*path)
        }
        *path = path.canonicalize()?;
    }
    collect::globset(&setup.scope)?;
    for scope in &setup.scope {
        relative(scope)?;
    }
    Ok(setup)
}
fn relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.contains(['\\', ':'])
            && !path.chars().any(char::is_control)
            && Path::new(path)
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_)))
            && path.split('/').all(|p| !matches!(p, "" | "." | "..")),
        "expected a repository-relative path: {path:?}"
    );
    Ok(())
}
fn subjects(root: &Path, files: &[String]) -> Result<Vec<String>> {
    files
        .iter()
        .map(|file| {
            let path = Path::new(file);
            let rel = if path.is_absolute() {
                path.strip_prefix(root)
                    .context("edit path is outside the repository")?
                    .to_str()
                    .context("non-UTF8 edit path")?
            } else {
                file
            };
            relative(rel)?;
            ensure!(
                !omitted(Path::new(rel)),
                "edit path is outside the snapshot inventory: {rel}"
            );
            let mut ancestor = root.to_path_buf();
            for component in Path::new(rel).components() {
                ancestor.push(component);
                match fs::symlink_metadata(&ancestor) {
                    Ok(meta) => ensure!(
                        !meta.is_symlink() || !ancestor.is_dir(),
                        "explicit edits through a directory symlink are unsupported: {rel}; use its physical path"
                    ),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) => return Err(e.into()),
                }
            }
            let mut ancestor = root.join(rel);
            while !ancestor.exists() {
                ensure!(ancestor.pop(), "cannot resolve edit path");
            }
            ensure!(
                ancestor.canonicalize()?.starts_with(root),
                "edit follows a symlink outside the repository"
            );
            Ok(rel.to_owned())
        })
        .collect()
}
fn omitted(path: &Path) -> bool {
    path.components().any(|p| {
        matches!(
            p.as_os_str().to_str(),
            Some(".git" | ".archgraph" | "node_modules" | ".venv" | "venv" | "__pycache__")
        )
    })
}
fn build_outputs(setup: &Setup) -> Result<globset::GlobSet> {
    let language = match &setup.language {
        Some(language) => serde_json::from_value(json!(language))?,
        None => collect::detect_language(&setup.root, collect::source_dir(&setup.root))?,
    };
    // An explicit include can restore generated source; retain the broader inventory then.
    let includes = setup
        .rules
        .as_deref()
        .map(rules::load)
        .transpose()?
        .is_some_and(|rules| !rules.doc.include.is_empty());
    collect::globset(&if includes {
        vec![]
    } else {
        collect::excludes::GENERATED_EXCLUDES
            .iter()
            .map(|pattern| (*pattern).to_owned())
            .chain(collect::excludes::output_globs(language))
            .collect()
    })
}
fn manifest(root: &Path, outputs: &globset::GlobSet) -> Result<BTreeMap<String, String>> {
    fn walk(
        root: &Path,
        dir: &Path,
        files: &mut BTreeSet<PathBuf>,
        outputs: &globset::GlobSet,
    ) -> Result<()> {
        for item in fs::read_dir(dir)? {
            let item = item?;
            let path = item.path();
            let rel = path.strip_prefix(root)?;
            if omitted(rel)
                || outputs.is_match(rel)
                || outputs.is_match(rel.join("__archgraph_build_output__"))
            {
                continue;
            }
            let meta = fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                walk(root, &path, files, outputs)?
            } else {
                files.insert(rel.to_path_buf());
            }
        }
        Ok(())
    }
    let mut paths = BTreeSet::new();
    // Native collection can include gitignored sources; Git's visible file list is insufficient.
    walk(root, root, &mut paths, outputs)?;
    let mut out = BTreeMap::new();
    for path in paths {
        let key = path.to_str().context("non-UTF8 source path")?;
        relative(key)?;
        let full = root.join(&path);
        let meta = match fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.is_symlink() {
            let target = full.canonicalize().context("broken source symlink")?;
            ensure!(
                target.starts_with(root),
                "source symlink escapes repository: {key}"
            );
            if target.is_dir() {
                ensure!(
                    !full
                        .parent()
                        .is_some_and(|parent| parent.starts_with(&target)),
                    "cyclic directory symlink in snapshot inventory: {key}"
                );
                let link = fs::read_link(&full)?;
                let identity = (
                    link.to_str().context("non-UTF8 directory symlink target")?,
                    target
                        .strip_prefix(root)?
                        .to_str()
                        .context("non-UTF8 directory symlink destination")?,
                );
                // Alias contents are already inventoried at physical paths; never walk the alias.
                out.insert(
                    key.to_owned(),
                    format!(
                        "directory-symlink:{}",
                        hash(&serde_json::to_vec(&identity)?)
                    ),
                );
            } else {
                ensure!(target.is_file(), "non-regular source symlink target: {key}");
                out.insert(key.to_owned(), hash(&fs::read(target)?));
            }
        } else if meta.is_file() {
            out.insert(key.to_owned(), hash(&fs::read(full)?));
        }
    }
    Ok(out)
}
fn policy(setup: &Setup) -> Result<String> {
    let rules = if let Some(path) = &setup.rules {
        Some(fs::read(path)?)
    } else {
        match fs::read(setup.root.join("layout.yaml")) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        }
    };
    Ok(hash(&serde_json::to_vec(&(setup, rules))?))
}
fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|p| before.get(*p) != after.get(*p))
        .cloned()
        .collect()
}
fn scan(setup: &Setup) -> Result<(Document, BTreeMap<String, String>)> {
    let outputs = build_outputs(setup)?;
    for _ in 0..2 {
        let before = manifest(&setup.root, &outputs)?;
        let graph_relative = format!(
            ".archgraph/hooks/snapshots/{}.scan.json",
            daemon_cli::token()?
        );
        let graph_path = checked(&setup.root, &graph_relative)?;
        let mut args = vec![
            "scan".to_owned(),
            setup.root.to_string_lossy().into_owned(),
            "--engine".into(),
            "native".into(),
            "--symbols".into(),
            "--output".into(),
            graph_path.to_string_lossy().into_owned(),
        ];
        if let Some(language) = &setup.language {
            args.extend(["--language".into(), language.clone()]);
        }
        if let Some(package) = &setup.package {
            args.extend(["--package".into(), package.clone()]);
        }
        if let Some(depth) = setup.depth {
            args.extend(["--depth".into(), depth.to_string()]);
        }
        if let Some(rules) = &setup.rules {
            args.extend(["--rules".into(), rules.to_string_lossy().into_owned()]);
        }
        // Request-local captured output keeps native hook stdout valid JSON.
        let capture = boundrail::output::Capture::begin();
        // A reasoned bypass changes the caller environment; collect independently of daemon trust.
        let response = if globals().no_daemon || std::env::var_os("ARCHGRAPH_BYPASS").is_some() {
            None
        } else {
            daemon_cli::forward_args(&setup.root, args.clone(), false)
                .context("daemon snapshot failed; correct it or explicitly select --no-daemon")?
        };
        let result = (|| -> Result<Document> {
            if let Some(output) = response {
                ensure!(output.code == 0, "daemon scan failed: {}", output.stderr);
                boundrail::restore_scan(read_scan(&graph_path)?)
            } else {
                let cli = Cli::try_parse_from(std::iter::once("boundrail".to_owned()).chain(args))?;
                let Cmd::Scan(args) = cli.command else {
                    unreachable!()
                };
                args.document()
            }
        })();
        let output = capture.finish(if result.is_ok() { 0 } else { 2 });
        remove(&setup.root, &graph_relative)?;
        let doc = result.context(output.stderr)?;
        let after = manifest(&setup.root, &outputs)?;
        if before == after {
            return Ok((doc, after));
        }
    }
    bail!("sources changed during both snapshot scans; retry after concurrent writes stop")
}
fn snapshot(root: &Path, id: &str) -> Result<(Snapshot, Document)> {
    token(id)?;
    let state: Snapshot = load(root, &format!(".archgraph/hooks/snapshots/{id}.json"))?;
    ensure!(
        state.schema == "archgraph/edit-snapshot/v1" && state.root == root,
        "snapshot belongs to a different repository or schema"
    );
    ensure!(
        state.setup.root == root && state.policy == policy(&state.setup)?,
        "architecture policy changed since the pre-edit snapshot; restore it or explicitly reset with a reason"
    );
    let path = checked(root, &format!(".archgraph/hooks/snapshots/{id}.graph.json"))?;
    ensure!(
        hash(&fs::read(&path)?) == state.graph_hash,
        "snapshot graph changed"
    );
    Ok((state, boundrail::restore_scan(read_scan(&path)?)?))
}
fn capture(setup: &Setup, agent: Agent, session: &str) -> Result<(String, Document)> {
    let policy_before = policy(setup)?;
    let (doc, files) = scan(setup)?;
    ensure!(
        policy_before == policy(setup)?,
        "policy changed during capture"
    );
    let bytes = serde_json::to_vec(&doc)?;
    let id = daemon_cli::token()?;
    let state = Snapshot {
        schema: "archgraph/edit-snapshot/v1".into(),
        root: setup.root.clone(),
        agent,
        session: session.into(),
        setup: setup.clone(),
        policy: policy_before,
        files,
        graph_hash: hash(&bytes),
    };
    // The private parent and atomic JSON writer keep baseline artifacts out of source control.
    save(
        &setup.root,
        &format!(".archgraph/hooks/snapshots/{id}.graph.json"),
        &doc,
    )?;
    save(
        &setup.root,
        &format!(".archgraph/hooks/snapshots/{id}.json"),
        &state,
    )?;
    Ok((id, doc))
}
fn compare(root: &Path, id: &str) -> Result<(Snapshot, edit_gate::Report, Document)> {
    let (state, before) = snapshot(root, id)?;
    let (after, files) = scan(&state.setup)?;
    ensure!(
        state.policy == policy(&state.setup)?,
        "policy changed during check"
    );
    let report = edit_gate::check(
        &before,
        &after,
        &edit_gate::Options {
            scope: state.setup.scope.clone(),
            changed_files: changed(&state.files, &files),
        },
    )?;
    Ok((state, report, after))
}
pub(super) fn snapshot_check(root: &Path, id: &str) -> Result<Value> {
    let root = root.canonicalize()?;
    // Immutable snapshots need no transaction lock; a hook may be awaiting this daemon.
    let (state, before) = snapshot(&root, id)?;
    let (after, files) = scan(&state.setup)?;
    ensure!(
        state.policy == policy(&state.setup)?,
        "policy changed during check"
    );
    let report = edit_gate::check(
        &before,
        &after,
        &edit_gate::Options {
            scope: state.setup.scope.clone(),
            changed_files: changed(&state.files, &files),
        },
    )?;
    let mut value =
        boundrail::mcp_tools::call(&after, &root, "check_diff", json!({}), Some(&before))?;
    for (key, item) in serde_json::to_value(report)?
        .as_object()
        .context("invalid gate report")?
    {
        value[key] = item.clone();
    }
    value["snapshot"] = json!(id);
    value["base_selection"] = json!("pre-edit snapshot");
    Ok(value)
}
fn enforce(
    root: &Path,
    id: &str,
    allow: &[String],
    reason: Option<&str>,
) -> Result<(edit_gate::Report, Document)> {
    let (state, mut report, doc) = compare(root, id)?;
    let env = std::env::var("ARCHGRAPH_BYPASS").ok();
    let mut accepted = BTreeSet::new();
    if let Some(reason) = env.as_deref() {
        ensure!(
            !reason.trim().is_empty(),
            "ARCHGRAPH_BYPASS requires a reason"
        );
        accepted.extend(report.violations.iter().map(|v| v.finding_id.clone()));
        edit_audit::override_event(
            root,
            agent_name(state.agent),
            id,
            reason,
            &accepted.iter().cloned().collect::<Vec<_>>(),
        )?;
    }
    if !allow.is_empty() {
        let reason = reason
            .filter(|s| !s.trim().is_empty())
            .context("--allow-once requires --reason")?;
        let used_path = format!(".archgraph/hooks/snapshots/{id}.allowed.json");
        let mut used: BTreeSet<String> = optional(root, &used_path)?.unwrap_or_default();
        for finding in allow {
            ensure!(
                !used.contains(finding),
                "finding was already allowed once: {finding}"
            );
            ensure!(
                report.violations.iter().any(|v| &v.finding_id == finding),
                "finding is not in this check: {finding}"
            );
        }
        edit_audit::override_event(root, agent_name(state.agent), id, reason, allow)?;
        used.extend(allow.iter().cloned());
        save(root, &used_path, &used)?;
        accepted.extend(used);
    } else {
        accepted.extend(
            optional::<BTreeSet<String>>(
                root,
                &format!(".archgraph/hooks/snapshots/{id}.allowed.json"),
            )?
            .unwrap_or_default(),
        );
    }
    report
        .violations
        .retain(|v| !accepted.contains(&v.finding_id));
    report.verdict = if report.violations.is_empty() {
        "PASS"
    } else {
        "BLOCK"
    }
    .into();
    if report.verdict == "BLOCK" {
        for violation in &mut report.violations {
            if let Ok(recipe) = boundrail::recipes::recipe_for(&doc, &violation.finding_id) {
                let mut recipe = recipe.clone();
                recipe.verify.structured.baseline_id = Some(violation.finding_id.clone());
                boundrail::apply::remember_proposal(root, &doc, &violation.finding_id, &recipe)?;
                violation.recipe = serde_json::to_value(recipe)?;
            }
        }
        edit_audit::fires(root, agent_name(state.agent), id, &report.violations)?;
    }
    Ok((report, doc))
}
fn message(report: &edit_gate::Report) -> Result<String> {
    Ok(format!(
        "{}\n{}",
        report.verdict,
        serde_json::to_string(report)?
    ))
}
fn state_path(key: &str) -> String {
    format!(".archgraph/hooks/sessions/{key}.json")
}
fn readonly(tool: &str) -> bool {
    matches!(
        tool,
        "Read"
            | "read"
            | "read_file"
            | "Glob"
            | "glob"
            | "Grep"
            | "grep"
            | "WebSearch"
            | "web_search"
            | "WebFetch"
            | "web_fetch"
    )
}
const LEASE: &str = ".archgraph/hooks/active-edit.json";
fn checkpoint(root: &Path, key: &str, baseline: &str) -> Result<()> {
    save(
        root,
        &state_path(key),
        &Session {
            baseline: baseline.into(),
            pending: false,
        },
    )
}
fn pre(root: &Path, setup: &Setup, agent: Agent, call: &hook_agents::Call) -> Result<String> {
    let tool = call.tool_id.as_ref().context("pre-edit tool ID missing")?;
    let key = session_key(agent, &call.session);
    if let Some(lease) = optional::<Lease>(root, LEASE)? {
        bail!(
            "edit snapshot {} is active; run its matching Post/Stop hook or hook reset --baseline {} --reason <reason>",
            lease.baseline,
            lease.baseline
        );
    }
    let existing: Option<Session> = optional(root, &state_path(&key))?;
    let files = subjects(root, &call.files)?;
    let outputs = build_outputs(setup)?;
    for file in &files {
        ensure!(
            !outputs.is_match(file),
            "edit path is generated build output outside the snapshot inventory: {file}"
        );
    }
    let (baseline, doc) = if let Some(existing) = existing.filter(|s| s.pending) {
        let (state, _) = snapshot(root, &existing.baseline)?;
        ensure!(
            state.agent == agent && state.session == call.session,
            "snapshot session mismatch"
        );
        ensure!(
            state.policy == policy(setup)?,
            "installed configuration changed during a pending edit"
        );
        let (doc, _) = scan(setup)?;
        (existing.baseline, doc)
    } else {
        capture(setup, agent, &call.session)?
    };
    let context = if files.is_empty() {
        json!({"summary":"Architecture baseline captured. All source changes are checked after this tool; repair new violations before finishing."})
    } else {
        explain_cli::for_agent(&doc, &files)?
    };
    let mut text = format!(
        "snapshot={baseline}\n{}",
        context["summary"]
            .as_str()
            .unwrap_or("Run the post-edit check before finishing.")
    );
    while text.len() > 400 {
        text.pop();
    }
    save(
        root,
        &state_path(&key),
        &Session {
            baseline: baseline.clone(),
            pending: true,
        },
    )?;
    save(
        root,
        LEASE,
        &Lease {
            session: key,
            tool: tool.clone(),
            baseline: baseline.clone(),
        },
    )?;
    Ok(text)
}
fn native(args: Native) -> Result<ExitCode> {
    let mut failure_event = false;
    let outcome = (|| -> Result<(bool, String, String)> {
        let mut body = String::new();
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_string(&mut body)?;
        ensure!(body.len() <= 1024 * 1024, "hook payload exceeds 1 MiB");
        let input: Value = serde_json::from_str(&body)?;
        failure_event = input["hook_event_name"] == "PostToolUseFailure";
        let call = hook_agents::decode(args.agent, args.event, &input)?;
        if args.event != Event::Stop && readonly(&call.tool) {
            return Ok((false, String::new(), String::new()));
        }
        let root = args.root.canonicalize()?;
        let _lock = locked(&root)?;
        let config: Installed = load(
            &root,
            &format!(".archgraph/hooks/configs/{}.json", agent_name(args.agent)),
        )?;
        ensure!(
            config.feedback_only == args.feedback_only,
            "hook capability mode differs from installed config"
        );
        ensure!(
            config.setup.root == root,
            "installed configuration belongs to another repository"
        );
        let key = session_key(args.agent, &call.session);
        match args.event {
            Event::Pre => Ok((
                false,
                String::new(),
                pre(&root, &canonical(config.setup.clone())?, args.agent, &call)?,
            )),
            Event::Post => {
                let lease: Lease = optional(&root, LEASE)?
                    .context("no matching pre-edit snapshot; edit result is unverified")?;
                ensure!(
                    lease.session == key && call.tool_id.as_ref() == Some(&lease.tool),
                    "post-edit session/tool does not match active snapshot"
                );
                let (report, _) = enforce(&root, &lease.baseline, &[], None)?;
                // Retain the original baseline through BLOCK; the next edit is a repair attempt.
                if report.verdict == "PASS" {
                    checkpoint(&root, &key, &lease.baseline)?;
                }
                remove(&root, LEASE)?;
                Ok((report.verdict == "BLOCK", message(&report)?, String::new()))
            }
            Event::Stop => {
                let lease = optional::<Lease>(&root, LEASE)?;
                if let Some(lease) = &lease {
                    ensure!(
                        lease.session == key,
                        "another session owns an active edit snapshot; complete that session or reset its exact baseline with a reason"
                    );
                }
                let Some(session) = optional::<Session>(&root, &state_path(&key))? else {
                    ensure!(
                        lease.is_none(),
                        "active edit is missing its session baseline"
                    );
                    return Ok((false, "No edit transaction recorded.".into(), String::new()));
                };
                if let Some(lease) = &lease {
                    ensure!(
                        lease.baseline == session.baseline,
                        "active edit and session baseline differ"
                    );
                }
                let (report, _) = enforce(&root, &session.baseline, &[], None)?;
                if report.verdict == "PASS" {
                    remove(&root, &state_path(&key))?;
                } else {
                    save(
                        &root,
                        &state_path(&key),
                        &Session {
                            baseline: session.baseline,
                            pending: true,
                        },
                    )?;
                }
                if lease.is_some() {
                    remove(&root, LEASE)?;
                }
                Ok((report.verdict == "BLOCK", message(&report)?, String::new()))
            }
        }
    })();
    let (blocked, message, context) = match outcome {
        Ok(out) => out,
        Err(e) => (true, format!("UNAVAILABLE\n{e:#}"), String::new()),
    };
    let mut response = hook_agents::response(args.agent, args.event, blocked, &message, &context)?;
    if args.agent == Agent::Claude && args.event == Event::Post && failure_event {
        response["hookSpecificOutput"]["hookEventName"] = json!("PostToolUseFailure");
        response
            .as_object_mut()
            .context("invalid native response")?
            .remove("decision");
        response
            .as_object_mut()
            .context("invalid native response")?
            .remove("reason");
    }
    println!("{}", serde_json::to_string(&response)?);
    Ok(ExitCode::SUCCESS)
}
pub(super) fn run(args: Hook) -> Result<ExitCode> {
    match args.command {
        Action::Run(args) => native(args),
        Action::Install(args) => {
            ensure!(
                args.feedback_only || matches!(args.agent, Agent::Claude | Agent::Codex),
                "this host does not provide verified strict completion blocking; use --feedback-only explicitly"
            );
            let setup = canonical(args.setup)?;
            let _lock = locked(&setup.root)?;
            let mut value = hook_agents::install(
                &setup.root,
                args.agent,
                &std::env::current_exe()?,
                args.feedback_only,
            )?;
            save(
                &setup.root,
                &format!(".archgraph/hooks/configs/{}.json", agent_name(args.agent)),
                &Installed {
                    setup: setup.clone(),
                    feedback_only: args.feedback_only,
                },
            )?;
            value["configuration"] = json!(setup.root.join(format!(
                ".archgraph/hooks/configs/{}.json",
                agent_name(args.agent)
            )));
            write(None, &value)?;
            Ok(ExitCode::SUCCESS)
        }
        Action::Capture(args) => {
            identity(&args.session)?;
            identity(&args.tool_id)?;
            let setup = canonical(args.setup)?;
            let _lock = locked(&setup.root)?;
            let call = hook_agents::Call {
                session: args.session,
                tool_id: Some(args.tool_id),
                files: args.files,
                tool: "manual".into(),
                failed: false,
            };
            let context = pre(&setup.root, &setup, args.agent, &call)?;
            let lease: Lease = load(&setup.root, LEASE)?;
            write(None, &json!({"snapshot":lease.baseline,"context":context}))?;
            Ok(ExitCode::SUCCESS)
        }
        Action::Check(args) => {
            let outcome = (|| -> Result<ExitCode> {
                let root = args.root.canonicalize()?;
                let _lock = locked(&root)?;
                token(&args.baseline)?;
                let lease = optional::<Lease>(&root, LEASE)?;
                if let Some(lease) = &lease {
                    ensure!(
                        lease.baseline == args.baseline,
                        "another baseline owns the active edit"
                    );
                }
                let (report, _) = enforce(
                    &root,
                    &args.baseline,
                    &args.allow_once,
                    args.reason.as_deref(),
                )?;
                if let Some(lease) = lease {
                    if report.verdict == "PASS" {
                        checkpoint(&root, &lease.session, &lease.baseline)?;
                    }
                    remove(&root, LEASE)?;
                }
                if globals().brief {
                    println!("{}", message(&report)?);
                } else {
                    write(None, &report)?;
                }
                Ok(if report.verdict == "PASS" {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                })
            })();
            match outcome {
                Err(error) if globals().brief => {
                    println!(
                        "UNAVAILABLE\n{}",
                        json!({"schema":"archgraph/edit-check/v1","verdict":"UNAVAILABLE","violations":[],"error":format!("{error:#}")})
                    );
                    Ok(ExitCode::from(2))
                }
                result => result,
            }
        }
        Action::Reset(args) => {
            let root = args.root.canonicalize()?;
            let _lock = locked(&root)?;
            token(&args.baseline)?;
            ensure!(!args.reason.trim().is_empty(), "reset requires a reason");
            let state: Snapshot = load(
                &root,
                &format!(".archgraph/hooks/snapshots/{}.json", args.baseline),
            )?;
            ensure!(state.root == root, "snapshot belongs to another repository");
            let key = session_key(state.agent, &state.session);
            let session: Session = load(&root, &state_path(&key))?;
            ensure!(
                session.baseline == args.baseline,
                "snapshot no longer owns this session"
            );
            if let Some(lease) = optional::<Lease>(&root, LEASE)? {
                ensure!(
                    lease.baseline == args.baseline,
                    "another edit owns the lease"
                );
            }
            edit_audit::override_event(
                &root,
                agent_name(state.agent),
                &args.baseline,
                &args.reason,
                &[],
            )?;
            remove(&root, &state_path(&key))?;
            remove(&root, LEASE)?;
            write(
                None,
                &json!({"verdict":"RESET","snapshot":args.baseline,"reason":args.reason}),
            )?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
pub(super) fn fires(args: Fires) -> Result<ExitCode> {
    ensure!(args.top, "use fires --top to count rule blocks");
    write(None, &edit_audit::top(&args.root.canonicalize()?)?)?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "archgraph-edit-runtime-{}",
                daemon_cli::token().unwrap()
            ));
            fs::create_dir_all(root.join("src/domain")).unwrap();
            fs::create_dir_all(root.join("src/adapters")).unwrap();
            fs::write(root.join("src/domain/a.ts"), "export const a = 1;\n").unwrap();
            fs::write(root.join("src/adapters/b.ts"), "export const b = 2;\n").unwrap();
            fs::write(root.join("layout.yaml"), "version: 1\nmodules:\n  - {id: domain, path: 'src/domain/**'}\n  - {id: adapters, path: 'src/adapters/**'}\nforbid:\n  - {id: boundary, from: 'src/domain/**', to: 'src/adapters/**', reason: 'Keep domain independent'}\n").unwrap();
            GLOBALS.with(|g| g.borrow_mut().no_daemon = true);
            Self(root.canonicalize().unwrap())
        }
        fn setup(&self) -> Setup {
            canonical(Setup {
                root: self.0.clone(),
                language: Some("typescript".into()),
                package: None,
                depth: None,
                rules: None,
                scope: vec![],
            })
            .unwrap()
        }
        fn call(&self, tool: &str) -> hook_agents::Call {
            hook_agents::Call {
                session: "fixture".into(),
                tool_id: Some(tool.into()),
                files: vec!["src/domain/a.ts".into()],
                tool: "Edit".into(),
                failed: false,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn blocked_debt_survives_repair_attempts_and_parallel_pre_is_rejected() {
        let fixture = Fixture::new();
        let setup = fixture.setup();
        let _lock = locked(&fixture.0).unwrap();
        pre(&fixture.0, &setup, Agent::Claude, &fixture.call("first")).unwrap();
        assert!(
            pre(
                &fixture.0,
                &setup,
                Agent::Claude,
                &fixture.call("concurrent")
            )
            .is_err()
        );
        let original: Lease = load(&fixture.0, LEASE).unwrap();
        fs::write(
            fixture.0.join("src/domain/a.ts"),
            "import {b} from '../adapters/b'; export const a=b;\n",
        )
        .unwrap();
        let (_, report, _) = compare(&fixture.0, &original.baseline).unwrap();
        assert_eq!(report.verdict, "BLOCK");
        assert!(!report.violations.is_empty());
        remove(&fixture.0, LEASE).unwrap();
        pre(&fixture.0, &setup, Agent::Claude, &fixture.call("repair")).unwrap();
        let repair: Lease = load(&fixture.0, LEASE).unwrap();
        assert_eq!(repair.baseline, original.baseline);
        assert_eq!(
            compare(&fixture.0, &repair.baseline).unwrap().1.verdict,
            "BLOCK"
        );
        fs::write(fixture.0.join("src/domain/a.ts"), "export const a=1;\n").unwrap();
        assert_eq!(
            compare(&fixture.0, &repair.baseline).unwrap().1.verdict,
            "PASS"
        );
    }

    #[test]
    fn preexisting_problem_passes_and_policy_changes_fail_closed() {
        let fixture = Fixture::new();
        let setup = fixture.setup();
        let _lock = locked(&fixture.0).unwrap();
        fs::write(
            fixture.0.join("src/domain/a.ts"),
            "import {b} from '../adapters/b'; export const a=b;\n",
        )
        .unwrap();
        let (id, _) = capture(&setup, Agent::Codex, "fixture").unwrap();
        assert_eq!(compare(&fixture.0, &id).unwrap().1.verdict, "PASS");
        fs::write(fixture.0.join("layout.yaml"), "version: 1\n").unwrap();
        assert!(
            compare(&fixture.0, &id)
                .err()
                .unwrap()
                .to_string()
                .contains("policy changed")
        );
        assert!(snapshot(&fixture.0, &"a".repeat(64)).is_err());
    }

    #[test]
    fn empty_tool_paths_receive_context_without_orphaning_a_lease() {
        let fixture = Fixture::new();
        let setup = fixture.setup();
        let _lock = locked(&fixture.0).unwrap();
        let mut call = fixture.call("shell");
        call.files.clear();
        let context = pre(&fixture.0, &setup, Agent::Claude, &call).unwrap();
        assert!(context.len() <= 400);
        let lease: Lease = load(&fixture.0, LEASE).unwrap();
        assert_eq!(lease.tool, "shell");
        assert_eq!(
            compare(&fixture.0, &lease.baseline).unwrap().1.verdict,
            "PASS"
        );
    }

    #[test]
    fn scope_checks_deleted_files_and_allow_once_requires_a_reason_and_logs() {
        let fixture = Fixture::new();
        let mut setup = fixture.setup();
        setup.scope = vec!["src/domain/**".into()];
        let _lock = locked(&fixture.0).unwrap();
        let (id, _) = capture(&setup, Agent::Claude, "scope").unwrap();
        fs::remove_file(fixture.0.join("src/adapters/b.ts")).unwrap();
        let report = compare(&fixture.0, &id).unwrap().1;
        assert_eq!(report.verdict, "BLOCK");
        let finding = report
            .violations
            .iter()
            .find(|v| v.file == "src/adapters/b.ts")
            .unwrap()
            .finding_id
            .clone();
        assert!(enforce(&fixture.0, &id, std::slice::from_ref(&finding), None).is_err());
        let (blocked, _) = enforce(&fixture.0, &id, &[], None).unwrap();
        assert_eq!(blocked.verdict, "BLOCK");
        assert_eq!(edit_audit::top(&fixture.0).unwrap()["total"], 1);
        let (allowed, _) = enforce(
            &fixture.0,
            &id,
            std::slice::from_ref(&finding),
            Some("approved scope exception"),
        )
        .unwrap();
        assert_eq!(allowed.verdict, "PASS");
        assert!(enforce(&fixture.0, &id, &[finding], Some("repeat")).is_err());
        let records = fs::read_to_string(fixture.0.join(".archgraph/overrides.log")).unwrap();
        assert!(records.contains("approved scope exception"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_state_directory_is_refused() {
        let fixture = Fixture::new();
        std::os::unix::fs::symlink(std::env::temp_dir(), fixture.0.join(".archgraph")).unwrap();
        assert!(locked(&fixture.0).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn directory_alias_inventory_tracks_physical_contents_and_link_targets_once() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let alias = fixture.0.join("domain-alias");
        symlink("src/domain", &alias).unwrap();
        let outputs = build_outputs(&fixture.setup()).unwrap();
        let before = manifest(&fixture.0, &outputs).unwrap();
        assert!(before.contains_key("domain-alias"));
        assert!(before.contains_key("src/domain/a.ts"));
        assert!(!before.contains_key("domain-alias/a.ts"));
        fs::write(alias.join("a.ts"), "export const a = 3;\n").unwrap();
        let edited = manifest(&fixture.0, &outputs).unwrap();
        assert_eq!(changed(&before, &edited), vec!["src/domain/a.ts"]);
        fs::remove_file(&alias).unwrap();
        symlink("src/adapters", &alias).unwrap();
        let retargeted = manifest(&fixture.0, &outputs).unwrap();
        assert_eq!(changed(&edited, &retargeted), vec!["domain-alias"]);
        fs::remove_file(&alias).unwrap();
        symlink("./src/adapters", &alias).unwrap();
        assert_eq!(
            changed(&retargeted, &manifest(&fixture.0, &outputs).unwrap()),
            vec!["domain-alias"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_alias_explicit_edits_are_refused_before_scope_or_output_bypass() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let mut setup = fixture.setup();
        setup.scope = vec!["src/domain/**".into()];
        symlink("../adapters", fixture.0.join("src/domain/bridge")).unwrap();
        fs::create_dir_all(fixture.0.join("dist")).unwrap();
        symlink("../../dist", fixture.0.join("src/domain/output-bridge")).unwrap();
        for file in [
            "src/domain/bridge/b.ts",
            "src/domain/bridge/new.ts",
            "src/domain/output-bridge/new.ts",
        ] {
            let error = subjects(&fixture.0, &[file.into()]).unwrap_err();
            assert!(error.to_string().contains("directory symlink"), "{error:#}");
            let mut call = fixture.call("alias-edit");
            call.files = vec![file.into()];
            let error = pre(&fixture.0, &setup, Agent::Codex, &call).unwrap_err();
            assert!(error.to_string().contains("directory symlink"), "{error:#}");
            assert!(!fixture.0.join(LEASE).exists());
        }
        assert!(subjects(&fixture.0, &["src/domain/a.ts".into()]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn directory_alias_allows_canonical_edits_and_observes_alias_write_repair() {
        let fixture = Fixture::new();
        let mut setup = fixture.setup();
        setup.scope = vec!["src/domain/**".into()];
        std::os::unix::fs::symlink("src/domain", fixture.0.join("domain-alias")).unwrap();
        std::os::unix::fs::symlink("src/adapters", fixture.0.join("adapters-alias")).unwrap();
        let _lock = locked(&fixture.0).unwrap();
        pre(&fixture.0, &setup, Agent::Codex, &fixture.call("canonical")).unwrap();
        let lease: Lease = load(&fixture.0, LEASE).unwrap();
        fs::write(
            fixture.0.join("domain-alias/a.ts"),
            "import {b} from '../adapters/b'; export const a=b;\n",
        )
        .unwrap();
        let report = compare(&fixture.0, &lease.baseline).unwrap().1;
        assert_eq!(report.verdict, "BLOCK");
        assert!(
            report.violations.iter().any(
                |finding| finding.file == "src/domain/a.ts" && finding.kind == "forbidden-edge"
            )
        );
        assert!(
            !report
                .violations
                .iter()
                .any(|finding| finding.kind == "scope")
        );
        fs::write(fixture.0.join("domain-alias/a.ts"), "export const a = 2;\n").unwrap();
        assert_eq!(
            compare(&fixture.0, &lease.baseline).unwrap().1.verdict,
            "PASS"
        );
        fs::write(
            fixture.0.join("adapters-alias/b.ts"),
            "export const b = 3;\n",
        )
        .unwrap();
        let report = compare(&fixture.0, &lease.baseline).unwrap().1;
        assert_eq!(report.verdict, "BLOCK");
        assert!(
            report
                .violations
                .iter()
                .any(|finding| finding.kind == "scope" && finding.file == "src/adapters/b.ts")
        );
        assert!(
            !report
                .violations
                .iter()
                .any(|finding| finding.file.starts_with("adapters-alias/"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_alias_external_broken_and_cyclic_targets_still_fail_closed() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let link = fixture.0.join("unsafe-link");
        let outputs = build_outputs(&fixture.setup()).unwrap();
        for target in [
            outside.0.clone(),
            fixture.0.join("missing"),
            link.clone(),
            fixture.0.clone(),
        ] {
            symlink(target, &link).unwrap();
            assert!(manifest(&fixture.0, &outputs).is_err());
            fs::remove_file(&link).unwrap();
        }
        let other = fixture.0.join("other-link");
        symlink("other-link", &link).unwrap();
        symlink("unsafe-link", &other).unwrap();
        assert!(manifest(&fixture.0, &outputs).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn directory_alias_non_regular_link_target_is_refused() {
        let root = PathBuf::from("/tmp").join(format!(
            "ag-hook-socket-{}",
            &daemon_cli::token().unwrap()[..16]
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Fixture(root.canonicalize().unwrap());
        let _socket = std::os::unix::net::UnixListener::bind(fixture.0.join("socket")).unwrap();
        std::os::unix::fs::symlink("socket", fixture.0.join("socket-link")).unwrap();
        let error = manifest(&fixture.0, &collect::globset(&[]).unwrap()).unwrap_err();
        assert!(error.to_string().contains("non-regular"), "{error:#}");
    }

    #[test]
    fn inventory_tracks_ignored_and_nested_target_sources() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join("src/target")).unwrap();
        fs::write(fixture.0.join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(fixture.0.join("ignored.ts"), "export const hidden=1;\n").unwrap();
        fs::write(
            fixture.0.join("src/target/nested.ts"),
            "export const nested=1;\n",
        )
        .unwrap();
        let files = manifest(&fixture.0, &build_outputs(&fixture.setup()).unwrap()).unwrap();
        assert!(files.contains_key("ignored.ts"));
        assert!(files.contains_key("src/target/nested.ts"));
        assert!(subjects(&fixture.0, &["node_modules/private.ts".into()]).is_err());
    }
    #[test]
    fn rust_build_outputs_are_not_hashed_or_counted_as_scope_changes() {
        let fixture = Fixture::new();
        let mut setup = fixture.setup();
        setup.language = Some("rust".into());
        fs::create_dir_all(fixture.0.join("target/debug")).unwrap();
        let outputs = build_outputs(&setup).unwrap();
        let before = manifest(&fixture.0, &outputs).unwrap();
        fs::write(fixture.0.join("target/debug/artifact"), "build result").unwrap();
        assert_eq!(before, manifest(&fixture.0, &outputs).unwrap());
        assert!(outputs.is_match("target/debug/artifact"));
        assert!(outputs.is_match("dist/bundle.js"));
        fs::write(
            fixture.0.join("layout.yaml"),
            "version: 1\ninclude: ['target/source.rs']\n",
        )
        .unwrap();
        assert!(!build_outputs(&setup).unwrap().is_match("target/source.rs"));
    }
    #[test]
    fn changed_files_include_additions_deletions_and_modifications() {
        let before = BTreeMap::from([
            ("gone".into(), "a".into()),
            ("same".into(), "b".into()),
            ("edit".into(), "c".into()),
        ]);
        let after = BTreeMap::from([
            ("added".into(), "d".into()),
            ("same".into(), "b".into()),
            ("edit".into(), "e".into()),
        ]);
        assert_eq!(changed(&before, &after), vec!["added", "edit", "gone"]);
    }
    #[test]
    fn state_ids_and_relative_paths_cannot_escape() {
        for path in ["../x", "/x", "a/../x", "a//x", "C:x", "a\\x"] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert!(relative("src/new.ts").is_ok());
        assert!(token(&"a".repeat(64)).is_ok());
        assert!(token("../snapshot").is_err());
        assert_ne!(
            session_key(Agent::Claude, "x"),
            session_key(Agent::Codex, "x")
        );
    }
    #[test]
    fn only_known_readonly_tools_skip_edit_transactions() {
        for tool in [
            "Read",
            "read",
            "read_file",
            "Glob",
            "grep",
            "Grep",
            "WebSearch",
            "web_fetch",
        ] {
            assert!(readonly(tool), "{tool}");
        }
        for tool in [
            "Bash",
            "bash",
            "Edit",
            "Write",
            "apply_patch",
            "mcp__filesystem__read_file",
            "unknown",
            "",
        ] {
            assert!(!readonly(tool), "{tool}");
        }
    }
}
