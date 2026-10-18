//! Reusable native collection sessions. Syntax is cached; shared resolution and barrel analysis
//! are recomposed so an export change cannot leave an importer's old target in the graph.
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{excludes::Policy, native};
use crate::model::Language;

pub struct Options {
    pub root: PathBuf,
    pub language: Language,
    pub source_dir: String,
    pub package: Option<String>,
    pub python_paths: Vec<PathBuf>,
    pub go: String,
    pub scan_all: bool,
}

pub struct Update {
    pub output: native::Output,
    pub parsed_files: usize,
    pub cached_files: usize,
    /// Full syntax/metadata refresh, separate from the always-global resolution composition.
    pub fallback_reason: Option<String>,
}

enum Cache {
    Typescript(Box<native::typescript::Cache>),
    Python(native::python::Cache),
    Go(native::go::Cache),
}
impl Cache {
    fn new(language: Language) -> Result<Self> {
        Ok(match language {
            Language::Typescript => Self::Typescript(Default::default()),
            Language::Python => Self::Python(Default::default()),
            Language::Go => Self::Go(Default::default()),
            _ => bail!(
                "incremental syntax collection is unavailable for {language:?}; use a full collector refresh"
            ),
        })
    }
    fn invalidate(&mut self, files: &[String]) {
        match self {
            Self::Typescript(cache) => cache.invalidate(files),
            Self::Python(cache) => cache.invalidate(files),
            Self::Go(cache) => cache.invalidate(files),
        }
    }
    fn counts(&self) -> (usize, usize) {
        match self {
            Self::Typescript(cache) => cache.counts(),
            Self::Python(cache) => cache.counts(),
            Self::Go(cache) => cache.counts(),
        }
    }
}

pub struct Session {
    options: Options,
    cache: Cache,
    inventory: BTreeSet<String>,
    initialized: bool,
}
impl Session {
    pub fn new(mut options: Options) -> Result<Self> {
        options.root = options
            .root
            .canonicalize()
            .context("resolving incremental collection root")?;
        let cache = Cache::new(options.language)?;
        Ok(Self {
            options,
            cache,
            inventory: BTreeSet::new(),
            initialized: false,
        })
    }

    /// TS resolver configuration reads, including missing paths that can later become valid.
    /// Paths are absolute, sorted, and reset alongside the collector's configuration cache.
    pub fn metadata_files(&self) -> Vec<PathBuf> {
        match &self.cache {
            Cache::Typescript(cache) => cache.metadata_files(),
            Cache::Python(_) | Cache::Go(_) => Vec::new(),
        }
    }

    pub fn collect(&mut self, policy: &Policy) -> Result<Update> {
        self.update(&[], policy)
    }

    /// Dirty paths may be absolute or root-relative. Watcher overflow must be passed as the
    /// root directory; any directory, inventory or non-source change forces a full refresh.
    pub fn update(&mut self, dirty: &[PathBuf], policy: &Policy) -> Result<Update> {
        let dirty: Vec<_> = dirty
            .iter()
            .map(|path| relative(&self.options.root, path))
            .collect::<Result<_>>()?;
        let mut fallback_reason = None;
        if self.initialized
            && dirty.iter().any(|file| {
                !self.inventory.contains(file)
                    || !self.options.root.join(file).is_file()
                    || std::fs::symlink_metadata(self.options.root.join(file))
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                    || !source_file(self.options.language, file)
            })
        {
            self.cache = Cache::new(self.options.language)?;
            fallback_reason = Some("source inventory or configuration changed; refreshing collector syntax and resolution metadata".into());
        }
        self.cache.invalidate(&dirty);
        let opts = &self.options;
        // shortcut: recompose resolution globally from cached syntax; narrow it when profiling
        // proves composition dominates and barrel/declaration invalidation is fully indexed.
        let output = match &mut self.cache {
            Cache::Typescript(cache) => native::typescript::collect_cached(
                &opts.root,
                &opts.source_dir,
                policy,
                opts.scan_all,
                cache,
            )?,
            Cache::Python(cache) => native::python::collect_cached(
                &opts.root,
                &opts.source_dir,
                opts.package.as_deref(),
                &opts.python_paths,
                policy,
                cache,
            )?,
            Cache::Go(cache) => native::go::collect_cached(&opts.root, &opts.go, policy, cache)?,
        };
        let (parsed_files, cached_files) = self.cache.counts();
        self.inventory = output.raw.files.iter().cloned().collect();
        self.initialized = true;
        Ok(Update {
            output,
            parsed_files,
            cached_files,
            fallback_reason,
        })
    }
}

fn relative(root: &Path, path: &Path) -> Result<String> {
    let path = if path.is_absolute() {
        path.strip_prefix(root)
            .context("dirty path lies outside the collection root")?
    } else {
        path
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            _ => bail!(
                "dirty path must remain inside the collection root: {}",
                path.display()
            ),
        }
    }
    // Existing symlinks are never trusted merely because their lexical path is contained.
    let absolute = root.join(&normalized);
    if absolute.exists() && !absolute.canonicalize()?.starts_with(root) {
        bail!(
            "dirty path resolves outside the collection root: {}",
            path.display()
        );
    }
    Ok(normalized.to_string_lossy().replace('\\', "/"))
}
fn source_file(language: Language, path: &str) -> bool {
    let extension = Path::new(path).extension().and_then(|part| part.to_str());
    match language {
        Language::Typescript => matches!(
            extension,
            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts")
        ),
        Language::Python => extension == Some("py"),
        Language::Go => extension == Some("go"),
        _ => false,
    }
}
