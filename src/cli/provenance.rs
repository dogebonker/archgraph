//! CLI and report enrichment for verified architecture provenance.

use super::*;
use archgraph::bisect::{self, Investigation, Options, Target};
use archgraph::model::{Evidence, Provenance};

#[derive(Args)]
pub(super) struct BisectArgs {
    /// Existing archgraph scan document; its module boundaries and rules stay fixed
    graph: PathBuf,
    /// Repository root used to produce the graph
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Zero-based finding index (defaults to the first cycle finding)
    #[arg(long, conflicts_with = "edge")]
    finding: Option<usize>,
    /// Investigate a forbidden import instead of a cycle finding
    #[arg(long, num_args = 2, value_names = ["FROM_FILE", "TO_FILE"])]
    edge: Vec<String>,
    /// Inclusive oldest commit to inspect on the first-parent chain
    #[arg(long)]
    from: Option<String>,
    /// Date window when --from is omitted
    #[arg(long, default_value = "2 years ago")]
    since: String,
    /// Maximum historical graph probes
    #[arg(long, default_value_t = 64, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(2..))]
    max_scans: usize,
    /// Optionally look up the introducing commit's PR using authenticated gh (read-only)
    #[arg(long)]
    github: bool,
    #[arg(long, value_enum, default_value_t = Format::Json)]
    format: Format,
    #[command(flatten)]
    collector: CollectorOpts,
}

/// The engine `doc` was collected with.
fn engine(doc: &Document) -> collect::Engine {
    doc.config
        .collection
        .as_ref()
        .map_or(collect::Engine::Reference, |collection| collection.engine)
}

fn collector_opts(
    root: &Path,
    options: &CollectorOpts,
    engine: collect::Engine,
    language: Language,
) -> Result<CollectorOpts> {
    let mut options = options.clone();
    if engine == collect::Engine::Native {
        if language == Language::Go && options.go.contains(['/', '\\']) {
            options.go = fs::canonicalize(&options.go)?
                .to_string_lossy()
                .into_owned();
        }
        return Ok(options);
    }
    options.depcruise = options
        .depcruise
        .as_ref()
        .map(fs::canonicalize)
        .transpose()?;
    if options.depcruise.is_none() {
        options.depcruise = fs::canonicalize(root)?
            .ancestors()
            .map(|dir| dir.join("node_modules/.bin/depcruise"))
            .find(|path| path.is_file());
    }
    options.rust_analyzer = options
        .rust_analyzer
        .as_ref()
        .map(fs::canonicalize)
        .transpose()?;
    Ok(options)
}

/// Probe with the original graph's selection and semantics, while collecting the selected
/// commit's files with `collector`'s executables.
pub(super) fn probe(
    root: &Path,
    original: &Document,
    collector: &CollectorOpts,
) -> Result<Document> {
    let config = &original.config;
    let collection = config.collection.as_ref();
    let mut collector = collector.clone();
    if let Some(collection) = collection {
        if collection.rust_build_scripts && !collector.rust_build_scripts {
            bail!(
                "graph used Rust build scripts; pass --rust-build-scripts explicitly to reproduce that execution mode"
            );
        }
        collector.rust_build_scripts = collection.rust_build_scripts;
    }
    let mut exclude = config
        .explicit_excludes
        .clone()
        .unwrap_or_else(|| config.excludes.clone());
    if let Some(rules) = &config.rules {
        exclude.extend(rules.doc.exclude.iter().cloned());
    }
    let opts = ScanOpts {
        engine: engine(original),
        depth: (!config.package_modules).then_some(config.depth),
        exclude,
        no_default_excludes: config.no_default_excludes,
        include_tests: config.include_tests,
        include_generated: config.include_generated,
        collection_includes: config.includes.clone(),
        language: Some(original.source.language),
        collector_input: None,
        package: collection.and_then(|c| c.python_package.clone()),
        python_path: collection
            .map(|c| c.python_paths.iter().map(PathBuf::from).collect())
            .unwrap_or_default(),
        rust_features: collection
            .and_then(|c| c.rust_features)
            .unwrap_or(rust::Features::All),
        rules: None,
        cochange: original.cochange.as_ref().map(|cochange| cochange.limit),
        symbols: false,
    };
    let collected = super::scan_doc(root, &opts, &HotspotOpts::default(), &collector, None, &[])?;
    let mut config = original.config.clone();
    config.file_metadata = collected.config.file_metadata;
    config.import_records = collected.imports;
    archgraph::analyze(
        collected.source,
        config,
        collected.files.into_iter().map(|file| file.path).collect(),
        collected.edges,
        collected.package_entries,
        collected.cochange,
    )
}

pub(super) fn run(args: BisectArgs) -> Result<()> {
    let doc = super::recompute(&args.graph, None)?;
    let target = if args.edge.is_empty() {
        let index = args
            .finding
            .or_else(|| {
                doc.findings
                    .iter()
                    .position(|finding| matches!(finding.evidence, Evidence::Cycle(_)))
            })
            .context("no cycle finding; select --finding INDEX or --edge FROM_FILE TO_FILE")?;
        Target::finding(&doc, index)?
    } else {
        let from = &args.edge[0];
        let to = &args.edge[1];
        if !doc.edges.iter().chain(&doc.external_edges).any(|edge| {
            edge.from == *from && edge.to == *to && edge.verdict == Some(Verdict::Forbidden)
        }) {
            bail!("{from} -> {to} is not a forbidden edge in the supplied graph");
        }
        Target::ForbiddenEdge {
            from: from.clone(),
            to: to.clone(),
        }
    };
    let options = collector_opts(
        &args.root,
        &args.collector,
        engine(&doc),
        doc.source.language,
    )?;
    let checkout = history::Checkout::new(&args.root)?;
    let mut result = bisect::investigate(
        &args.root,
        &doc,
        target,
        &Options {
            from: args.from,
            since: args.since,
            max_scans: args.max_scans,
            ..Options::default()
        },
        |oid| probe(&checkout.at(oid)?, &doc, &options),
    )?;
    if args.github
        && let Some(commit) = &mut result.introduced_by
        && commit.pull_request.is_none()
    {
        match github_pr(&args.root, &commit.commit) {
            Ok(number) => commit.pull_request = number,
            Err(error) => result
                .limitations
                .push(format!("Optional GitHub PR lookup unavailable: {error:#}")),
        }
    }
    match args.format {
        Format::Json => super::write(None, &result),
        Format::Markdown => {
            print!("{}", markdown(&result));
            Ok(())
        }
    }
}

fn provenance(result: Investigation) -> (Option<bisect::Commit>, Provenance) {
    (
        result.introduced_by,
        Provenance {
            status: result.status.into(),
            method: result.method.into(),
            from: result.from,
            to: result.to,
            previous_commit: result.previous_commit,
            closing_edge: result.closing_edge,
            limitations: result.limitations,
            scans: result.scans,
        },
    )
}

fn unavailable(doc: &Document, status: &str, limitation: &str) -> Provenance {
    Provenance {
        status: status.into(),
        method: "none".into(),
        from: None,
        to: doc
            .source
            .git
            .as_ref()
            .map(|git| git.commit.clone())
            .unwrap_or_default(),
        previous_commit: None,
        closing_edge: None,
        limitations: vec![limitation.into()],
        scans: 0,
    }
}

/// A report has one disposable checkout and a shared 64-scan budget across all findings.
/// Cached collector results can be reused by subsequent finding investigations.
pub(super) fn enrich(root: &Path, opts: &CollectorOpts, doc: &mut Document) -> Result<()> {
    enrich_from(root, opts, doc, None, &std::cell::OnceCell::new())
}

/// `enrich` from commit `from`; `setup` holds the collector options and checkout, made on first
/// use, so a caller annotating several documents clones the repository once.
pub(super) fn enrich_from(
    root: &Path,
    opts: &CollectorOpts,
    doc: &mut Document,
    from: Option<String>,
    setup: &std::cell::OnceCell<Result<(CollectorOpts, history::Checkout)>>,
) -> Result<()> {
    let mut cached: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut scans = 0;
    for index in 0..doc.findings.len() {
        let result = match setup.get_or_init(|| {
            collector_opts(root, opts, engine(doc), doc.source.language)
                .and_then(|options| Ok((options, history::Checkout::new(root)?)))
        }) {
            Err(error) => Err(anyhow::anyhow!(
                "historical checkout unavailable: {error:#}"
            )),
            Ok(_) if scans >= 64 => Err(anyhow::anyhow!(
                "shared report provenance budget exhausted (64 historical scans)"
            )),
            Ok((options, checkout)) => bisect::investigate(
                root,
                doc,
                Target::finding(doc, index)?,
                &Options {
                    from: from.clone(),
                    ..Options::default()
                },
                |oid| {
                    if let Some(bytes) = cached.get(oid) {
                        let input: ScanInput = serde_json::from_slice(bytes)?;
                        return archgraph::restore_scan(input);
                    }
                    if scans >= 64 {
                        bail!("shared report provenance budget exhausted (64 historical scans)");
                    }
                    scans += 1;
                    let graph = probe(&checkout.at(oid)?, doc, options)?;
                    cached.insert(oid.to_owned(), serde_json::to_vec(&graph)?);
                    Ok(graph)
                },
            ),
        };
        let (introduced, details) = match result {
            Ok(result) => provenance(result),
            Err(error) => (
                None,
                unavailable(
                    doc,
                    "unknown",
                    &format!("Investigation unavailable: {error:#}"),
                ),
            ),
        };
        doc.findings[index].age_days = introduced.as_ref().map(|commit| commit.age_days);
        doc.findings[index].introduced_by = introduced;
        doc.findings[index].provenance = Some(details);
    }
    Ok(())
}

fn repository(url: &str) -> Option<&str> {
    let repository = url
        .trim()
        .strip_prefix("https://github.com/")
        .or_else(|| url.trim().strip_prefix("git@github.com:"))
        .or_else(|| url.trim().strip_prefix("ssh://git@github.com/"))?;
    let repository = repository
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(repository.trim_end_matches('/'));
    let parts: Vec<_> = repository.split('/').collect();
    (parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        }))
    .then_some(repository)
}

fn github_pr(root: &Path, commit: &str) -> Result<Option<u64>> {
    let origin = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()?;
    if !origin.status.success() {
        bail!("no origin remote");
    }
    let origin = String::from_utf8(origin.stdout)?;
    let repository =
        repository(&origin).context("origin is not a supported github.com repository URL")?;
    let output = Command::new("gh")
        .args([
            "api",
            &format!("repos/{repository}/commits/{commit}/pulls"),
            "--jq",
            ".[0].number // empty",
        ])
        .output()
        .context("running gh api")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let value = String::from_utf8(output.stdout)?;
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(
            value
                .trim()
                .parse()
                .context("invalid GitHub pull request number")?,
        ))
    }
}

fn inline(value: &str) -> String {
    value.replace(['\r', '\n', '`'], " ")
}

fn markdown(result: &Investigation) -> String {
    let subject = match &result.target {
        Target::Cycle { members, .. } => format!("Cycle {}", members.join(" ↔ ")),
        Target::ForbiddenEdge { from, to } => format!("Forbidden import {from} → {to}"),
        Target::Finding {
            finding_kind,
            subject,
        } => format!("{finding_kind}: {subject}"),
    };
    let mut text = format!(
        "## {}\n\nStatus: **{}**; method: {}; {} commits, {} probes.\n",
        inline(&subject),
        result.status,
        result.method,
        result.commits,
        result.scans
    );
    if let Some(commit) = &result.introduced_by {
        text.push_str(&format!(
            "\nVerified absent → present at `{}` on {} by `{}` (`{}`), {} days ago",
            commit.commit,
            commit.committed_at,
            inline(&commit.author),
            inline(&commit.email),
            commit.age_days
        ));
        if let Some(pr) = commit.pull_request {
            text.push_str(&format!(", PR #{pr}"));
        }
        text.push_str(".\n");
        if let Some(edge) = &result.closing_edge {
            text.push_str(&format!(
                "\nClosing import: `{}` → `{}`.\n",
                inline(&edge.from),
                inline(&edge.to)
            ));
        }
        if !matches!(result.target, Target::Finding { .. }) {
            text.push_str("\nCounterfactual: archgraph-guard would have failed this change when configured to reject this new cycle or forbidden import under the supplied graph's fixed configuration. Historical CI installation was not checked.\n");
        }
    } else {
        text.push_str("\nNo introduction date or author has been verified.\n");
    }
    if !result.limitations.is_empty() {
        text.push_str("\nLimitations:\n\n");
    }
    for limitation in &result.limitations {
        text.push_str(&format!("- {}\n", inline(limitation)));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::repository;

    #[test]
    fn native_provenance_freezes_go_executable_before_historical_checkout() {
        use clap::Parser;
        let super::super::Cmd::Scan(args) = super::super::Cli::try_parse_from([
            "archgraph",
            "scan",
            ".",
            "--engine",
            "native",
            "--go",
            "./Cargo.toml",
            "--depcruise",
            "/nonexistent/unused-native-tool",
        ])
        .unwrap()
        .command
        else {
            unreachable!()
        };
        let options = super::collector_opts(
            std::path::Path::new("/nonexistent/historical-root"),
            &args.collector,
            args.opts.engine,
            archgraph::model::Language::Go,
        )
        .unwrap();
        assert_eq!(
            std::path::Path::new(&options.go),
            std::fs::canonicalize("Cargo.toml").unwrap()
        );
        let options = super::collector_opts(
            std::path::Path::new("/nonexistent/historical-root"),
            &args.collector,
            args.opts.engine,
            archgraph::model::Language::Typescript,
        )
        .unwrap();
        assert_eq!(options.go, "./Cargo.toml");
    }

    #[test]
    fn saved_collection_settings_do_not_authorize_rust_code_execution() {
        use clap::Parser;
        let source = serde_json::from_value(serde_json::json!({"language":"rust", "collector":{"name":"rust-analyzer","version":null,"cruised":1},"git":null,"generated_at":"test"})).unwrap();
        let config = serde_json::from_value(serde_json::json!({"depth":2,"source_root":".","excludes":[],"include_tests":false,"rules":null,"collection":{"python_package":null,"rust_features":"default","rust_build_scripts":true}})).unwrap();
        let graph = archgraph::analyze(
            source,
            config,
            vec!["src/lib.rs".into()],
            vec![],
            vec![],
            None,
        )
        .unwrap();
        let super::super::Cmd::Scan(args) =
            super::super::Cli::try_parse_from(["archgraph", "scan", "."])
                .unwrap()
                .command
        else {
            unreachable!()
        };
        let error = super::probe(std::path::Path::new("."), &graph, &args.collector).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("pass --rust-build-scripts explicitly")
        );
    }

    #[test]
    fn bisect_and_impact_graphs_reject_options_they_would_ignore() {
        use clap::{Parser, error::ErrorKind};
        let parse = |args: &[&str]| super::super::Cli::try_parse_from(args).map(|_| ());
        let error = |args: &[&str]| parse(args).unwrap_err().kind();
        // Bisect reproduces the saved graph's selection; only collector executables apply.
        for flag in [
            "--depth=1",
            "--include-tests",
            "--rules=x",
            "--engine=native",
        ] {
            assert_eq!(
                error(&["archgraph", "bisect", "g.json", flag]),
                ErrorKind::UnknownArgument
            );
        }
        parse(&["archgraph", "bisect", "g.json", "--go=go", "--depcruise=d"]).unwrap();
        // Impact with --graph reuses its selection; hotspots and collectors still apply.
        for flag in ["--depth=1", "--include-tests", "--language=go", "--symbols"] {
            assert_eq!(
                error(&["archgraph", "impact", "a..b", "--graph=g.json", flag]),
                ErrorKind::ArgumentConflict
            );
        }
        parse(&[
            "archgraph",
            "impact",
            "a..b",
            "--graph=g.json",
            "--hotspots",
            "--go=go",
        ])
        .unwrap();
        parse(&["archgraph", "impact", "a..b", "--depth=1"]).unwrap();
        // Scans accept every option.
        for command in ["scan", "report", "history"] {
            parse(&[
                "archgraph",
                command,
                ".",
                "--engine=native",
                "--depth=1",
                "--exclude=x",
                "--no-default-excludes",
                "--include-tests",
                "--include-generated",
                "--language=go",
                "--collector-input=x",
                "--package=p",
                "--python-path=p",
                "--rust-features=default",
                "--rules=x",
                "--cochange",
                "--symbols",
                "--hotspots",
                "--hotspot-authors",
                "--hotspot-max-files=5",
                "--depcruise=d",
                "--python=p",
                "--go=g",
                "--java=j",
                "--elixir=e",
                "--rust-analyzer=r",
                "--rust-build-scripts",
            ])
            .unwrap();
        }
    }

    #[test]
    fn github_origin_is_validated_before_api_lookup() {
        assert_eq!(
            repository("https://github.com/team/project.git\n"),
            Some("team/project")
        );
        assert_eq!(
            repository("git@github.com:team/project.git"),
            Some("team/project")
        );
        assert!(repository("https://example.com/team/project").is_none());
        assert!(repository("https://github.com/team/project/extra").is_none());
        assert!(repository("https://github.com/team/project?token=secret").is_none());
    }
    #[test]
    fn advisory_finding_does_not_claim_a_guard_failure() {
        let result = super::Investigation {
            schema: "archgraph/v1",
            kind: "bisect",
            target: super::Target::Finding {
                finding_kind: "misplaced-file".into(),
                subject: "src/a.ts".into(),
            },
            status: "verified-transition",
            method: "scan-bisect",
            from: None,
            to: "new".into(),
            commits: 2,
            scans: 2,
            introduced_by: Some(archgraph::bisect::Commit {
                commit: "new".into(),
                author: "Author".into(),
                email: "author@example.invalid".into(),
                authored_at: "2026-01-01T00:00:00Z".into(),
                committed_at: "2026-01-01T00:00:00Z".into(),
                subject: "move file".into(),
                pull_request: None,
                age_days: 1,
            }),
            previous_commit: Some("old".into()),
            closing_edge: None,
            limitations: vec![],
        };
        let markdown = super::markdown(&result);
        assert!(markdown.contains("Verified absent → present"));
        assert!(!markdown.contains("guard would have failed"));
    }
}
