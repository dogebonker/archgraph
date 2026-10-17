//! Rust through `rust-analyzer scip`, from the active rustup toolchain, in an environment that
//! runs no repository code: no build scripts or proc macros unless asked, the toolchain's own
//! rustc and cargo, no repository rustflags, wrappers, linkers, environment or downloads. A
//! repository file that would still name a program is refused before anything runs.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::RawGraph;
use crate::model::{Collector, Edge, EdgeKind};

/// The Cargo features rust-analyzer enables (`cargo.features`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Features {
    /// Every feature of every package
    All,
    /// Each package's default features
    Default,
}

pub struct Options<'a> {
    /// rust-analyzer to run instead of the toolchain's.
    pub rust_analyzer: Option<&'a Path>,
    pub features: Features,
    /// Runs build scripts and proc macros: repository code, with the user's permissions.
    pub build_scripts: bool,
}

/// SCIP messages, the fields archgraph reads (scip.proto, sourcegraph/scip).
mod scip {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Index {
        #[prost(message, optional, tag = "1")]
        pub metadata: Option<Metadata>,
        #[prost(message, repeated, tag = "2")]
        pub documents: Vec<Document>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Metadata {
        #[prost(message, optional, tag = "2")]
        pub tool_info: Option<ToolInfo>,
        #[prost(string, tag = "3")]
        pub project_root: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ToolInfo {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(string, tag = "2")]
        pub version: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Document {
        #[prost(string, tag = "1")]
        pub relative_path: String,
        #[prost(message, repeated, tag = "2")]
        pub occurrences: Vec<Occurrence>,
        #[prost(message, repeated, tag = "3")]
        pub symbols: Vec<SymbolInformation>,
        #[prost(string, tag = "5")]
        pub text: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Occurrence {
        #[prost(int32, repeated, tag = "1")]
        pub range: Vec<i32>,
        #[prost(string, tag = "2")]
        pub symbol: String,
        #[prost(int32, tag = "3")]
        pub symbol_roles: i32,
        #[prost(int32, repeated, tag = "7")]
        pub enclosing_range: Vec<i32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SymbolInformation {
        #[prost(string, tag = "1")]
        pub symbol: String,
        #[prost(message, repeated, tag = "4")]
        pub relationships: Vec<Relationship>,
        #[prost(int32, tag = "5")]
        pub kind: i32,
        #[prost(message, optional, boxed, tag = "7")]
        pub signature_documentation: Option<Box<Document>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Relationship {
        #[prost(string, tag = "1")]
        pub symbol: String,
        #[prost(bool, tag = "3")]
        pub is_implementation: bool,
    }

    /// `SymbolRole.Definition`.
    pub const DEFINITION: i32 = 1;
    /// `SymbolInformation.Kind.Module`.
    pub const MODULE: i32 = 29;
}

/// What `run` writes and `parse` reads: the index without locals and without occurrences of
/// symbols it defines nowhere (std, external crates), and the workspace packages under the root.
#[derive(Serialize, Deserialize)]
struct Dump {
    rust_analyzer: String,
    toolchain: String,
    features: Features,
    build_scripts: bool,
    packages: Vec<Package>,
    documents: Vec<Doc>,
}

#[derive(Serialize, Deserialize)]
struct Package {
    name: String,
    /// The manifest's directory, root-relative (`.` for the root).
    dir: String,
    targets: Vec<Target>,
}

#[derive(Serialize, Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    /// Root-relative.
    src_path: String,
}

#[derive(Serialize, Deserialize)]
struct Doc {
    path: String,
    occurrences: Vec<Occ>,
    symbols: Vec<Sym>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Occ {
    range: Vec<i32>,
    symbol: String,
    roles: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    enclosing_range: Vec<i32>,
}

#[derive(Serialize, Deserialize)]
struct Sym {
    symbol: String,
    kind: i32,
    /// The declaration as written (`pub fn f()`, `mod tests`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    signature: String,
    /// The traits (or trait members) this symbol implements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    implements: Vec<String>,
}

/// The rustup toolchain every child of the scan runs from.
struct Toolchain {
    name: String,
    bin: PathBuf,
    host: String,
    rust_analyzer: PathBuf,
    /// Installed toolchain names (`stable-aarch64-apple-darwin`, `1.95.0-...`, custom links).
    installed: Vec<String>,
}

impl Toolchain {
    /// rustup's active toolchain, asked from `tmp` (outside the repository, so its
    /// rust-toolchain.toml does not apply), never installing anything.
    fn find(rust_analyzer: Option<&Path>, tmp: &Path) -> Result<Toolchain> {
        let rustup = |args: &[&str]| -> Result<String> {
            let out = Command::new("rustup")
                .args(args)
                .current_dir(tmp)
                .env("RUSTUP_AUTO_INSTALL", "0")
                .output()
                .context("running rustup (install it from https://rustup.rs)")?;
            if !out.status.success() {
                bail!(
                    "rustup {} failed: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        };
        let active = rustup(&["show", "active-toolchain"])?;
        let name = active
            .split_whitespace()
            .next()
            .context("rustup reports no active toolchain")?
            .to_owned();
        let rustc = PathBuf::from(rustup(&["which", "--toolchain", &name, "rustc"])?);
        let bin = rustc.parent().context("rustc has no directory")?.to_owned();
        let rust_analyzer = match rust_analyzer {
            Some(p) => fs::canonicalize(p)
                .with_context(|| format!("--rust-analyzer {}: no such executable", p.display()))?,
            None => PathBuf::from(
                rustup(&["which", "--toolchain", &name, "rust-analyzer"]).with_context(|| {
                    format!("run `rustup component add rust-analyzer --toolchain {name}`")
                })?,
            ),
        };
        let toolchain = Toolchain {
            host: String::new(),
            installed: Vec::new(),
            name,
            bin,
            rust_analyzer,
        };
        if !toolchain.library().is_dir() {
            bail!(
                "toolchain {} has no rust-src, which rust-analyzer needs for std: run \
                 `rustup component add rust-src --toolchain {0}`",
                toolchain.name
            );
        }
        let out = Command::new(&rustc)
            .arg("-vV")
            .current_dir(tmp)
            .output()
            .with_context(|| format!("running {}", rustc.display()))?;
        let host = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("host: "))
            .context("rustc -vV names no host")?
            .to_owned();
        let installed = rustup(&["toolchain", "list"])?
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .map(str::to_owned)
            .collect();
        Ok(Toolchain {
            host,
            installed,
            ..toolchain
        })
    }

    /// The std sources rust-analyzer loads.
    fn library(&self) -> PathBuf {
        self.bin.with_file_name("lib/rustlib/src/rust/library")
    }

    /// Whether rustup would find `channel` installed, by name or with the host appended.
    fn has(&self, channel: &str) -> bool {
        let full = format!("{channel}-{}", self.host);
        self.installed.iter().any(|t| *t == channel || *t == full)
    }

    /// `program` with the environment of every cargo, rustc and rust-analyzer the scan runs:
    /// this toolchain's binaries first on PATH and named by RUSTC and CARGO, no rustc wrapper,
    /// no rustflags from the repository's config (CARGO_ENCODED_RUSTFLAGS takes precedence over
    /// every other source, and empty means none), `cc` as the host linker, a build directory
    /// outside the repository, and no network.
    fn command(&self, program: &Path, target_dir: &Path) -> Command {
        let path = env::var_os("PATH").unwrap_or_default();
        let path =
            env::join_paths(std::iter::once(self.bin.clone()).chain(env::split_paths(&path)))
                .unwrap_or(path);
        let linker = format!(
            "CARGO_TARGET_{}_LINKER",
            self.host.to_uppercase().replace(['-', '.'], "_")
        );
        let mut cmd = Command::new(program);
        cmd.env("PATH", path)
            .env("RUSTUP_TOOLCHAIN", &self.name)
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("RUSTC", self.bin.join("rustc"))
            .env("CARGO", self.bin.join("cargo"))
            .env("RUSTC_WRAPPER", "")
            .env("RUSTC_WORKSPACE_WRAPPER", "")
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .env_remove("RUSTFLAGS")
            .env("CARGO_TARGET_DIR", target_dir)
            .env("CARGO_NET_OFFLINE", "true")
            .env(linker, "cc");
        cmd
    }
}

#[derive(Deserialize)]
struct Metadata {
    workspace_root: PathBuf,
    packages: Vec<MetaPackage>,
}

#[derive(Deserialize)]
struct MetaPackage {
    name: String,
    manifest_path: PathBuf,
    targets: Vec<MetaTarget>,
}

#[derive(Deserialize)]
struct MetaTarget {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

/// Indexes the Cargo workspace at `root` and returns the JSON `parse` reads. Refuses a
/// repository that names a toolchain, a program or a library for cargo or rust-analyzer, and one
/// without Cargo.lock; fails on a load missing dependencies and on an empty index.
pub fn run(root: &Path, opts: &Options) -> Result<Vec<u8>> {
    let root = fs::canonicalize(root)?;
    if !root.join("Cargo.toml").is_file() {
        bail!("no Cargo.toml in {}", root.display());
    }
    let channels = refuse(&root)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = env::temp_dir().join(format!("archgraph-rust-{}-{nanos}", std::process::id()));
    fs::create_dir(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let result = fs::canonicalize(&tmp)
        .map_err(anyhow::Error::from)
        .and_then(|tmp| run_in(&root, opts, &tmp, &channels));
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn run_in(
    root: &Path,
    opts: &Options,
    tmp: &Path,
    channels: &[(PathBuf, String)],
) -> Result<Vec<u8>> {
    let tc = Toolchain::find(opts.rust_analyzer, tmp)?;
    if let Some((file, channel)) = channels.iter().find(|(_, c)| !tc.has(c)) {
        bail!(
            "refusing to scan: {} names toolchain {channel}, which is not installed, and \
             archgraph never has rustup download one: install it with `rustup toolchain install \
             {channel}`",
            file.display()
        );
    }
    let target = tmp.join("target");
    // cargo reads .cargo/config.toml from its working directory up, as when rust-analyzer runs
    // it in the workspace: the files `refuse` checked.
    let cargo = |args: &[&str]| -> Result<Vec<u8>, String> {
        let out = tc
            .command(&tc.bin.join("cargo"), &target)
            .args(args)
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .current_dir(root)
            .output()
            .map_err(|e| format!("running cargo: {e}"))?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    };
    let meta = cargo(&[
        "metadata",
        "--format-version",
        "1",
        "--no-deps",
        "--offline",
    ])
    .map_err(|e| anyhow!("cargo metadata failed: {e}"))?;
    let meta: Metadata = serde_json::from_slice(&meta).context("reading cargo metadata")?;
    if !meta.workspace_root.join("Cargo.lock").is_file() {
        bail!(
            "no Cargo.lock in {}: run `cargo generate-lockfile` there first (archgraph never writes \
             into the repository, and cargo would)",
            meta.workspace_root.display()
        );
    }
    // The resolution rust-analyzer asks for: the host's dependencies only.
    let mut resolve = vec!["metadata", "--format-version", "1", "--offline", "--locked"];
    resolve.extend(["--filter-platform", &tc.host]);
    if opts.features == Features::All {
        resolve.push("--all-features");
    }
    cargo(&resolve).map_err(|e| {
        anyhow!(
            "cargo cannot resolve the workspace offline from Cargo.lock: {e}\n`cargo fetch \
             --locked` downloads the dependencies, and runs no repository code"
        )
    })?;

    let tru = if opts.build_scripts {
        None
    } else {
        Some(true_program()?)
    };
    let config_path = tmp.join("rust-analyzer.json");
    fs::write(
        &config_path,
        config(tru.as_deref(), &target, opts.features).to_string(),
    )?;
    let index_path = tmp.join("index.scip");
    let out = tc
        .command(&tc.rust_analyzer, &target)
        .arg("scip")
        .arg(root)
        .arg("--output")
        .arg(&index_path)
        .arg("--config-path")
        .arg(&config_path)
        .arg("--exclude-vendored-libraries")
        .current_dir(tmp)
        .env("PWD", tmp)
        .output()
        .with_context(|| format!("running {}", tc.rust_analyzer.display()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        bail!("rust-analyzer scip failed: {}", stderr.trim());
    }
    if let Some(line) = config_error(&stderr) {
        bail!("rust-analyzer rejected archgraph's configuration: {line}");
    }
    if let Some(error) = degraded_load(&stderr, &tc.library().to_string_lossy()) {
        bail!(
            "rust-analyzer loaded the workspace without its dependencies: {error}\n`cargo fetch \
             --locked` downloads them, and runs no repository code"
        );
    }
    let bytes = fs::read(&index_path).context("reading rust-analyzer's index")?;
    let index = scip::Index::decode(bytes.as_slice()).context("decoding rust-analyzer's index")?;
    let dump = dump(index, root, meta, &tc.name, opts)?;
    Ok(serde_json::to_vec(&dump)?)
}

/// `true` from an absolute PATH entry: rust-analyzer runs the build-script command in the
/// repository, where a relative entry could name a repository file.
fn true_program() -> Result<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .filter(|d| d.is_absolute())
        .map(|d| d.join("true"))
        .find(|p| p.is_file())
        .context("no `true` program in an absolute PATH directory")
}

/// rust-analyzer's configuration: build scripts replaced by `tru` (rust-analyzer then builds
/// no proc macro either) unless it is None, and `cfg(test)` off, so `cfg(not(test))` items stay
/// defined; test code is told apart lexically.
pub fn config(tru: Option<&Path>, target_dir: &Path, features: Features) -> Value {
    let mut cargo = json!({
        "targetDir": target_dir,
        "features": match features {
            Features::All => json!("all"),
            Features::Default => json!([]),
        },
    });
    if let Some(tru) = tru {
        cargo["buildScripts"] = json!({ "overrideCommand": [tru] });
    }
    json!({ "cargo": cargo, "cfg": { "setTest": false } })
}

/// The `Config Error(s)` line, when its error sink is not empty (rust-analyzer logs the line
/// even when it is).
pub fn config_error(stderr: &str) -> Option<&str> {
    stderr
        .lines()
        .find(|l| l.contains("error_sink=ConfigErrors(") && !l.contains("ConfigErrors([])"))
}

/// The first `cargo metadata` failure rust-analyzer fell back from to `--no-deps` that is not
/// about the sysroot under `library`: offline, std's own dependencies are never there, which
/// only drops references inside std macros' arguments.
pub fn degraded_load(stderr: &str, library: &str) -> Option<String> {
    stderr
        .split("returning succeeded result with `--no-deps`")
        .skip(1)
        .map(|chunk| {
            let end = ["\nrust-analyzer: ", "Stack backtrace:"]
                .iter()
                .filter_map(|m| chunk.find(m))
                .min()
                .unwrap_or(chunk.len());
            &chunk[..end]
        })
        .find(|chunk| !chunk.contains(library))
        .map(|chunk| chunk.trim().to_owned())
}

fn dump(
    index: scip::Index,
    root: &Path,
    meta: Metadata,
    toolchain: &str,
    opts: &Options,
) -> Result<Dump> {
    let metadata = index.metadata.unwrap_or_default();
    let expected = format!("file://{}", root.display());
    if metadata.project_root.trim_end_matches('/') != expected {
        bail!(
            "rust-analyzer indexed {}, not {expected}",
            metadata.project_root
        );
    }
    let docs: Vec<scip::Document> = index
        .documents
        .into_iter()
        .filter(|d| {
            let p = &d.relative_path;
            !p.is_empty() && !p.starts_with('/') && !p.split('/').any(|s| s == "..")
        })
        .collect();
    if docs.is_empty() {
        bail!("rust-analyzer indexed no files under {}", root.display());
    }
    let defined: HashSet<String> = docs
        .iter()
        .flat_map(|d| &d.occurrences)
        .filter(|o| o.symbol_roles & scip::DEFINITION != 0)
        .map(|o| o.symbol.clone())
        .collect();
    let kept = |s: &str| !s.starts_with("local ") && defined.contains(s);
    let mut documents: Vec<Doc> = docs
        .into_iter()
        .map(|d| {
            let mut occurrences: Vec<Occ> = d
                .occurrences
                .into_iter()
                .filter(|o| kept(&o.symbol))
                .map(|o| Occ {
                    range: o.range,
                    symbol: o.symbol,
                    roles: o.symbol_roles,
                    enclosing_range: o.enclosing_range,
                })
                .collect();
            occurrences.sort();
            let symbols = d
                .symbols
                .into_iter()
                .filter(|s| kept(&s.symbol))
                .map(|s| Sym {
                    implements: s
                        .relationships
                        .into_iter()
                        .filter(|r| r.is_implementation)
                        .map(|r| r.symbol)
                        .collect(),
                    signature: s
                        .signature_documentation
                        .map(|d| d.text)
                        .unwrap_or_default(),
                    kind: s.kind,
                    symbol: s.symbol,
                })
                .collect();
            Doc {
                path: d.relative_path,
                occurrences,
                symbols,
            }
        })
        .collect();
    documents.sort_by(|a, b| a.path.cmp(&b.path));
    let rel = |p: &Path| -> Option<String> {
        let rel = p
            .strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        Some(if rel.is_empty() { ".".to_owned() } else { rel })
    };
    let mut packages: Vec<Package> = meta
        .packages
        .into_iter()
        .filter_map(|p| {
            Some(Package {
                dir: rel(p.manifest_path.parent()?)?,
                targets: p
                    .targets
                    .into_iter()
                    .filter_map(|t| {
                        Some(Target {
                            src_path: rel(&t.src_path)?,
                            name: t.name,
                            kind: t.kind,
                        })
                    })
                    .collect(),
                name: p.name,
            })
        })
        .collect();
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(Dump {
        rust_analyzer: metadata.tool_info.unwrap_or_default().version,
        toolchain: toolchain.to_owned(),
        features: opts.features,
        build_scripts: opts.build_scripts,
        packages,
        documents,
    })
}

/// Cross-file references to workspace symbols, including trait implementations.
pub fn parse(json: &[u8]) -> Result<RawGraph> {
    let dump: Dump = serde_json::from_slice(json).context("reading rust-analyzer JSON")?;
    let crate_roots: HashSet<&str> = dump
        .documents
        .iter()
        .flat_map(|d| &d.symbols)
        .filter(|s| s.kind == scip::MODULE && s.signature.starts_with("extern crate "))
        .map(|s| s.symbol.as_str())
        .collect();
    let mut definitions: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for doc in &dump.documents {
        for occurrence in &doc.occurrences {
            if occurrence.roles & scip::DEFINITION != 0 && !occurrence.symbol.starts_with("local ")
            {
                definitions
                    .entry(&occurrence.symbol)
                    .or_default()
                    .insert(&doc.path);
            }
        }
    }
    let mut pairs = BTreeSet::new();
    for doc in &dump.documents {
        let namespaces: Vec<&str> = doc
            .symbols
            .iter()
            .filter(|s| s.kind == scip::MODULE)
            .map(|s| s.symbol.as_str())
            .collect();
        let references = doc
            .occurrences
            .iter()
            .filter(|o| o.roles & scip::DEFINITION == 0)
            .map(|o| o.symbol.as_str())
            .chain(
                doc.symbols
                    .iter()
                    .flat_map(|s| s.implements.iter().map(String::as_str)),
            );
        for symbol in references {
            // Namespace traversal through `crate` or an enclosing module is not a dependency.
            if crate_roots.contains(symbol)
                || (symbol.ends_with('/')
                    && namespaces
                        .iter()
                        .any(|&namespace| namespace != symbol && namespace.starts_with(symbol)))
            {
                continue;
            }
            if let Some(paths) = definitions.get(symbol) {
                for &path in paths {
                    if path != doc.path {
                        pairs.insert((doc.path.as_str(), path));
                    }
                }
            }
        }
    }
    let edges = pairs
        .into_iter()
        .map(|(from, to)| Edge {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: EdgeKind::Static,
            verdict: None,
            rule: None,
            reach: None,
        })
        .collect();
    let collector = Collector {
        name: "rust-analyzer".to_owned(),
        version: Some(dump.rust_analyzer),
        cruised: dump.documents.len(),
        unresolved: None,
        type_marking: None,
        main_packages: Vec::new(),
        bound_files: Vec::new(),
        go_modules: Vec::new(),
    };
    Ok(RawGraph {
        files: dump.documents.into_iter().map(|d| d.path).collect(),
        edges,
        collector,
    })
}

/// Refuses, before anything runs, a repository with a file in `root` or an ancestor that would
/// have rust-analyzer, cargo or rustup run what the repository chooses: a rust-project.json, a
/// Cargo config key (`cargo_config_refusal`; $CARGO_HOME's own config is the user's), or a
/// toolchain file naming a toolchain by path. Returns the channels of the nearest toolchain
/// files, which must be installed.
fn refuse(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")))
        .and_then(|p| fs::canonicalize(p).ok());
    let mut channels = Vec::new();
    let mut toolchain_found = false;
    for dir in root.ancestors() {
        for name in ["rust-project.json", ".rust-project.json"] {
            let path = dir.join(name);
            if path.exists() {
                bail!(
                    "refusing to scan: {}: rust-analyzer would load it instead of Cargo.toml, and \
                     it can name proc-macro libraries to load",
                    path.display()
                );
            }
        }
        let dot_cargo = dir.join(".cargo");
        let users = cargo_home.is_some() && fs::canonicalize(&dot_cargo).ok() == cargo_home;
        for name in ["config", "config.toml"] {
            let path = dot_cargo.join(name);
            if !users && path.is_file() {
                let text = fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                cargo_config_refusal(&text)
                    .map_err(|e| anyhow!("refusing to scan: {} {e}", path.display()))?;
            }
        }
        // rustup reads the nearest toolchain file only.
        if !toolchain_found {
            for name in ["rust-toolchain", "rust-toolchain.toml"] {
                let path = dir.join(name);
                if path.is_file() {
                    toolchain_found = true;
                    let text = fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    match toolchain_channel(name, &text) {
                        Ok(Some(channel)) => channels.push((path, channel)),
                        Ok(None) => {}
                        Err(e) => bail!("refusing to scan: {} {e}", path.display()),
                    }
                }
            }
        }
    }
    Ok(channels)
}

/// Err with the reason when a Cargo config sets a key that makes cargo run with an environment,
/// a program, a library or a target the repository chooses: `[env]`, `[unstable]` (rust-analyzer
/// runs some cargo commands as nightly), `include`, `build.target`, rustc, its
/// wrappers, rustdoc, linkers, runners, `links` overrides, credential providers and the doc
/// browser. Rustflags are allowed: CARGO_ENCODED_RUSTFLAGS replaces them. A file this reader
/// cannot parse is refused too.
pub fn cargo_config_refusal(text: &str) -> Result<(), String> {
    let keys = toml_keys(text)
        .map_err(|e| format!("cannot read it as TOML ({e}), so archgraph cannot check it"))?;
    match keys.iter().find(|(path, _)| refused(path)) {
        Some((path, _)) => Err(format!(
            "sets `{}`, so cargo would run with an environment, program, library or target the \
             repository chooses; remove it to scan",
            path.join(".")
        )),
        None => Ok(()),
    }
}

fn refused(path: &[String]) -> bool {
    // Cargo spells keys in kebab-case; snake_case is refused alike.
    let path: Vec<String> = path.iter().map(|s| s.replace('_', "-")).collect();
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    match path.as_slice() {
        ["env" | "unstable" | "include", ..] => true,
        [
            "build",
            "target" | "rustc" | "rustc-wrapper" | "rustc-workspace-wrapper" | "rustdoc",
            ..,
        ] => true,
        ["target", _, key, ..] => !matches!(*key, "rustflags" | "rustdocflags"),
        ["host", rest @ ..] => {
            rest.len() > 2 || rest.iter().any(|k| matches!(*k, "linker" | "runner"))
        }
        ["doc", "browser", ..] => true,
        [.., "credential-provider" | "global-credential-providers"] => true,
        _ => false,
    }
}

/// The channel a rust-toolchain or rust-toolchain.toml file names, if any; Err for a toolchain
/// by path (`toolchain.path`, or a legacy file naming a path), whose programs the repository
/// chooses.
pub fn toolchain_channel(name: &str, text: &str) -> Result<Option<String>, String> {
    let line = text.trim();
    let channel =
        if name == "rust-toolchain" && !line.is_empty() && !line.contains(['\n', '=', '[']) {
            Some(line.to_owned())
        } else {
            let keys = toml_keys(text).map_err(|e| format!("cannot be read as TOML ({e})"))?;
            let mut channel = None;
            for (path, value) in keys {
                match path
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice()
                {
                    ["toolchain", "path"] => {
                        return Err(
                            "sets `toolchain.path`, a toolchain of programs the repository chooses"
                                .to_owned(),
                        );
                    }
                    ["toolchain", "channel"] => {
                        channel = Some(value.ok_or(
                            "sets `toolchain.channel` to a value that is not a one-line string",
                        )?);
                    }
                    _ => {}
                }
            }
            channel
        };
    match channel {
        Some(c) if c.contains(['/', '\\']) => Err(format!(
            "names toolchain {c}, a path to programs the repository chooses"
        )),
        c => Ok(c),
    }
}

/// The key paths a TOML document defines, in order: each table header and each assigned key
/// under its table, then the keys of its inline tables, with the value when it is a one-line
/// string. Errors on anything it does not recognise as TOML, so no key hides in syntax it skips.
fn toml_keys(text: &str) -> Result<Keys, String> {
    let mut p = Toml {
        s: text.chars().collect(),
        i: 0,
        out: Vec::new(),
    };
    let mut table: Vec<String> = Vec::new();
    loop {
        p.blank();
        let Some(c) = p.peek() else { break };
        if c == '[' {
            p.i += 1;
            let array = p.eat('[');
            table = p.key()?;
            p.expect(']')?;
            if array {
                p.expect(']')?;
            }
            p.out.push((table.clone(), None));
        } else {
            let mut path = table.clone();
            path.extend(p.key()?);
            p.expect('=')?;
            p.ws();
            p.value(path)?;
        }
        p.ws();
        if p.peek() == Some('#') {
            p.comment();
        }
        match p.peek() {
            None | Some('\n') => {}
            Some('\r') if p.s.get(p.i + 1) == Some(&'\n') => {}
            Some(c) => return Err(p.error(&format!("unexpected {c:?}"))),
        }
    }
    Ok(p.out)
}

/// Each key path with its value when that is a one-line string.
type Keys = Vec<(Vec<String>, Option<String>)>;

struct Toml {
    s: Vec<char>,
    i: usize,
    out: Keys,
}

impl Toml {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.i += 1;
        }
        found
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {c:?}")))
        }
    }

    fn error(&self, what: &str) -> String {
        let end = self.i.min(self.s.len());
        let line = self.s[..end].iter().filter(|&&c| c == '\n').count() + 1;
        format!("{what} on line {line}")
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.i += 1;
        }
    }

    fn comment(&mut self) {
        while !matches!(self.peek(), None | Some('\n')) {
            self.i += 1;
        }
    }

    /// Spaces, line ends and comments.
    fn blank(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\n' | '\r') => self.i += 1,
                Some('#') => self.comment(),
                _ => return,
            }
        }
    }

    /// A dotted key, with the spaces around and inside it.
    fn key(&mut self) -> Result<Vec<String>, String> {
        let mut path = Vec::new();
        loop {
            self.ws();
            let part = match self.peek() {
                Some('"') => self.basic()?,
                Some('\'') => self.literal()?,
                _ => {
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    {
                        self.i += 1;
                    }
                    if start == self.i {
                        return Err(self.error("expected a key"));
                    }
                    self.s[start..self.i].iter().collect()
                }
            };
            path.push(part);
            self.ws();
            if !self.eat('.') {
                return Ok(path);
            }
        }
    }

    /// `n` hex digits as a char.
    fn hex(&mut self, n: usize) -> Result<char, String> {
        let digits: String = self
            .s
            .get(self.i..self.i + n)
            .unwrap_or_default()
            .iter()
            .collect();
        self.i += n;
        u32::from_str_radix(&digits, 16)
            .ok()
            .filter(|_| digits.len() == n)
            .and_then(char::from_u32)
            .ok_or_else(|| self.error("bad escape"))
    }

    /// A one-line basic string, escapes decoded.
    fn basic(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => return Err(self.error("unterminated string")),
                Some('"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    self.i += 1;
                    let c = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated string"))?;
                    self.i += 1;
                    out.push(match c {
                        'b' => '\u{8}',
                        't' => '\t',
                        'n' => '\n',
                        'f' => '\u{c}',
                        'r' => '\r',
                        'e' => '\u{1b}',
                        '"' => '"',
                        '\\' => '\\',
                        'x' => self.hex(2)?,
                        'u' => self.hex(4)?,
                        'U' => self.hex(8)?,
                        _ => return Err(self.error("bad escape")),
                    });
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// A one-line literal string.
    fn literal(&mut self) -> Result<String, String> {
        self.i += 1;
        let start = self.i;
        loop {
            match self.peek() {
                None | Some('\n') => return Err(self.error("unterminated string")),
                Some('\'') => {
                    let s = self.s[start..self.i].iter().collect();
                    self.i += 1;
                    return Ok(s);
                }
                Some(_) => self.i += 1,
            }
        }
    }

    /// Skips a multi-line string opened by three `quote`s, through its closing three and the
    /// up to two quotes TOML allows right before them.
    fn multiline(&mut self, quote: char) -> Result<(), String> {
        self.i += 3;
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated multi-line string")),
                Some('\\') if quote == '"' => self.i += 2,
                Some(c) if c == quote && self.s.get(self.i..self.i + 3) == Some(&[c, c, c]) => {
                    self.i += 3;
                    for _ in 0..2 {
                        self.eat(quote);
                    }
                    return Ok(());
                }
                Some(_) => self.i += 1,
            }
        }
    }

    /// A value assigned to `path`: records `path`, and the keys of any inline table in it.
    fn value(&mut self, path: Vec<String>) -> Result<(), String> {
        let at = self.out.len();
        self.out.push((path.clone(), None));
        match self.peek() {
            Some(q @ ('"' | '\'')) => {
                if self.s.get(self.i..self.i + 3) == Some(&[q, q, q]) {
                    self.multiline(q)?;
                } else {
                    let s = if q == '"' {
                        self.basic()?
                    } else {
                        self.literal()?
                    };
                    self.out[at].1 = Some(s);
                }
            }
            Some('[') => {
                self.i += 1;
                loop {
                    self.blank();
                    if self.eat(']') {
                        break;
                    }
                    self.value(path.clone())?;
                    self.blank();
                    if !self.eat(',') {
                        self.expect(']')?;
                        break;
                    }
                }
            }
            Some('{') => {
                self.i += 1;
                loop {
                    self.blank();
                    if self.eat('}') {
                        break;
                    }
                    let mut inner = path.clone();
                    inner.extend(self.key()?);
                    self.expect('=')?;
                    self.ws();
                    self.value(inner)?;
                    self.blank();
                    if !self.eat(',') {
                        self.expect('}')?;
                        break;
                    }
                }
            }
            _ => {
                let start = self.i;
                while !matches!(
                    self.peek(),
                    None | Some(',' | ']' | '}' | '\n' | '\r' | '#')
                ) {
                    self.i += 1;
                }
                if self.s[start..self.i].iter().all(|c| c.is_whitespace()) {
                    return Err(self.error("expected a value"));
                }
            }
        }
        Ok(())
    }
}
