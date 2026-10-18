use super::*;
use archgraph::diff::count;
use archgraph::model::CycleLevel;

#[derive(Args)]
pub(super) struct Report {
    /// Repository root
    #[arg(default_value = ".")]
    pub(super) root: PathBuf,
    #[command(flatten)]
    opts: ScanOpts,
    #[command(flatten)]
    hotspots: HotspotOpts,
    #[command(flatten)]
    collector: CollectorOpts,
    /// Directory for archgraph.json and archgraph.html [default: <root>/.archgraph]
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Skip bounded Git provenance probes
    #[arg(long)]
    no_provenance: bool,
}

pub(super) fn run(args: Report) -> Result<ExitCode> {
    let root = args.root.as_path();
    let rules = args.opts.rules.clone().or_else(|| {
        let found = root.join("layout.yaml");
        found.is_file().then_some(found)
    });
    let out_dir = args.out_dir.unwrap_or_else(|| root.join(".archgraph"));
    let mut doc = scan_cli::scan_doc(
        root,
        &args.opts,
        &args.hotspots,
        &args.collector,
        rules.as_deref(),
        &[&out_dir],
    )?;
    if !args.no_provenance && args.opts.collector_input.is_none() && doc.source.git.is_some() {
        provenance_cli::enrich(root, &args.collector, &mut doc)?;
    }
    let (json, html) = (
        out_dir.join("archgraph.json"),
        out_dir.join("archgraph.html"),
    );
    // The default folder lives in the scanned repository, which may ship it (or the files in it)
    // as links that would make this write land outside it.
    // ponytail: checked, not opened with O_NOFOLLOW; a link swapped in during the scan still wins.
    history::refuse_symlink(&out_dir).context("pass --out-dir to write elsewhere")?;
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    write(Some(&json), &doc)?;
    write_file(&html, render::render(&doc)?.as_bytes())?;

    if globals().brief {
        write(
            None,
            &serde_json::json!({
                "schema": SCHEMA, "kind": "report", "summary": doc.summary,
                "counts": {"files": doc.files.len(), "modules": doc.modules.len(), "imports": doc.edges.len()},
                "import_resolution": doc.import_resolution,
                "findings": doc.findings, "diagnostics": doc.diagnostics, "cycles": doc.cycles,
                "limitations": doc.source.collector.limitations,
                "artifacts": {"json": json, "html": html},
            }),
        )?;
    } else {
        let bold = globals().tty && std::env::var_os("NO_COLOR").is_none();
        print!(
            "{}",
            summary(&doc, root, rules.as_deref(), &json, &html, bold)
        );
    }
    Ok(if doc.summary.forbidden > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// The report printed after writing `json` and `html`; `bold` styles the headings.
fn summary(
    doc: &Document,
    root: &Path,
    rules: Option<&Path>,
    json: &Path,
    html: &Path,
    bold: bool,
) -> String {
    let heading = |text: &str| {
        if bold {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    let mut out = format!(
        "{}\n",
        heading(&format!("archgraph report: {}", root.display()))
    );
    match &rules {
        Some(path) => out += &format!("rules: {}\n", path.display()),
        None => out += "rules: none (pass --rules, or add layout.yaml to the root)\n",
    }
    if doc.source.collector.type_marking.as_deref() == Some("unavailable") {
        out += "note: type-only imports could not be told apart; some cycles may include imports TypeScript removes\n";
    }
    out += &format!(
        "\n{}\n  {}, {}, {}\n",
        heading("Graph"),
        count(doc.files.len(), "file"),
        count(doc.modules.len(), "module"),
        count(doc.edges.len(), "import")
    );
    if !doc.source.collector.limitations.is_empty() {
        out += &format!(
            "  {} collection limits recorded in source.collector.limitations\n",
            doc.source.collector.limitations.len()
        );
    }
    if let Some(resolution) = &doc.import_resolution {
        let percent = resolution
            .resolved_percent
            .map(|p| format!("{p:.1}%"))
            .unwrap_or_else(|| "n/a".into());
        out += &format!(
            "  {} imports, {} resolved ({percent}), {} unresolved — run archgraph unresolved for the list\n",
            resolution.imports, resolution.resolved, resolution.unresolved
        );
    } else {
        out += "  import resolution accounting unavailable in this recording; rescan with the live collector\n";
    }
    if !doc.external_nodes.is_empty() {
        out += &format!(
            "  {} known external packages (outside internal metrics)\n",
            doc.external_nodes.len()
        );
    }
    if let Some(coverage) = &doc.test_coverage {
        out += &format!(
            "  tests import {} of {} production modules; {} without importing tests\n",
            coverage.covered,
            coverage.modules,
            coverage.without_importing_tests.len()
        );
        for module in &coverage.without_importing_tests {
            out += &format!("    {module}\n");
        }
    }
    out += &format!("  {} excluded files\n", doc.config.excluded_files.len());
    out += &format!("\n{}\n", heading("External exposure"));
    if let Some(exposure) = &doc.external_exposure {
        out += &format!(
            "  advisory threshold >{}% of import occurrences; known packages include standard libraries\n",
            exposure.threshold_percent
        );
        for row in &exposure.modules {
            let percent = row
                .external_percent
                .map(|p| format!("{p:.1}%"))
                .unwrap_or_else(|| "n/a".into());
            let weighted_percent = row
                .weighted_external_percent
                .map(|p| format!("{p:.1}%"))
                .unwrap_or_else(|| "n/a".into());
            out += &format!(
                "  {}: {} packages, {}/{} imports ({percent}), weight {:.4}/{:.4} ({weighted_percent}){}\n",
                row.module,
                row.external_packages,
                row.external_imports,
                row.imports,
                row.external_weight,
                row.total_weight,
                if row.exceeds_threshold {
                    " — high exposure; review wrapper or boundary"
                } else {
                    ""
                }
            );
            if row.unresolved_imports > 0 {
                out += &format!(
                    "    {} unresolved; known external share is a lower bound\n",
                    row.unresolved_imports
                );
            }
        }
    } else {
        out += "  unavailable in this recording; rescan with import accounting\n";
    }
    if let Some(owners) = &doc.config.codeowners {
        let owned = doc.files.iter().filter(|f| !f.owners.is_empty()).count();
        let mut counts = BTreeMap::new();
        for file in &doc.files {
            for owner in &file.owners {
                *counts.entry(owner.as_str()).or_insert(0usize) += 1;
            }
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        out += &format!(
            "\n{} ({})\n  {owned} of {} files owned; shared files count for each owner\n",
            heading("Ownership"),
            owners.path,
            doc.files.len()
        );
        for (owner, n) in counts.iter().take(10) {
            out += &format!("  {owner}: {}\n", count(*n, "file"));
        }
    }
    if rules.is_some() {
        out += &format!("\n{}\n", heading("Rules"));
        for (verdict, label) in [
            (Verdict::Forbidden, "forbidden"),
            (Verdict::Unplanned, "unplanned"),
        ] {
            let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
            for e in doc
                .edges
                .iter()
                .chain(&doc.external_edges)
                .filter(|e| e.verdict == Some(verdict))
            {
                *by_rule
                    .entry(e.rule.as_deref().unwrap_or_default())
                    .or_default() += 1;
            }
            let total: usize = by_rule.values().sum();
            let mut by_rule: Vec<_> = by_rule.into_iter().collect();
            by_rule.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let rules: Vec<String> = by_rule.iter().map(|(r, n)| format!("{r} {n}")).collect();
            out += &match total {
                0 => format!("  0 {label}\n"),
                n => format!("  {n} {label}: {}\n", rules.join(", ")),
            };
        }
    }
    out += &format!("\n{}\n", heading("Cycles"));
    if !doc.type_only_cycles.is_empty() {
        out += &format!(
            "  {} type-only/deferred cycles (type, lazy or structural/mixed; excluded from runtime findings)\n",
            doc.type_only_cycles.len()
        );
    }
    for grabbag in &doc.grabbags {
        out += &format!(
            "  cycle via grab-bag {}: {} ({} genuine residual cycles)\n",
            grabbag
                .candidates
                .iter()
                .map(|c| c.module.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            grabbag.members.join(", "),
            grabbag.remaining_cycles.len()
        );
        for candidate in &grabbag.candidates {
            out += &format!(
                "    {}: cohesion {:.4}, fan-in {:.4}, internal edges/file {:.4}; split folder; involved files: {}\n",
                candidate.module,
                candidate.cohesion,
                candidate.fan_in,
                candidate.internal_edges_per_file,
                candidate.files.join(", ")
            );
        }
    }
    let module_cycles: Vec<_> = doc
        .cycles
        .iter()
        .filter(|c| c.level == CycleLevel::Module)
        .collect();
    match module_cycles.iter().map(|c| c.members.len()).max() {
        None => out += "  no module cycles\n",
        Some(largest) => {
            out += &format!(
                "  {}, largest {}\n",
                count(module_cycles.len(), "module cycle"),
                count(largest, "module")
            );
            // A registrar move leads when it beats the cycle's single-import cut and the cycle
            // shrinks; the others follow the cut.
            let single_cut = |c: &&archgraph::model::Cycle| {
                c.two_way.iter().flatten().any(|t| {
                    t.imports == 1 && !t.registry && !t.entry && !t.even && !t.resolved_by_move
                })
            };
            let mut trailing = String::new();
            for c in &module_cycles {
                for r in &c.registrar_moves {
                    let (Some(to), Some(removed)) = (&r.to, r.pairs_removed) else {
                        continue;
                    };
                    if removed <= 0 {
                        continue;
                    }
                    let out = if r.leads(single_cut(c)) {
                        &mut out
                    } else {
                        &mut trailing
                    };
                    let cycle = match (r.cycle_before, r.cycle_after) {
                        (Some(b), Some(0)) => format!("the {b}-module cycle goes"),
                        (Some(b), Some(a)) if a < b => {
                            format!("the cycle drops from {b} to {a} modules")
                        }
                        (Some(b), Some(a)) if a == b => format!("the cycle stays at {b} modules"),
                        (Some(b), Some(a)) => format!("the cycle goes from {b} to {a} modules"),
                        _ => String::new(),
                    };
                    *out += &format!(
                        "  move {} to {to}: removes {}; {cycle}; no new cycle\n",
                        r.file,
                        count(removed as usize, "two-way pair")
                    );
                }
            }
            let lightest = module_cycles
                .iter()
                .flat_map(|c| c.two_way.iter().flatten())
                .filter(|t| !t.registry && !t.entry && !t.even && !t.resolved_by_move)
                .min_by_key(|t| (t.imports, t.reverse_imports));
            let moved = module_cycles.iter().any(|c| {
                c.registrar_moves
                    .iter()
                    .any(|r| r.pairs_removed.is_some_and(|n| n > 0))
            });
            let pairs = module_cycles
                .iter()
                .any(|c| c.two_way.iter().flatten().next().is_some());
            out += &match lightest {
                Some(t) => format!(
                    "  lightest two-way cut: {} -> {}, {} ({} the other way)\n",
                    t.from,
                    t.to,
                    count(t.imports, "import"),
                    t.reverse_imports
                ),
                // The move is the step: no other cut remains.
                None if moved => String::new(),
                None if pairs => {
                    "  no lighter side to cut: the two-way pairs are even or wiring; see the cycles' break sets\n".into()
                }
                None => "  no two modules import each other; see the cycles' break sets\n".into(),
            };
            out += &trailing;
        }
    }
    out += &format!("\n{}\n", heading("Findings"));
    let mut by_kind: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &doc.findings {
        by_kind.entry(f.kind.as_str()).or_default().push(&f.subject);
    }
    let mut by_kind: Vec<_> = by_kind.into_iter().collect();
    by_kind.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    if by_kind.is_empty() {
        out += "  none\n";
    }
    for (kind, subjects) in &by_kind {
        out += &format!("  {kind} ({})\n", subjects.len());
        for subject in subjects.iter().take(3) {
            // Whole-tree findings have subject `.`: name what they are about.
            let subject = match (*kind, *subject) {
                ("seam", ".") => "clusters across folders",
                ("small-repo", ".") => "the whole source tree",
                (_, s) => s,
            };
            out += &format!("    {subject}\n");
        }
        if subjects.len() > 3 {
            out += &format!("    and {} more\n", subjects.len() - 3);
        }
    }
    if !doc.diagnostics.is_empty() {
        out += &format!("\n{}\n", heading("Diagnostics"));
        let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for diagnostic in &doc.diagnostics {
            grouped
                .entry(diagnostic.finding.kind.as_str())
                .or_default()
                .push(&diagnostic.finding.subject);
        }
        for (kind, subjects) in grouped {
            out += &format!("  {kind} ({})\n", subjects.len());
            for subject in subjects.iter().take(3) {
                out += &format!("    {subject}\n");
            }
        }
    }
    out += &format!(
        "\n{}\n  {}\n  {}\n",
        heading("Wrote"),
        json.display(),
        html.display()
    );
    out
}
