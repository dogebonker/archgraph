use super::*;

#[derive(Clone, Default, serde::Serialize)]
struct Timings {
    collect_ms: f64,
    normalize_ms: f64,
    analyze_ms: f64,
    provenance_ms: f64,
    publish_ms: f64,
    /// Initial standalone scan includes both collection and analysis.
    full_scan_ms: f64,
}

struct Selection {
    inventory: Vec<String>,
    kept: BTreeSet<String>,
    excluded: Vec<String>,
}

pub(super) struct Warm {
    pub(super) args: Serve,
    pub(super) doc: Arc<Document>,
    pub(super) previous: Option<Arc<Document>>,
    pub(super) session: Option<collect::incremental::Session>,
    pub(super) analysis: archgraph::incremental::Analysis,
    pub(super) graph: PathBuf,
    pub(super) generation: u64,
    pub(super) parsed: usize,
    pub(super) fallback: Option<String>,
    pub(super) refresh_ms: f64,
    dirty_proof: bool,
    raw_fingerprint: Option<Vec<u8>>,
    timings: Timings,
    selection: Option<Selection>,
}
impl Warm {
    pub(super) fn cold(mut args: Serve) -> Result<Self> {
        let started = Instant::now();
        let mut timings = Timings::default();
        args.root = args.root.canonicalize()?;
        configure_index(&args.root)?;
        args.opts.rules = args
            .opts
            .rules
            .map(fs::canonicalize)
            .transpose()?
            .or_else(|| {
                let path = args.root.join("layout.yaml");
                path.is_file().then_some(path)
            });
        let graph = args.root.join(".archgraph/daemon-graph.json");
        let scan_started = Instant::now();
        let mut doc = scan_cli::scan_doc(
            &args.root,
            &args.opts,
            &args.hotspots,
            &args.collector,
            args.opts.rules.as_deref(),
            &[&args.root.join(".archgraph")],
        )?;
        timings.full_scan_ms = scan_started.elapsed().as_secs_f64() * 1000.0;
        let mut raw_fingerprint = None;
        let mut selection = None;
        let mut parsed = 0;
        let session = if args.opts.engine == collect::Engine::Native
            && matches!(
                doc.source.language,
                Language::Typescript | Language::Python | Language::Go
            ) {
            let mut session = collect::incremental::Session::new(collect::incremental::Options {
                root: args.root.clone(),
                language: doc.source.language,
                source_dir: collect::source_dir(&args.root).into(),
                package: args.opts.package.clone(),
                python_paths: args.opts.python_path.clone(),
                go: args.collector.go.clone(),
                scan_all: args.opts.include_tests
                    || args.opts.include_generated
                    || args.opts.no_default_excludes
                    || !doc.config.includes.is_empty(),
            })?;
            let policy = policy(&args.root, &doc.config, doc.source.language, &args.opts)?;
            let collect_started = Instant::now();
            let update = session.collect(&policy)?;
            timings.collect_ms = collect_started.elapsed().as_secs_f64() * 1000.0;
            parsed = update.parsed_files;
            let normalize_started = Instant::now();
            let mut output = update.output;
            let excluded = normalize(&mut output, &policy, &mut selection)?;
            raw_fingerprint = Some(fingerprint(&output.raw, &excluded)?);
            timings.normalize_ms = normalize_started.elapsed().as_secs_f64() * 1000.0;
            let analyze_started = Instant::now();
            let config = native_config(&doc.config, &args.opts, &mut output, excluded);
            let mut source = doc.source.clone();
            source.collector = output.raw.collector;
            source.git = git(&args.root, &[&args.root.join(".archgraph")]);
            source.generated_at = scan_cli::source_time(&args.root)?;
            let entries = if source.language == Language::Typescript {
                collect::depcruise::package_entries(&args.root, &output.raw.files)
            } else {
                Vec::new()
            };
            let cochange = args
                .opts
                .cochange
                .map(|count| archgraph::cochange::collect(&args.root, count, &output.raw.files))
                .transpose()?;
            doc = archgraph::analyze(
                source,
                config,
                output.raw.files,
                output.raw.edges,
                entries,
                cochange,
            )?;
            if args.opts.symbols {
                archgraph::surface::attach(
                    &mut doc,
                    output.symbols,
                    archgraph::surface::DEFAULT_MAX_SYMBOLS,
                )?;
            }
            if let Some(since) = &args.hotspots.hotspots {
                let report = archgraph::hotspots::collect(
                    &args.root,
                    &doc,
                    &archgraph::hotspots::Options {
                        since: since.clone(),
                        max_files: args.hotspots.hotspot_max_files,
                        authors: args.hotspots.hotspot_authors,
                    },
                )?;
                archgraph::apply_hotspots(&mut doc, report);
            }
            timings.analyze_ms += analyze_started.elapsed().as_secs_f64() * 1000.0;
            Some(session)
        } else {
            None
        };
        let analyze_started = Instant::now();
        let module_of: BTreeMap<_, _> = doc
            .files
            .iter()
            .map(|file| (file.path.clone(), file.module.clone()))
            .collect();
        let analysis = archgraph::incremental::Analysis::new(
            module_of.clone(),
            doc.edges.clone(),
            archgraph::incremental::Options {
                entries: modules::cycle_entries(
                    module_of.keys(),
                    &doc.config.source_root,
                    &doc.package_entries,
                ),
                barrels: doc
                    .files
                    .iter()
                    .filter(|file| file.barrel)
                    .map(|file| file.path.clone())
                    .collect(),
                tests: doc
                    .files
                    .iter()
                    .filter(|file| modules::is_test(&file.path, &doc.config))
                    .map(|file| file.path.clone())
                    .collect(),
                directory_units: doc.source.language == Language::Go,
                all_kinds: doc.config.all_kinds,
            },
        )?;
        timings.analyze_ms += analyze_started.elapsed().as_secs_f64() * 1000.0;
        let dirty_proof = doc.source.git.is_some()
            && archgraph::git::has_tracked_changes(
                &args.root,
                &doc.files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap_or(false);
        let mut value = Self {
            args,
            doc: Arc::new(doc),
            previous: None,
            session,
            analysis,
            graph,
            generation: 1,
            parsed,
            fallback: None,
            refresh_ms: 0.0,
            dirty_proof,
            raw_fingerprint,
            timings,
            selection,
        };
        let publish_started = Instant::now();
        write_file(
            &value.graph,
            &(serde_json::to_string_pretty(&*value.doc)? + "\n").into_bytes(),
        )?;
        value.timings.publish_ms = publish_started.elapsed().as_secs_f64() * 1000.0;
        value.refresh_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(value)
    }

    pub(super) fn refresh(&mut self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        self.fallback = None;
        self.timings = Timings::default();
        // Non-source changes can alter rules, entrypoints, ownership, aliases or Git evidence.
        let triggers: Vec<_> = paths
            .iter()
            .filter_map(|path| {
                let reason = if !path.starts_with(&self.args.root) {
                    "outside scan root"
                } else if !path.is_file() {
                    "directory, deleted file or watcher rescan"
                } else if self
                    .args
                    .opts
                    .rules
                    .as_ref()
                    .is_some_and(|rules| path == rules)
                {
                    "layout rules"
                } else if self
                    .args
                    .opts
                    .collector_input
                    .as_ref()
                    .is_some_and(|input| path == input)
                {
                    "recorded collector input"
                } else if !source_file(self.doc.source.language, path) {
                    "non-source metadata"
                } else {
                    return None;
                };
                Some((path.clone(), reason))
            })
            .collect();
        if !triggers.is_empty() {
            let reasons: BTreeSet<_> = triggers.iter().map(|(_, reason)| *reason).collect();
            let paths: Vec<_> = triggers.iter().map(|(path, _)| path.clone()).collect();
            return self.full_refresh(
                started,
                &reasons.into_iter().collect::<Vec<_>>().join(", "),
                &paths,
            );
        }
        if self.session.is_none() {
            return self.full_refresh(
                started,
                "collector does not support cached native syntax",
                paths,
            );
        }
        if self.doc.hotspots.is_some() {
            return self.full_refresh(
                started,
                "hotspot evidence requires collection and history refresh",
                paths,
            );
        }
        let collect_started = Instant::now();
        let policy = policy(
            &self.args.root,
            &self.doc.config,
            self.doc.source.language,
            &self.args.opts,
        )?;
        let update = self.session.as_mut().unwrap().update(paths, &policy)?;
        self.timings.collect_ms = collect_started.elapsed().as_secs_f64() * 1000.0;
        self.parsed = update.parsed_files;
        self.fallback = update.fallback_reason;
        // A new source file may change package discovery; use the full configuration path.
        if let Some(reason) = self.fallback.clone() {
            return self.full_refresh(started, &reason, paths);
        }
        let normalize_started = Instant::now();
        let mut output = update.output;
        let excluded = normalize(&mut output, &policy, &mut self.selection)?;
        let fingerprint = fingerprint(&output.raw, &excluded)?;
        let unchanged = self.raw_fingerprint.as_ref() == Some(&fingerprint);
        self.timings.normalize_ms = normalize_started.elapsed().as_secs_f64() * 1000.0;
        let config = native_config(&self.doc.config, &self.args.opts, &mut output, excluded);
        let raw = output.raw;
        let mut source = self.doc.source.clone();
        source.collector = raw.collector;
        // Content edits cannot change the deterministic HEAD timestamp; Git/config events refresh it.
        let provenance_started = Instant::now();
        let changed_paths: Vec<_> = paths
            .iter()
            .filter_map(|path| {
                path.strip_prefix(&self.args.root)
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
            })
            .collect();
        if let Some(provenance) = &mut source.git
            && archgraph::git::has_tracked_changes(&self.args.root, &changed_paths)
                .is_ok_and(|changed| changed)
        {
            // A tracked dirty source proves the checkout dirty; a negative result cannot prove it clean.
            provenance.dirty = true;
            self.dirty_proof = true;
        } else {
            source.git = git(&self.args.root, &[&self.args.root.join(".archgraph")]);
            self.dirty_proof = false;
        }
        self.timings.provenance_ms = provenance_started.elapsed().as_secs_f64() * 1000.0;
        let analyze_started = Instant::now();
        let mut doc = if unchanged {
            let mut doc = (*self.doc).clone();
            doc.source = source;
            doc.config = config;
            doc
        } else {
            eprintln!(
                "daemon: global findings fallback; SCCs and metrics reuse closed dirty dependency regions"
            );
            archgraph::analyze_cached(
                source,
                config,
                raw.files,
                raw.edges,
                self.doc.package_entries.clone(),
                self.doc.cochange.clone(),
                &mut self.analysis,
            )?
        };
        if self.args.opts.symbols {
            archgraph::surface::attach(
                &mut doc,
                output.symbols,
                archgraph::surface::DEFAULT_MAX_SYMBOLS,
            )?;
        }
        self.timings.analyze_ms = analyze_started.elapsed().as_secs_f64() * 1000.0;
        self.install(doc, started, true)?;
        self.raw_fingerprint = Some(fingerprint);
        Ok(())
    }

    fn full_refresh(&mut self, started: Instant, reason: &str, paths: &[PathBuf]) -> Result<()> {
        let shown = paths
            .iter()
            .take(5)
            .map(|path| {
                let relative = path.strip_prefix(&self.args.root).unwrap_or(path);
                format!(
                    "{:?}",
                    if relative.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        relative
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let remainder = if paths.len() > 5 {
            format!(" (+{} more)", paths.len() - 5)
        } else {
            String::new()
        };
        self.fallback = Some(format!("{reason}; dirty paths: [{shown}]{remainder}"));
        eprintln!(
            "daemon: full collection fallback: {}",
            self.fallback.as_deref().unwrap()
        );
        let replacement = Self::cold(Serve {
            root: self.args.root.clone(),
            socket: self.args.socket.clone(),
            port: self.args.port,
            once: self.args.once,
            opts: self.args.opts.clone(),
            hotspots: self.args.hotspots.clone(),
            collector: self.args.collector.clone(),
        })?;
        self.parsed = replacement.parsed;
        self.dirty_proof = replacement.dirty_proof;
        self.session = replacement.session;
        self.analysis = replacement.analysis;
        self.raw_fingerprint = replacement.raw_fingerprint;
        self.selection = replacement.selection;
        let prior_collect_ms = self.timings.collect_ms;
        self.timings = replacement.timings;
        self.timings.collect_ms += prior_collect_ms;
        // cold already published the replacement atomically.
        self.install((*replacement.doc).clone(), started, false)
    }

    fn install(&mut self, doc: Document, started: Instant, publish: bool) -> Result<()> {
        if publish {
            let publish_started = Instant::now();
            write_file(
                &self.graph,
                &(serde_json::to_string_pretty(&doc)? + "\n").into_bytes(),
            )?;
            self.timings.publish_ms += publish_started.elapsed().as_secs_f64() * 1000.0;
        }
        self.previous = Some(std::mem::replace(&mut self.doc, Arc::new(doc)));
        self.generation += 1;
        self.refresh_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(())
    }

    pub(super) fn activate(&self) {
        ACTIVE.with(|active| {
            *active.borrow_mut() = Some(Active {
                root: self.args.root.clone(),
                graph: self.graph.clone(),
                key: key(
                    &self.args.opts,
                    &self.args.hotspots,
                    &self.args.collector,
                    self.args.opts.rules.as_deref(),
                ),
                doc: self.doc.clone(),
                dirty_proof:self.dirty_proof,
                stats: json!({"generation": self.generation, "parsed_files": self.parsed,
                "refresh_ms": self.refresh_ms, "fallback_reason": self.fallback,
                "cached_snapshots": 1 + usize::from(self.previous.is_some()), "timings": self.timings}),
            })
        });
    }
}

fn native_config(
    previous: &archgraph::model::Config,
    opts: &ScanOpts,
    output: &mut collect::native::Output,
    excluded: Vec<String>,
) -> archgraph::model::Config {
    let mut config = previous.clone();
    config.source_root.clone_from(&output.source_root);
    config.file_metadata = std::mem::take(&mut output.raw.metadata);
    config.import_records = output.raw.imports.take();
    config.excluded_files = excluded;
    config.include_tests = opts.include_tests
        || opts.no_default_excludes
        || output
            .raw
            .files
            .iter()
            .any(|path| collect::excludes::is_test(path));
    config
}

fn policy(
    root: &Path,
    config: &archgraph::model::Config,
    language: Language,
    opts: &ScanOpts,
) -> Result<collect::excludes::Policy> {
    collect::excludes::Policy::new(
        root,
        language,
        collect::excludes::Options {
            no_defaults: config.no_default_excludes,
            // config.include_tests also records selectively included tests; it is not the flag.
            include_tests: opts.include_tests,
            include_generated: opts.include_generated,
            cli_excludes: config.explicit_excludes.clone().unwrap_or_default(),
            excludes: config
                .rules
                .as_ref()
                .map(|rules| rules.doc.exclude.clone())
                .unwrap_or_default(),
            includes: config.includes.clone(),
        },
    )
}

/// Keep this identical to scan_doc's native post-collection selection and symbol accounting.
fn normalize(
    output: &mut collect::native::Output,
    policy: &collect::excludes::Policy,
    selection: &mut Option<Selection>,
) -> Result<Vec<String>> {
    if output
        .raw
        .imports
        .as_ref()
        .is_some_and(|imports| imports.iter().any(|import| import.unresolved.is_some()))
    {
        output.symbols.complete = false;
        output.symbols.limitations.push("Unresolved requests can hide local symbol consumers; unused-export suggestions are withheld.".into());
    }
    output.symbols.limitations.sort();
    output.symbols.limitations.dedup();
    output
        .raw
        .collector
        .limitations
        .clone_from(&output.symbols.limitations);
    if selection
        .as_ref()
        .is_none_or(|selection| selection.inventory != output.raw.files)
    {
        let kept: BTreeSet<_> = policy.select(&output.raw.files)?.into_iter().collect();
        let excluded = output
            .raw
            .files
            .iter()
            .filter(|path| !kept.contains(*path))
            .cloned()
            .collect();
        *selection = Some(Selection {
            inventory: output.raw.files.clone(),
            kept,
            excluded,
        });
    }
    // Policy changes use full_refresh, which replaces this selection along with the session.
    let selection = selection.as_ref().unwrap();
    let kept = &selection.kept;
    output.raw.files.retain(|path| kept.contains(path));
    output
        .raw
        .edges
        .retain(|edge| kept.contains(&edge.from) && kept.contains(&edge.to));
    if let Some(imports) = &mut output.raw.imports {
        imports.retain(|import| kept.contains(&import.from));
    }
    output
        .symbols
        .exports
        .retain(|export| kept.contains(&export.file) && kept.contains(&export.target.file));
    output.symbols.imports.retain(|import| {
        kept.contains(&import.from)
            && kept.contains(&import.to)
            && import
                .target
                .as_ref()
                .is_none_or(|target| kept.contains(&target.file))
    });
    Ok(selection.excluded.clone())
}
fn fingerprint(raw: &collect::RawGraph, excluded: &[String]) -> Result<Vec<u8>> {
    let metadata: BTreeMap<_, _> = raw
        .metadata
        .iter()
        .map(|(path, value)| (path, (value.barrel, value.test)))
        .collect();
    Ok(serde_json::to_vec(&(
        &raw.files,
        &raw.edges,
        &raw.imports,
        metadata,
        excluded,
    ))?)
}
fn source_file(language: Language, path: &Path) -> bool {
    let extension = path.extension().and_then(|part| part.to_str());
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

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let mut value = Self(std::env::temp_dir().join(format!(
                "archgraph-warm-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )));
            value.write(".archgraph/placeholder", "");
            value.write(
                "a/a.ts",
                "import { value } from '../b/b'; export const result = value;\n",
            );
            value.write("b/b.ts", "export const value = 1;\n");
            value.write(
                "tests/hidden.test.ts",
                "import { result } from '../a/a'; export const hidden = result;\n",
            );
            value.write(
                "tests/chosen.test.ts",
                "import { result } from '../a/a'; export const chosen = result;\n",
            );
            value.write("policy.yaml", "version: 1\n");
            value.0 = value.0.canonicalize().unwrap();
            value
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn warm(&self) -> Warm {
            let cli = Cli::try_parse_from([
                "archgraph",
                "serve",
                self.0.to_str().unwrap(),
                "--engine",
                "native",
                "--symbols",
                "--rules",
                self.0.join("policy.yaml").to_str().unwrap(),
            ])
            .unwrap();
            let Cmd::Serve(args) = cli.command else {
                unreachable!()
            };
            Warm::cold(args).unwrap()
        }
        fn fresh(&self, warm: &Warm) -> Document {
            scan_cli::scan_doc(
                &self.0,
                &warm.args.opts,
                &warm.args.hotspots,
                &warm.args.collector,
                warm.args.opts.rules.as_deref(),
                &[&self.0.join(".archgraph")],
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn assert_fresh(fixture: &Fixture, warm: &Warm) {
        assert_eq!(
            serde_json::to_value(&*warm.doc).unwrap(),
            serde_json::to_value(fixture.fresh(warm)).unwrap()
        );
    }

    #[test]
    fn warm_normalization_keeps_excluded_files_out_and_reuses_unchanged_architecture() {
        let fixture = Fixture::new();
        let mut warm = fixture.warm();
        let fingerprint = warm.raw_fingerprint.clone();
        assert!(!warm.analysis.components().is_empty());
        fixture.write("b/b.ts", "export const value = 2;\n");
        warm.refresh(&[fixture.0.join("b/b.ts")]).unwrap();
        assert_eq!(warm.parsed, 1);
        assert!(warm.timings.collect_ms > 0.0);
        assert!(warm.timings.normalize_ms > 0.0);
        assert!(warm.timings.analyze_ms > 0.0);
        assert!(warm.timings.publish_ms > 0.0);
        assert_eq!(warm.timings.full_scan_ms, 0.0);
        assert_eq!(warm.raw_fingerprint, fingerprint);
        assert!(
            warm.doc
                .files
                .iter()
                .all(|file| !file.path.starts_with("tests/"))
        );
        assert!(
            warm.doc
                .symbols
                .as_ref()
                .unwrap()
                .exports
                .iter()
                .all(|export| !export.file.starts_with("tests/"))
        );
        assert_fresh(&fixture, &warm);
        fixture.write(
            "a/a.ts",
            "\nimport { value } from '../b/b'; export const result = value;\n",
        );
        warm.refresh(&[fixture.0.join("a/a.ts")]).unwrap();
        assert_ne!(warm.raw_fingerprint, fingerprint);
        assert_eq!(warm.doc.edges[0].line, Some(2));
        assert_fresh(&fixture, &warm);
    }

    #[test]
    fn warm_selective_test_include_does_not_disable_other_test_exclusions() {
        let fixture = Fixture::new();
        fixture.write(
            "policy.yaml",
            "version: 1\ninclude: [tests/chosen.test.ts]\n",
        );
        let mut warm = fixture.warm();
        assert!(warm.doc.config.include_tests);
        fixture.write("b/b.ts", "export const value = 2;\n");
        warm.refresh(&[fixture.0.join("b/b.ts")]).unwrap();
        assert!(
            warm.doc
                .files
                .iter()
                .any(|file| file.path == "tests/chosen.test.ts")
        );
        assert!(
            warm.doc
                .files
                .iter()
                .all(|file| file.path != "tests/hidden.test.ts")
        );
        assert_fresh(&fixture, &warm);
    }

    #[test]
    fn warm_cached_selection_preserves_archignore_and_refreshes_when_it_changes() {
        let fixture = Fixture::new();
        fixture.write("unused/hidden.ts", "export const hidden = 1;\n");
        fixture.write("unused/included.ts", "export const included = 1;\n");
        fixture.write(".archgraphignore", "unused/*\n!unused/included.ts\n");
        let mut warm = fixture.warm();
        let inventory = warm.selection.as_ref().unwrap().inventory.clone();
        fixture.write("b/b.ts", "export const value = 2;\n");
        warm.refresh(&[fixture.0.join("b/b.ts")]).unwrap();
        assert_eq!(warm.selection.as_ref().unwrap().inventory, inventory);
        assert!(
            warm.doc
                .files
                .iter()
                .any(|file| file.path == "unused/included.ts")
        );
        assert!(
            warm.doc
                .files
                .iter()
                .all(|file| file.path != "unused/hidden.ts")
        );
        assert_fresh(&fixture, &warm);
        fixture.write(".archgraphignore", "unused/*\n");
        warm.refresh(&[fixture.0.join(".archgraphignore")]).unwrap();
        assert!(
            warm.doc
                .files
                .iter()
                .all(|file| !file.path.starts_with("unused/"))
        );
        assert_fresh(&fixture, &warm);
    }

    #[test]
    fn warm_tracked_dirty_proof_preserves_full_checkout_scope_on_revert() {
        let fixture = Fixture::new();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&fixture.0)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Archgraph Test",
                    "-c",
                    "user.email=archgraph@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet", "--template="]);
        git(&["add", "."]);
        git(&["commit", "--quiet", "-m", "fixture"]);
        let mut warm = fixture.warm();
        assert!(!warm.doc.source.git.as_ref().unwrap().dirty);
        let commit = warm.doc.source.git.as_ref().unwrap().commit.clone();
        fixture.write("b/b.ts", "export const value = 2;\n");
        warm.refresh(&[fixture.0.join("b/b.ts")]).unwrap();
        assert!(warm.doc.source.git.as_ref().unwrap().dirty);
        assert_eq!(warm.doc.source.git.as_ref().unwrap().commit, commit);
        assert_fresh(&fixture, &warm);
        fixture.write("b/b.ts", "export const value = 1;\n");
        warm.refresh(&[fixture.0.join("b/b.ts")]).unwrap();
        assert!(!warm.doc.source.git.as_ref().unwrap().dirty);
        assert_fresh(&fixture, &warm);
        fixture.write("notes.md", "untracked unrelated content\n");
        // A negative tracked-path proof must still discover unrelated untracked files.
        warm.refresh(&[fixture.0.join("b/b.ts")]).unwrap();
        assert!(warm.doc.source.git.as_ref().unwrap().dirty);
        assert_fresh(&fixture, &warm);
    }

    #[test]
    fn warm_custom_rules_reload_instead_of_retaining_old_verdicts() {
        let fixture = Fixture::new();
        let mut warm = fixture.warm();
        assert_eq!(warm.doc.summary.forbidden, 0);
        fixture.write(
            "policy.yaml",
            "version: 1\nforbid:\n  - {from: 'a/**', to: 'b/**'}\n",
        );
        warm.refresh(&[fixture.0.join("policy.yaml")]).unwrap();
        assert_eq!(warm.doc.summary.forbidden, 1);
        assert!(warm.fallback.as_ref().unwrap().contains("layout rules"));
        assert!(warm.fallback.as_ref().unwrap().contains("policy.yaml"));
        assert_fresh(&fixture, &warm);
    }
}
