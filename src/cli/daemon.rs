//! Local, serial command execution with atomic graph generations.
use super::*;
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const VERSION: u32 = 1;
const FRAME_LIMIT: usize = 16 * 1024 * 1024;
#[derive(Args, Clone)]
pub(super) struct Serve {
    #[arg(default_value = ".")]
    root: PathBuf,
    #[arg(long, conflicts_with = "port")]
    socket: Option<PathBuf>,
    #[arg(long, num_args=0..=1, default_missing_value="0")]
    port: Option<u16>,
    /// Warm, answer one request and exit without a watcher
    #[arg(long)]
    once: bool,
    #[command(flatten)]
    opts: ScanOpts,
    #[command(flatten)]
    hotspots: HotspotOpts,
    #[command(flatten)]
    collector: CollectorOpts,
}
#[derive(Args)]
pub(super) struct Status {
    #[arg(default_value = ".")]
    root: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
struct Endpoint {
    version: u32,
    binary: String,
    binary_metadata: String,
    root: PathBuf,
    socket: Option<PathBuf>,
    port: Option<u16>,
    token: String,
}
#[derive(Serialize, Deserialize)]
struct Request {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Params,
}
#[derive(Serialize, Deserialize)]
struct Params {
    token: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, Option<String>>,
    tty: bool,
}
#[derive(Serialize, Deserialize)]
struct RpcError {
    code: i32,
    message: String,
}
#[derive(Serialize, Deserialize)]
struct Response {
    jsonrpc: String,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<archgraph::output::Output>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}
#[derive(Clone)]
struct Active {
    root: PathBuf,
    graph: PathBuf,
    key: Vec<u8>,
    doc: Arc<Document>,
    stats: Value,
    dirty_proof: bool,
}
thread_local! { static CALLER_TTY: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
pub(super) fn caller_tty() -> Option<bool> {
    CALLER_TTY.with(|v| v.get())
}
thread_local! { static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) }; }
fn key(
    opts: &ScanOpts,
    hotspots: &HotspotOpts,
    collector: &CollectorOpts,
    rules: Option<&Path>,
) -> Vec<u8> {
    let mut options = serde_json::to_value(opts).expect("serializing scan options");
    options.as_object_mut().unwrap().remove("rules");
    let rule = rules.and_then(|p| p.canonicalize().ok());
    if let Some(path) = opts.collector_input.as_ref() {
        options["collector_input"] =
            serde_json::to_value(path.canonicalize().ok()).expect("serializing path");
    }
    let mut executables = serde_json::to_value(collector).expect("serializing collectors");
    for name in [
        "depcruise",
        "python",
        "go",
        "java",
        "elixir",
        "rust_analyzer",
    ] {
        if let Some(path) = executables[name]
            .as_str()
            .filter(|p| p.contains(['/', '\\']))
        {
            executables[name] = serde_json::to_value(Path::new(path).canonicalize().ok())
                .expect("serializing executable");
        }
    }
    serde_json::to_vec(&(
        options,
        hotspots,
        executables,
        rule,
        globals().all_kinds,
        globals().granularity,
    ))
    .expect("serializing scan options")
}
pub(super) fn cached_scan(
    root: &Path,
    opts: &ScanOpts,
    hotspots: &HotspotOpts,
    collector: &CollectorOpts,
    rules: Option<&Path>,
    outputs: &[&Path],
) -> Option<Document> {
    let mut doc = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .filter(|a| {
                root.canonicalize().ok().as_ref() == Some(&a.root)
                    && key(opts, hotspots, collector, rules) == a.key
            })
            .map(|a| (*a.doc).clone())
    })?;
    let proved = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.dirty_proof));
    if !proved
        || outputs.iter().any(|p| {
            std::path::absolute(p).ok().is_none_or(|p| {
                p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                    || root
                        .canonicalize()
                        .ok()
                        .is_none_or(|r| !p.starts_with(r.join(".archgraph")))
            })
        })
    {
        // Unknown/clean states need current whole-checkout provenance, including sibling files.
        doc.source.git = git(root, outputs);
    }
    if let (Some(saved), Some(path)) = (&mut doc.config.rules, rules) {
        saved.path = path.to_string_lossy().into_owned();
    }
    for limit in doc.source.collector.limitations.iter().take(10) {
        eprintln!("warning: {limit}");
    }
    if let Some(n) = doc.source.collector.unresolved.filter(|n| *n > 0) {
        if doc.imports.is_some() {
            eprintln!(
                "warning: {n} unresolved collector reference(s) reported by {}; explicit import accounting is reported separately",
                doc.source.collector.name
            );
        } else {
            eprintln!(
                "warning: {n} unresolved import(s) reported by {}; their edges are missing",
                doc.source.collector.name
            );
        }
    }
    if let Some(owners) = &doc.config.codeowners {
        for w in &owners.warnings {
            eprintln!("warning: {}:{}: {}", owners.path, w.line, w.message);
        }
    }
    Some(doc)
}

pub(super) fn cached_graph(path: &Path) -> Option<Document> {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .filter(|a| {
                path.canonicalize().ok().as_ref() == Some(&a.graph)
                    && !globals().all_kinds
                    && globals().granularity.is_none()
            })
            .map(|a| (*a.doc).clone())
    })
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes.as_ref())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn binary_metadata() -> Result<String> {
    let m = fs::metadata(std::env::current_exe()?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "{}:{}:{}:{}:{}",
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        Ok(format!(
            "{}:{:?}:{:?}",
            m.len(),
            m.modified()?,
            m.created()?
        ))
    }
}
fn binary_hash() -> Result<String> {
    Ok(digest(fs::read(std::env::current_exe()?)?))
}
struct IndexCache {
    root: PathBuf,
    actual: PathBuf,
    private: PathBuf,
    stamp: Option<Stamp>,
}
thread_local! {static INDEX_CACHE:RefCell<Option<IndexCache>>=const{RefCell::new(None)};}
fn copy_index(actual: &Path, private: &Path) -> Result<()> {
    history::refuse_symlink(private.parent().context("missing daemon index parent")?)?;
    history::refuse_symlink(private)?;
    let temporary = private.with_extension(format!(
        "index-{}-{}.tmp",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let bytes = fs::read(actual)?;
    let result = (|| {
        private_write(&temporary, &bytes)?;
        fs::rename(&temporary, private)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub(super) fn configure_index(root: &Path) -> Result<()> {
    INDEX_CACHE.with(|cache| *cache.borrow_mut() = None);
    let Ok(path) = archgraph::git::text(
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ) else {
        return Ok(());
    };
    let actual = PathBuf::from(path.trim());
    let private = root.join(".archgraph/daemon.index");
    if !actual.is_file() || actual == private {
        return Ok(());
    }
    copy_index(&actual, &private)?;
    let observed = stamp(&actual);
    INDEX_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(IndexCache {
            root: root.to_owned(),
            actual,
            private,
            stamp: observed,
        })
    });
    Ok(())
}
pub(super) fn cached_index(root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    INDEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let cache = cache.as_mut().filter(|c| c.root == root)?;
        if history::refuse_symlink(cache.private.parent()?).is_err()
            || history::refuse_symlink(&cache.private).is_err()
        {
            return None;
        }
        let observed = stamp(&cache.actual);
        if observed != cache.stamp {
            copy_index(&cache.actual, &cache.private).ok()?;
            cache.stamp = observed;
        }
        Some(cache.private.clone())
    })
}

fn environment() -> BTreeMap<String, Option<String>> {
    let values: BTreeMap<_, _> = std::env::vars_os()
        .filter(|(key, _)| !matches!(key.to_str(), Some("PWD" | "OLDPWD" | "SHLVL" | "_")))
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    BTreeMap::from([(
        "fingerprint".into(),
        Some(digest(
            serde_json::to_vec(&values).expect("serializing environment"),
        )),
    )])
}

fn descriptor(root: &Path) -> PathBuf {
    root.join(".archgraph/daemon.json")
}
fn discover(root: &Path) -> Option<Endpoint> {
    let start = root.canonicalize().ok()?;
    for dir in start.ancestors() {
        let path = descriptor(dir);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 65536 {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                continue;
            }
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        if bytes.len() > 65536 {
            continue;
        }
        let Ok(ep) = serde_json::from_slice::<Endpoint>(&bytes) else {
            continue;
        };
        if ep.version == VERSION
            && ep.root == dir
            && (binary_metadata().ok().as_ref() == Some(&ep.binary_metadata)
                || binary_hash().ok().as_ref() == Some(&ep.binary))
        {
            return Some(ep);
        }
    }
    None
}
trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}
fn connect(ep: &Endpoint) -> Result<Box<dyn Stream>> {
    if let Some(port) = ep.port {
        let s = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse()?,
            Duration::from_millis(50),
        )?;
        s.set_read_timeout(Some(Duration::from_secs(600)))?;
        s.set_write_timeout(Some(Duration::from_secs(5)))?;
        return Ok(Box::new(s));
    }
    #[cfg(unix)]
    {
        let s = std::os::unix::net::UnixStream::connect(
            ep.socket.as_ref().context("missing daemon socket")?,
        )?;
        s.set_read_timeout(Some(Duration::from_secs(600)))?;
        s.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(Box::new(s))
    }
    #[cfg(not(unix))]
    {
        bail!("Unix sockets unavailable; start serve --port")
    }
}
fn send(stream: &mut dyn Stream, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > FRAME_LIMIT {
        bail!("daemon frame exceeds 16 MiB; use --brief or --no-daemon")
    };
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}
fn receive<T: serde::de::DeserializeOwned>(stream: &mut dyn Stream) -> Result<T> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let size = u32::from_be_bytes(length) as usize;
    if size > FRAME_LIMIT {
        bail!("daemon frame exceeds 16 MiB; use --brief or --no-daemon")
    };
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
fn rpc(ep: &Endpoint, args: Vec<String>, tty: bool) -> Result<Option<archgraph::output::Output>> {
    let Ok(mut stream) = connect(ep) else {
        return Ok(None);
    };
    let req = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: "command".into(),
        params: Params {
            token: ep.token.clone(),
            args,
            cwd: std::env::current_dir()?,
            env: environment(),
            tty,
        },
    };
    send(&mut *stream,&req).context("daemon request interrupted; use --no-daemon after checking whether output files were written")?;
    let response:Response=receive(&mut *stream).context("daemon response lost; command may have run; inspect outputs before retrying with --no-daemon")?;
    if let Some(error) = response.error {
        bail!("{}", error.message)
    };
    Ok(response.result)
}
pub(super) fn forward(cli: &Cli) -> Result<Option<archgraph::output::Output>> {
    use std::io::IsTerminal;
    if cli.no_daemon {
        return Ok(None);
    }
    if !matches!(
        cli.command,
        Cmd::Scan(_)
            | Cmd::Report(_)
            | Cmd::Check(_)
            | Cmd::Context(_)
            | Cmd::Impact(_)
            | Cmd::Surface(_)
            | Cmd::Simulate(_)
            | Cmd::Explain(_)
            | Cmd::Status(_)
            | Cmd::McpCall(_)
    ) {
        return Ok(None);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--github-actions") {
        return Ok(None);
    }
    let start = match &cli.command {
        Cmd::Scan(a) => a.root.clone(),
        Cmd::Report(a) => a.root.clone(),
        Cmd::Status(a) => a.root.clone(),
        Cmd::Context(a) => a.root.clone(),
        _ => std::env::current_dir()?,
    };
    forward_args(&start, args, std::io::stdout().is_terminal())
}
pub(super) fn forward_args(
    start: &Path,
    args: Vec<String>,
    tty: bool,
) -> Result<Option<archgraph::output::Output>> {
    let Some(ep) = discover(start) else {
        return Ok(None);
    };
    rpc(&ep, args, tty)
}
pub(super) fn status(args: Status) -> Result<()> {
    let active = ACTIVE.with(|a| a.borrow().clone());
    let value = if let Some(a) = active {
        json!({"schema":"archgraph/daemon/v1","kind":"status","running":true,"root":a.root,"files":a.doc.files.len(),"modules":a.doc.modules.len(),"cycles":a.doc.cycles.len(),"snapshot_limit":2,"generation":a.stats["generation"],"parsed_files":a.stats["parsed_files"],"refresh_ms":a.stats["refresh_ms"],"fallback_reason":a.stats["fallback_reason"],"cached_snapshots":a.stats["cached_snapshots"],"timings":a.stats["timings"]})
    } else {
        json!({"schema":"archgraph/daemon/v1","kind":"status","running":false,"root":args.root.canonicalize()?,"hint":"Start archgraph serve . to keep this graph warm."})
    };
    write(None, &value)
}
fn token() -> Result<String> {
    let mut bytes = [0u8; 32];
    #[cfg(unix)]
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(h: *mut std::ffi::c_void, p: *mut u8, n: u32, flags: u32) -> i32;
        } // System RNG, with no algorithm handle.
        if unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 32, 2) } < 0 {
            bail!("cannot create daemon authentication token; retry or use --no-daemon")
        };
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::fs::OpenOptions;
    history::refuse_symlink(path)?;
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    Ok(())
}
struct Cleanup {
    descriptor: PathBuf,
    socket: Option<PathBuf>,
    token: String,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if fs::read(&self.descriptor)
            .ok()
            .and_then(|b| serde_json::from_slice::<Endpoint>(&b).ok())
            .is_some_and(|e| e.token == self.token)
        {
            let _ = fs::remove_file(&self.descriptor);
            if let Some(s) = &self.socket {
                let _ = fs::remove_file(s);
            }
        }
    }
}
#[path = "daemon_cache.rs"]
mod cache;
use cache::Warm;

fn ignored(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).ok().is_some_and(|p| {
        if p.starts_with(".git") {
            return !matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("HEAD" | "index" | "packed-refs")
            ) && !p.starts_with(".git/refs");
        }
        p.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some(".archgraph" | "node_modules" | "target" | "__pycache__")
            )
        })
    })
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    identity: u64,
    modified: Option<std::time::SystemTime>,
    len: u64,
}
fn stamp(path: &Path) -> Option<Stamp> {
    fs::metadata(path).ok().map(|m| Stamp {
        identity: {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                m.ino()
            }
            #[cfg(not(unix))]
            {
                0
            }
        },
        modified: m.modified().ok(),
        len: m.len(),
    })
}
fn stamps(warm: &Warm, extra: &[PathBuf]) -> BTreeMap<PathBuf, Option<Stamp>> {
    let mut paths: BTreeSet<PathBuf> = warm
        .doc
        .files
        .iter()
        .map(|f| warm.args.root.join(&f.path))
        .chain(
            warm.doc
                .config
                .excluded_files
                .iter()
                .map(|f| warm.args.root.join(f)),
        )
        .collect();
    for name in [
        "layout.yaml",
        ".gitignore",
        ".archgraphignore",
        "CODEOWNERS",
        ".github/CODEOWNERS",
        "docs/CODEOWNERS",
        "package.json",
        "tsconfig.json",
        "go.mod",
        "go.sum",
    ] {
        paths.insert(warm.args.root.join(name));
    }
    if let Some(p) = &warm.args.opts.rules {
        paths.insert(p.clone());
    }
    let parents: Vec<_> = paths
        .iter()
        .filter_map(|p| p.parent().map(Path::to_owned))
        .filter(|p| !p.starts_with(warm.args.root.join(".git")))
        .collect();
    paths.extend(parents);
    paths.extend(extra.iter().cloned());
    paths.insert(warm.args.root.clone());
    paths
        .into_iter()
        .map(|p| {
            let s = stamp(&p);
            (p, s)
        })
        .collect()
}
fn same_stamps(a: &BTreeMap<PathBuf, Option<Stamp>>, b: &BTreeMap<PathBuf, Option<Stamp>>) -> bool {
    a.keys()
        .chain(b.keys())
        .all(|path| a.get(path).and_then(Option::as_ref) == b.get(path).and_then(Option::as_ref))
}
fn metadata_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for name in [
        "HEAD",
        "index",
        "packed-refs",
        "config",
        "info/exclude",
        "refs",
    ] {
        if let Ok(value) = archgraph::git::text(
            root,
            &["rev-parse", "--path-format=absolute", "--git-path", name],
        ) {
            paths.insert(PathBuf::from(value.trim()));
        }
    }
    if let Ok(branch) = archgraph::git::text(root, &["symbolic-ref", "-q", "HEAD"])
        && let Ok(value) = archgraph::git::text(
            root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                branch.trim(),
            ],
        )
    {
        paths.insert(PathBuf::from(value.trim()));
    }
    for dir in root.ancestors() {
        paths.insert(dir.join(".git"));
        for name in ["tsconfig.json", "package.json", "pnpm-workspace.yaml"] {
            paths.insert(dir.join(name));
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    if let Ok(path) =
        archgraph::git::text(root, &["config", "--path", "--get", "core.excludesFile"])
    {
        let path = PathBuf::from(path.trim());
        paths.insert(if path.is_absolute() {
            path
        } else {
            root.join(path)
        });
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.insert(home.join(".gitconfig"));
        paths.insert(home.join(".config/git/config"));
        paths.insert(home.join(".config/git/ignore"));
    }
    paths.into_iter().collect()
}
fn startup_stamps(root: &Path, extra: &[PathBuf]) -> Result<BTreeMap<PathBuf, Option<Stamp>>> {
    let mut paths: BTreeSet<PathBuf> = extra.iter().cloned().collect();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        paths.insert(dir.clone());
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if ignored(&path, root) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path)
            } else if kind.is_file() {
                paths.insert(path);
            }
        }
    }
    Ok(paths
        .into_iter()
        .map(|p| {
            let s = stamp(&p);
            (p, s)
        })
        .collect())
}

fn drain(
    rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    root: &Path,
    dirty: &mut BTreeSet<PathBuf>,
) {
    while let Ok(event) = rx.try_recv() {
        match event {
            Ok(event) => {
                if event.kind.is_access() {
                    continue;
                }
                if event.need_rescan() {
                    dirty.insert(root.to_owned());
                }
                for p in event.paths {
                    if p.is_dir() && event.kind.is_modify() {
                        continue;
                    }
                    if !ignored(&p, root) {
                        dirty.insert(p);
                    }
                }
            }
            Err(e) => {
                eprintln!("daemon: watcher error: {e}; full refresh required");
                dirty.insert(root.to_owned());
            }
        }
    }
}

/// Before/after metadata barriers prevent delayed notify events from serving an older tree.
fn refresh_pending(
    warm: &mut Warm,
    rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    known: &mut BTreeMap<PathBuf, Option<Stamp>>,
    dirty: &mut BTreeSet<PathBuf>,
    overflow: &std::sync::atomic::AtomicBool,
    extra: &mut Vec<PathBuf>,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let rescan = overflow.swap(false, std::sync::atomic::Ordering::Relaxed);
        drain(rx, &warm.args.root, dirty);
        let observed = stamps(warm, extra);
        let force_rescan = dirty.contains(&warm.args.root);
        dirty.retain(|p| known.get(p).is_none() || observed.get(p) != known.get(p));
        if force_rescan {
            dirty.insert(warm.args.root.clone());
        }
        if rescan {
            dirty.insert(warm.args.root.clone());
        }
        for (path, now) in &observed {
            if known.get(path).and_then(Option::as_ref) != now.as_ref() {
                dirty.insert(path.clone());
            }
        }
        if dirty.is_empty() {
            return Ok(());
        }
        let paths: Vec<_> = std::mem::take(dirty).into_iter().collect();
        if let Err(error) = warm.refresh(&paths) {
            dirty.extend(paths);
            return Err(error);
        }
        if warm.fallback.is_some() {
            *extra = metadata_paths(&warm.args.root);
        }
        if let Some(session) = &warm.session {
            extra.extend(session.metadata_files());
            extra.sort();
            extra.dedup();
        }
        let after = stamps(warm, extra);
        if same_stamps(&after, &observed) {
            *known = after;
            return Ok(());
        }
        *known = observed;
        if Instant::now() > deadline {
            bail!("repository keeps changing during refresh; wait for writes to finish and retry")
        }
    }
}

pub(super) fn serve(mut args: Serve) -> Result<ExitCode> {
    let base_globals = globals();
    let root = args.root.canonicalize()?;
    history::refuse_symlink(&root.join(".archgraph"))?;
    fs::create_dir_all(root.join(".archgraph"))?;
    args.socket = args.socket.map(std::path::absolute).transpose()?;
    let path = descriptor(&root);
    let existing = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Endpoint>(&bytes).ok());
    if let Some(old) = existing.filter(|e| e.root == root)
        && connect(&old).is_ok()
    {
        bail!(
            "daemon already running for {}; use status or --no-daemon",
            root.display()
        );
    }
    if path.exists() {
        history::refuse_symlink(&path)?;
        fs::remove_file(&path)?;
    }
    let socket = if args.port.is_some() {
        None
    } else {
        Some(
            args.socket
                .clone()
                .unwrap_or_else(|| root.join(".archgraph/daemon.sock")),
        )
    };
    #[cfg(unix)]
    let unix = if let Some(socket) = &socket {
        if let Ok(meta) = fs::symlink_metadata(socket) {
            use std::os::unix::fs::FileTypeExt;
            if std::os::unix::net::UnixStream::connect(socket).is_ok() {
                bail!(
                    "{} has a live listener; choose another --socket",
                    socket.display()
                )
            }
            if !meta.file_type().is_socket() {
                bail!(
                    "{} is not a stale socket; choose --socket another-path",
                    socket.display()
                )
            }
            fs::remove_file(socket)?;
        }
        let s = std::os::unix::net::UnixListener::bind(socket)
            .context("cannot bind daemon socket; choose a shorter --socket path or use --port")?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        s.set_nonblocking(true)?;
        Some(s)
    } else {
        None
    };
    #[cfg(not(unix))]
    if socket.is_some() {
        bail!("use serve --port on this platform")
    }
    let tcp = args
        .port
        .map(|p| TcpListener::bind(("127.0.0.1", p)))
        .transpose()?;
    if let Some(tcp) = &tcp {
        tcp.set_nonblocking(true)?;
    }
    let ep = Endpoint {
        version: VERSION,
        binary: binary_hash()?,
        binary_metadata: binary_metadata()?,
        root: root.clone(),
        socket: socket.clone(),
        port: tcp
            .as_ref()
            .map(|s| s.local_addr().map(|a| a.port()))
            .transpose()?,
        token: token()?,
    };
    let cleanup = Cleanup {
        descriptor: path.clone(),
        socket,
        token: ep.token.clone(),
    };
    let (tx, rx) = mpsc::sync_channel(1024);
    let overflow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watch_overflow = overflow.clone();
    let mut watcher = if args.once {
        None
    } else {
        Some(notify::recommended_watcher(move |event| {
            if tx.try_send(event).is_err() {
                watch_overflow.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })?)
    };
    if let Some(w) = &mut watcher {
        w.watch(&root, RecursiveMode::Recursive)?;
    }
    let mut extra = metadata_paths(&root);
    let mut before = startup_stamps(&root, &extra)?;
    let startup_args = args.clone();
    let mut warm = Warm::cold(args)?;
    let startup_deadline = Instant::now() + Duration::from_secs(60);
    let stable = loop {
        let mut startup_dirty = BTreeSet::new();
        drain(&rx, &root, &mut startup_dirty);
        if let Some(session) = &warm.session {
            extra.extend(session.metadata_files());
            extra.sort();
            extra.dedup();
        }
        let after = startup_stamps(&root, &extra)?;
        if after == before
            && startup_dirty.is_empty()
            && !overflow.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            break after;
        }
        if Instant::now() > startup_deadline {
            bail!(
                "repository keeps changing during daemon startup; finish writes and restart serve"
            )
        }
        eprintln!("daemon: full collection fallback: files changed during startup");
        extra.extend(metadata_paths(&root));
        extra.sort();
        extra.dedup();
        before = startup_stamps(&root, &extra)?;
        warm = Warm::cold(startup_args.clone())?;
    };
    // Catch workspace metadata changes above a scanned monorepo package.
    if let Some(w) = &mut watcher {
        for parent in root.ancestors().skip(1) {
            for name in ["tsconfig.json", "package.json", "pnpm-workspace.yaml"] {
                let p = parent.join(name);
                if p.is_file() {
                    w.watch(&p, RecursiveMode::NonRecursive)?;
                }
            }
            if parent.join(".git").exists() {
                break;
            }
        }
    }
    if let Some(w) = &mut watcher {
        for p in &extra {
            if p.exists() {
                let mode = if p.is_dir() && p.file_name().is_some_and(|n| n == "refs") {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                w.watch(p, mode)?;
            }
        }
    }
    private_write(&path, &serde_json::to_vec(&ep)?)?;
    eprintln!("daemon: ready for {} at {}", root.display(), path.display());
    let env = environment();
    let mut dirty = BTreeSet::new();
    let mut known = stamps(&warm, &extra)
        .into_keys()
        .map(|p| {
            let value = stable.get(&p).cloned().unwrap_or(None);
            (p, value)
        })
        .collect();
    loop {
        GLOBALS.with(|g| *g.borrow_mut() = base_globals.clone());
        collect::request_timeout(
            (base_globals.tool_timeout > 0).then(|| Duration::from_secs(base_globals.tool_timeout)),
        );
        drain(&rx, &root, &mut dirty);
        if (!dirty.is_empty() || overflow.load(std::sync::atomic::Ordering::Relaxed))
            && let Err(error) = refresh_pending(
                &mut warm, &rx, &mut known, &mut dirty, &overflow, &mut extra,
            )
        {
            eprintln!(
                "daemon: graph refresh failed: {error:#}; fix the source/configuration and retry"
            );
        }
        let mut incoming: Option<Box<dyn Stream>> = None;
        #[cfg(unix)]
        if let Some(s) = &unix {
            match s.accept() {
                Ok((s, _)) => {
                    // macOS accepted sockets can inherit the listener's nonblocking mode.
                    s.set_nonblocking(false)?;
                    s.set_read_timeout(Some(Duration::from_secs(5)))?;
                    s.set_write_timeout(Some(Duration::from_secs(30)))?;
                    incoming = Some(Box::new(s));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }
        if let Some(s) = &tcp {
            match s.accept() {
                Ok((s, _)) => {
                    // macOS accepted sockets can inherit the listener's nonblocking mode.
                    s.set_nonblocking(false)?;
                    s.set_read_timeout(Some(Duration::from_secs(5)))?;
                    s.set_write_timeout(Some(Duration::from_secs(30)))?;
                    incoming = Some(Box::new(s));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }
        let Some(mut incoming) = incoming else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        let request: Request = match receive(&mut *incoming) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("daemon: rejected request: {e:#}");
                continue;
            }
        };
        let result = (|| -> Result<archgraph::output::Output> {
            if request.jsonrpc != "2.0"
                || request.method != "command"
                || request.params.token != ep.token
            {
                bail!("invalid local daemon request; use the matching archgraph CLI")
            }
            if request.params.env != env {
                bail!(
                    "daemon environment differs from caller; restart serve in this environment or use --no-daemon"
                )
            }
            let cwd = request.params.cwd.canonicalize()?;
            refresh_pending(
                &mut warm, &rx, &mut known, &mut dirty, &overflow, &mut extra,
            )?;
            let cli = Cli::try_parse_from(
                std::iter::once("archgraph".into()).chain(request.params.args.clone()),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            if !matches!(
                cli.command,
                Cmd::Scan(_)
                    | Cmd::Report(_)
                    | Cmd::Check(_)
                    | Cmd::Context(_)
                    | Cmd::Impact(_)
                    | Cmd::Surface(_)
                    | Cmd::Simulate(_)
                    | Cmd::Explain(_)
                    | Cmd::Status(_)
                    | Cmd::McpCall(_)
            ) || request.params.args.iter().any(|a| a == "--github-actions")
            {
                bail!(
                    "unsupported daemon command; use --no-daemon for side effects or other commands"
                )
            }
            let previous = std::env::current_dir()?;
            std::env::set_current_dir(&cwd)?;
            warm.activate();
            CALLER_TTY.with(|v| v.set(Some(request.params.tty)));
            let capture = archgraph::output::Capture::begin();
            let code = execute(cli);
            let code = if code == ExitCode::SUCCESS {
                0
            } else if code == ExitCode::from(1) {
                1
            } else {
                2
            };
            let out = capture.finish(code);
            ACTIVE.with(|a| *a.borrow_mut() = None);
            CALLER_TTY.with(|v| v.set(None));
            std::env::set_current_dir(previous)?;
            GLOBALS.with(|g| *g.borrow_mut() = base_globals.clone());
            Ok(out)
        })();
        let response = match result {
            Ok(out) => Response {
                jsonrpc: "2.0".into(),
                id: request.id,
                result: Some(out),
                error: None,
            },
            Err(e) => Response {
                jsonrpc: "2.0".into(),
                id: request.id,
                result: None,
                error: Some(RpcError {
                    code: -32000,
                    message: format!("{e:#}"),
                }),
            },
        };
        let response = if serde_json::to_vec(&response)?.len() > FRAME_LIMIT {
            Response {
                jsonrpc: "2.0".into(),
                id: response.id,
                result: None,
                error: Some(RpcError {
                    code: -32001,
                    message: "response exceeds 16 MiB; use --brief or --no-daemon".into(),
                }),
            }
        } else {
            response
        };
        let _ = send(&mut *incoming, &response);
        if warm.args.once {
            break;
        }
    }
    drop(cleanup);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_uses_standard_params_and_exclusive_result_or_error() {
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "command".into(),
            params: Params {
                token: "fixture".into(),
                args: vec!["status".into()],
                cwd: PathBuf::from("/repo"),
                env: BTreeMap::new(),
                tty: false,
            },
        };
        let schema: Value =
            serde_json::from_str(include_str!("../../schema/daemon.v1.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&serde_json::to_value(request).unwrap()));
        let response = Response {
            jsonrpc: "2.0".into(),
            id: 1,
            result: None,
            error: Some(RpcError {
                code: -32000,
                message: "use --no-daemon".into(),
            }),
        };
        let mut wire = std::io::Cursor::new(Vec::new());
        send(&mut wire, &response).unwrap();
        wire.set_position(0);
        let received: Response = receive(&mut wire).unwrap();
        let value = serde_json::to_value(received).unwrap();
        assert!(validator.is_valid(&value));
        assert!(value.get("result").is_none());
        assert_eq!(value["error"]["code"], -32000);
    }
    #[test]
    fn newly_discovered_missing_resolution_probes_do_not_trigger_refresh() {
        let known = BTreeMap::new();
        let observed = BTreeMap::from([(PathBuf::from("missing/package.json"), None)]);
        assert!(same_stamps(&known, &observed));
        assert!(same_stamps(&observed, &known));
    }
    #[test]
    fn watcher_rescan_and_errors_preserve_full_refresh_sentinel() {
        let (tx, rx) = mpsc::channel();
        let root = Path::new("/repo");
        let event =
            notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan);
        tx.send(Ok(event)).unwrap();
        let mut dirty = BTreeSet::new();
        drain(&rx, root, &mut dirty);
        assert!(dirty.remove(root));
        tx.send(Err(notify::Error::generic("fixture overflow")))
            .unwrap();
        drain(&rx, root, &mut dirty);
        assert!(dirty.contains(root));
    }
    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let mut wire = std::io::Cursor::new(((FRAME_LIMIT + 1) as u32).to_be_bytes().to_vec());
        let error = receive::<Response>(&mut wire).err().unwrap().to_string();
        assert!(error.contains("--brief"));
    }
}
