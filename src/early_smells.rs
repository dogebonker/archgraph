//! Optional, reasoned limits for the first architecture check.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{Document, ModuleClass};
use crate::surface::SymbolTarget;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limit<T> {
    pub max: T,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Switch {
    pub enabled: bool,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Thresholds {
    pub file_size: Limit<usize>,
    pub fan_in: Limit<usize>,
    /// Percentage of production files, from 0 (exclusive) through 100.
    pub module_reach: Limit<f64>,
    pub single_implementation: Switch,
    pub every_file_in_layer: Switch,
}
impl Default for Thresholds {
    fn default() -> Self {
        Self {
            file_size: Limit {
                max: 500,
                reason: "Keep files small enough to review.".into(),
            },
            fan_in: Limit {
                max: 30,
                reason: "Keep changes from affecting too many consumers.".into(),
            },
            module_reach: Limit {
                max: 50.0,
                reason: "Keep module dependencies focused.".into(),
            },
            single_implementation: Switch {
                enabled: true,
                reason: "Review abstractions with only one observed implementation.".into(),
            },
            every_file_in_layer: Switch {
                enabled: true,
                reason: "Give every production file an architectural home.".into(),
            },
        }
    }
}
impl Thresholds {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.file_size.max > 0,
            "early-smells.file-size.max must be positive"
        );
        ensure!(
            self.fan_in.max > 0,
            "early-smells.fan-in.max must be positive"
        );
        ensure!(
            self.module_reach.max.is_finite()
                && self.module_reach.max > 0.0
                && self.module_reach.max <= 100.0,
            "early-smells.module-reach.max must be finite and in (0, 100]"
        );
        for reason in [
            &self.file_size.reason,
            &self.fan_in.reason,
            &self.module_reach.reason,
            &self.single_implementation.reason,
            &self.every_file_in_layer.reason,
        ] {
            ensure!(
                !reason.trim().is_empty() && !reason.chars().any(char::is_control),
                "early-smells rules require a nonempty reason without control characters"
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: String,
    pub file: String,
    pub reason: String,
    pub evidence: String,
    pub confidence: String,
    pub blocking: bool,
    pub recipe: Value,
}
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub verdict: String,
    pub blocking_count: usize,
    pub violations: Vec<Violation>,
    pub limitations: Vec<String>,
}
impl Report {
    fn add(
        &mut self,
        rule: &str,
        file: &str,
        reason: &str,
        evidence: String,
        instruction: &str,
        blocking: bool,
    ) {
        self.blocking_count += usize::from(blocking);
        self.violations.push(Violation {
            rule: rule.into(), file: file.into(), reason: reason.into(), evidence,
            confidence: if blocking { "high" } else { "medium" }.into(), blocking,
            recipe: json!({"mechanical":false,"steps":[{"kind":"instruction","instruction":instruction}]}),
        });
    }
}

/// Saved graphs support graph limits; source-dependent checks require an explicit live root.
pub fn check(doc: &Document, root: Option<&Path>) -> Result<Report> {
    let mut report = Report {
        verdict: "PASS".into(),
        blocking_count: 0,
        violations: vec![],
        limitations: vec![],
    };
    let Some(rules) = doc.config.rules.as_ref() else {
        return Ok(report);
    };
    let Some(limits) = rules.doc.early_smells.as_ref() else {
        return Ok(report);
    };
    limits.validate()?;
    let compiled = crate::rules::Compiled::new(&rules.doc)?;
    let files: BTreeMap<_, _> = doc
        .files
        .iter()
        .filter(|f| f.class != Some(ModuleClass::Tests))
        .map(|f| (f.path.as_str(), f))
        .collect();
    let mut fan_in: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut reach: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in &doc.edges {
        if let (Some(from), Some(to)) = (files.get(edge.from.as_str()), files.get(edge.to.as_str()))
        {
            if from.path != to.path {
                fan_in.entry(&to.path).or_default().insert(&from.path);
            }
            if from.module != to.module {
                reach.entry(&to.module).or_default().insert(&from.path);
            }
        }
    }
    for (file, importers) in fan_in {
        if importers.len() > limits.fan_in.max {
            report.add("fan-in", file, &limits.fan_in.reason, format!("{} distinct production importing files exceed {}", importers.len(), limits.fan_in.max), "Narrow the file's responsibilities or move consumers behind focused public entrypoints; rerun check.", true);
        }
    }
    for (module, importers) in reach {
        let percent = 100.0 * importers.len() as f64 / files.len().max(1) as f64;
        if percent > limits.module_reach.max {
            report.add("module-reach", module, &limits.module_reach.reason, format!("{}/{} production files outside the module import it ({percent:.2}%), exceeding {}%", importers.len(), files.len(), limits.module_reach.max), "Split unrelated module responsibilities or review and explain this module's threshold; rerun check.", true);
        }
    }
    let root = root.map(Path::canonicalize).transpose()?;
    let mut syntax = BTreeMap::new();
    for (path, _) in files {
        if limits.every_file_in_layer.enabled && compiled.layer_for(path).is_none() {
            report.add("every-file-in-layer", path, &limits.every_file_in_layer.reason, "Production file matches no declared layer".into(), "Move the file into an intended layer or add its reviewed path to layout.yaml layers; rerun check.", true);
        }
        if let Some(root) = &root {
            let relative = Path::new(path);
            ensure!(
                !path.is_empty()
                    && !path.contains('\\')
                    && !path.contains(':')
                    && !path.chars().any(char::is_control)
                    && relative
                        .components()
                        .all(|c| matches!(c, Component::Normal(_))),
                "unsafe source path {path:?}"
            );
            let full = root
                .join(relative)
                .canonicalize()
                .with_context(|| format!("resolving {path}"))?;
            ensure!(full.starts_with(root), "source escapes live root: {path}");
            let source =
                std::fs::read_to_string(&full).with_context(|| format!("reading {path}"))?;
            let lines = source.lines().count();
            if lines > limits.file_size.max {
                report.add(
                    "file-size",
                    path,
                    &limits.file_size.reason,
                    format!("{lines} lines exceed {}", limits.file_size.max),
                    "Extract a cohesive responsibility into the intended layer; rerun check.",
                    true,
                );
            }
            if limits.single_implementation.enabled
                && matches!(
                    relative.extension().and_then(|v| v.to_str()),
                    Some("ts" | "tsx")
                )
            {
                syntax.insert(
                    path.to_owned(),
                    crate::collect::native::typescript::architecture(path, &source)?,
                );
            }
        }
    }
    if root.is_none() {
        report
            .limitations
            .push("File size unavailable for a saved graph: no live source root supplied.".into());
        if limits.single_implementation.enabled {
            report.limitations.push(
                "Single-implementation evidence unavailable without live TypeScript sources."
                    .into(),
            );
        }
    } else if limits.single_implementation.enabled {
        single_implementations(doc, &syntax, limits, &mut report);
    }
    report
        .violations
        .sort_by(|a, b| (&a.rule, &a.file, &a.evidence).cmp(&(&b.rule, &b.file, &b.evidence)));
    if report.blocking_count > 0 {
        report.verdict = "BLOCK".into();
    } else if root.is_none() {
        report.verdict = "INCOMPLETE".into();
    }
    Ok(report)
}

fn single_implementations(
    doc: &Document,
    syntax: &BTreeMap<String, crate::collect::native::typescript::ArchitectureFacts>,
    limits: &Thresholds,
    report: &mut Report,
) {
    report.limitations.push("Single-implementation candidates count only resolved explicit TypeScript class implements clauses in scanned production files; structural typing, inheritance, declaration merging and unscanned implementations remain unknown. Candidates are advisory.".into());
    let Some(symbols) = &doc.symbols else {
        report
            .limitations
            .push("Single-implementation evidence unavailable: scan with --symbols.".into());
        return;
    };
    if syntax.is_empty() {
        report.limitations.push(
            "Single-implementation evidence unavailable: no scanned TypeScript sources.".into(),
        );
        return;
    }
    let interfaces: BTreeSet<_> = symbols
        .exports
        .iter()
        .filter_map(|export| {
            syntax
                .get(&export.target.file)
                .filter(|facts| facts.interfaces.contains(&export.target.name))
                .map(|_| export.target.clone())
        })
        .collect();
    let mut implementations: BTreeMap<SymbolTarget, BTreeSet<(String, String)>> = BTreeMap::new();
    for (file, facts) in syntax {
        if facts.incomplete {
            report.limitations.push(format!(
                "{file}: unsupported inheritance or syntax makes implementation counts incomplete."
            ));
        }
        for (class, name) in &facts.implementations {
            let local = SymbolTarget {
                file: file.clone(),
                name: name.clone(),
            };
            let target = if interfaces.contains(&local) {
                Some(local)
            } else {
                let targets: BTreeSet<_> = facts
                    .imports
                    .iter()
                    .filter(|(alias, _, _)| alias == name)
                    .flat_map(|(_, original, specifier)| {
                        let imported_files: BTreeSet<_> = doc
                            .imports
                            .iter()
                            .flatten()
                            .filter(|r| r.from == *file && r.specifier == *specifier)
                            .flat_map(|r| r.targets.iter().map(String::as_str))
                            .collect();
                        symbols
                            .imports
                            .iter()
                            .filter(move |i| {
                                i.from == *file
                                    && i.name == *original
                                    && (imported_files.contains(i.to.as_str())
                                        || i.target.as_ref().is_some_and(|t| {
                                            imported_files.contains(t.file.as_str())
                                        }))
                            })
                            .filter_map(|i| i.target.clone())
                    })
                    .collect();
                (targets.len() == 1).then(|| targets.into_iter().next().unwrap())
            };
            if let Some(target) = target.filter(|t| interfaces.contains(t)) {
                implementations
                    .entry(target)
                    .or_default()
                    .insert((file.clone(), class.clone()));
            } else {
                report.limitations.push(format!("{file}: cannot resolve explicit implements {name}; its target count is unavailable."));
            }
        }
    }
    for (target, classes) in implementations {
        if classes.len() == 1 {
            let (file, class) = classes.first().unwrap();
            report.add("single-implementation", &target.file, &limits.single_implementation.reason, format!("Exported interface {} has one observed explicit implementation: {file}::{class}", target.name), "Review whether this public interface expresses a stable boundary; keep justified ports, otherwise simplify the abstraction. This is an advisory candidate, not a safe automatic edit.", false);
        }
    }
    report.limitations.sort();
    report.limitations.dedup();
}
