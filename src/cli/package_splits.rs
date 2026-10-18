use super::*;
use archgraph::package_splits::{self, Options};

#[derive(Args)]
pub(super) struct PackageSplitsArgs {
    /// Saved scan document (v1 or v1.1)
    graph: PathBuf,
    /// Scanned source root, used to inspect workspace/package metadata
    #[arg(long)]
    root: Option<PathBuf>,
    /// Minimum weighted cohesion after known package dependencies are externalized
    #[arg(long, default_value_t = 0.8)]
    min_cohesion: f64,
    /// Maximum distinct file dependencies to cut; zero shows free candidates only
    #[arg(long, default_value_t = 3)]
    max_cuts: usize,
    /// Maximum candidates to simulate and print
    #[arg(long, default_value_t = 20, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    limit: usize,
    #[arg(long, value_enum, default_value_t = Format::Markdown)]
    format: Format,
    /// Save an enriched graph for map island highlights and later analysis
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(super) fn run(args: PackageSplitsArgs) -> Result<()> {
    if matches!(args.format, Format::Json) && LEGACY_V1.load(Ordering::Relaxed) {
        bail!("standalone package-splits JSON requires archgraph/v1.1; omit --legacy-v1");
    }
    let mut doc = recompute(&args.graph, None)?;
    let workspace = if let Some(root) = &args.root {
        let mut verified_root = false;
        if let Some(expected) = &doc.source.git {
            let outputs: Vec<&Path> = std::iter::once(args.graph.as_path())
                .chain(args.output.as_deref())
                .collect();
            let actual =
                git(root, &outputs).context("cannot verify the workspace root's Git revision")?;
            if actual.commit != expected.commit {
                bail!(
                    "workspace root does not match the graph's Git revision; use its scanned checkout or rescan"
                );
            }
            if let Some(expected_root) = &doc.source.scan_root {
                if git_scan_root(root).as_ref() != Some(expected_root) {
                    bail!(
                        "workspace root differs from the directory recorded by the scan; use its scanned root"
                    );
                }
                verified_root = !actual.dirty && !expected.dirty;
            }
        }
        let mut inventory = package_splits::workspaces::discover(root)?;
        if !verified_root {
            inventory.checked = false;
            inventory.limitations.push("Workspace metadata is unverified: the graph lacks scan-root provenance or the scanned/current checkout is dirty. Rescan a clean checkout to establish package boundaries.".into());
        }
        inventory
    } else {
        doc.package_splits
            .as_ref()
            .map(|saved| saved.workspace.clone())
            .unwrap_or_default()
    };
    let report = package_splits::analyze(
        &doc,
        Options {
            min_cohesion: args.min_cohesion,
            max_cuts: args.max_cuts,
            limit: args.limit,
        },
        workspace,
    )?;
    doc.package_splits = Some(report);
    if let Some(path) = &args.output {
        write(Some(path), &doc)?;
    }
    let report = doc.package_splits.as_ref().unwrap();
    match args.format {
        Format::Json => write(None, report),
        Format::Markdown => {
            println!(
                "Package splits: {} candidates — import-level evidence",
                report.candidates.len()
            );
            println!("Workspace metadata checked: {}", report.workspace.checked);
            for (i, candidate) in report.candidates.iter().enumerate() {
                println!(
                    "\n{}. {} — {} / {}",
                    i + 1,
                    serde_json::to_string(&candidate.path)?,
                    serde_json::to_string(&candidate.classification)?,
                    serde_json::to_string(&candidate.readiness)?
                );
                println!(
                    "  score {} = {} importing modules × {} production files; cohesion {:.3}; {} internal edges",
                    candidate.score,
                    candidate.fan_in,
                    candidate.size,
                    candidate.cohesion,
                    candidate.internal_edges
                );
                println!("  {} dependencies to cut", candidate.cut_list.len());
                for cut in &candidate.cut_list {
                    println!(
                        "    {}{} -> {} [{}]: {}",
                        serde_json::to_string(&cut.from)?,
                        cut.line.map(|line| format!(":{line}")).unwrap_or_default(),
                        serde_json::to_string(&cut.to)?,
                        cut.kind.as_str(),
                        serde_json::to_string(&cut.reason)?
                    );
                }
                for dependency in &candidate.package_dependencies {
                    println!(
                        "  package dependency {} -> {} ({})",
                        serde_json::to_string(&dependency.from)?,
                        serde_json::to_string(&dependency.to)?,
                        serde_json::to_string(&dependency.package)?
                    );
                }
                if !candidate.external_packages.is_empty() {
                    println!(
                        "  external packages: {}",
                        serde_json::to_string(&candidate.external_packages)?
                    );
                }
                let simulation = &candidate.simulation;
                println!(
                    "  {} cycle simulation: {} -> {} remainder SCCs; verified {}; no new cycles {}",
                    simulation.scope,
                    simulation.cycles_before,
                    simulation.cycles_after,
                    simulation.verified,
                    simulation.no_new_cycles
                );
                if !simulation.boundary_cycle.is_empty() {
                    println!(
                        "  package boundary cycle: {}",
                        serde_json::to_string(&simulation.boundary_cycle)?
                    );
                }
                for note in &candidate.uncertainties {
                    println!("  review: {}", serde_json::to_string(note)?);
                }
            }
            for note in report
                .limitations
                .iter()
                .chain(&report.workspace.limitations)
            {
                println!("note: {}", serde_json::to_string(note)?);
            }
            Ok(())
        }
    }
}
