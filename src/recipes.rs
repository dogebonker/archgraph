//! Executable graph edits and explicit, measured verification contracts.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::{Document, Edge, Evidence, Finding, FindingKind, Link, Move, Verdict};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub steps: Vec<Step>,
    pub expected: Expected,
    pub verify: Verification,
    pub confidence: Confidence,
    pub risk_notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub from: String,
    pub to: String,
    pub detail: Value,
    pub mechanical: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    CutImport,
    MoveFile,
    AddReexport,
    ExtractInterface,
    SplitModule,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub pairs_removed: Option<i64>,
    pub cycle_size_after: Option<usize>,
    pub new_cycles: Option<usize>,
    pub instability_delta: BTreeMap<String, Option<f64>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub predicate: String,
    pub structured: Predicate,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Predicate {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings_absent: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub absent_edges: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_present: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files_absent: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cycles_in: Vec<String>,
    /// Modules/files used to measure predicted deltas, without asserting their cycles vanish.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equals: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forbidden: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Expected>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}
impl Confidence {
    pub fn weight(self) -> f64 {
        match self {
            Self::High => 1.0,
            Self::Medium => 0.6,
            Self::Low => 0.3,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub finding: Finding,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct VerificationResult {
    pub pass: bool,
    pub checks: Vec<VerificationCheck>,
}
#[derive(Debug, Clone, Serialize)]
pub struct VerificationCheck {
    pub predicate: String,
    pub pass: bool,
    pub expected: Value,
    pub actual: Value,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Rank {
    #[default]
    Recipe,
    Legacy,
}

/// Same identity as the MCP v1 tools; measurements and ranking never affect it.
pub fn stable_id(finding: &Finding) -> String {
    let sorted = |values: &[String]| {
        values
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };
    let identity = match &finding.evidence {
        Evidence::Cycle(c) => json!({"level":c.level,"members":sorted(&c.members)}),
        Evidence::ReexportCycle { members, .. } => json!({"members":sorted(members)}),
        Evidence::MisplacedFile { target_module, .. } => json!({"target":target_module}),
        Evidence::HiddenCoupling { files, .. } => json!({"files":sorted(files)}),
        Evidence::UnstableDependency { from, to, .. } => json!({"from":from,"to":to}),
        Evidence::ForbiddenEdge { edge } => {
            json!({"from":edge.from,"to":edge.to,"kind":edge.kind,"specifier":edge.specifier})
        }
        Evidence::DeepImport {
            from,
            to,
            entrypoint,
            symbols,
        } => json!({"from":from,"to":to,"entrypoint":entrypoint,"symbols":sorted(symbols)}),
        _ => Value::Null,
    };
    let bytes = serde_json::to_vec(&json!([finding.kind.as_str(), finding.subject, identity]))
        .expect("JSON values serialize");
    format!(
        "ag-{}",
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

pub fn recipe_for<'a>(doc: &'a Document, id: &str) -> Result<&'a Recipe> {
    if let Some(diagnostic) = doc.diagnostics.iter().find(|d| d.finding.finding_id == id) {
        bail!("observation has no repair recipe: {}", diagnostic.reason);
    }
    doc.findings
        .iter()
        .find(|f| f.finding_id == id)
        .with_context(|| {
            format!("unknown finding_id {id}: run report or verify and use a current finding_id")
        })?
        .recipe
        .as_ref()
        .context("finding has no recipe: rescan with the current archgraph version")
}

pub fn rank(doc: &mut Document, rank: Rank) {
    if let Some(mut hotspots) = doc.hotspots.take() {
        crate::hotspots::attach_findings(&mut hotspots, doc);
        let priorities = crate::hotspots::finding_priorities(&hotspots, doc.findings.len());
        for (finding, priority) in doc.findings.iter_mut().zip(priorities) {
            finding.hotspot_priority = Some(priority);
        }
        doc.hotspots = Some(hotspots);
    }
    let score = |f: &Finding| {
        f.hotspot_priority.unwrap_or(1.0)
            * f.recipe.as_ref().map_or(0.0, |r| {
                r.confidence.weight() * r.expected.pairs_removed.unwrap_or(0).max(0) as f64
            })
    };
    doc.findings.sort_by(|a, b| {
        let priority = match rank {
            Rank::Recipe => score(b).total_cmp(&score(a)),
            Rank::Legacy => b
                .hotspot_priority
                .unwrap_or(0.0)
                .total_cmp(&a.hotspot_priority.unwrap_or(0.0)),
        };
        priority.then_with(|| {
            (a.kind, &a.subject, &a.finding_id).cmp(&(b.kind, &b.subject, &b.finding_id))
        })
    });
    if let Some(mut hotspots) = doc.hotspots.take() {
        crate::hotspots::attach_findings(&mut hotspots, doc);
        doc.hotspots = Some(hotspots);
    }
}

/// Reattachment replaces generated boundary findings and preserves diagnostic evidence.
pub fn attach(doc: &mut Document) -> Result<()> {
    let mut findings = std::mem::take(&mut doc.findings);
    findings.extend(
        std::mem::take(&mut doc.diagnostics)
            .into_iter()
            .map(|d| d.finding),
    );
    findings.retain(|f| !matches!(f.kind, FindingKind::ForbiddenEdge | FindingKind::DeepImport));
    for edge in doc
        .edges
        .iter()
        .chain(&doc.external_edges)
        .filter(|e| e.verdict == Some(Verdict::Forbidden))
    {
        findings.push(finding(
            FindingKind::ForbiddenEdge,
            format!("{} → {}", edge.from, edge.to),
            Evidence::ForbiddenEdge { edge: edge.clone() },
        ));
    }
    if let Some(surface) = &doc.surface {
        for module in &surface.modules {
            if let Some(entrypoint) = module.entrypoints.first() {
                for deep in &module.deep_imports {
                    findings.push(finding(
                        FindingKind::DeepImport,
                        format!("{} → {}", deep.from, deep.to),
                        Evidence::DeepImport {
                            from: deep.from.clone(),
                            to: deep.to.clone(),
                            entrypoint: entrypoint.clone(),
                            symbols: deep.symbols.clone(),
                        },
                    ));
                }
            }
        }
    }
    findings.sort_by(|a, b| (a.kind, &a.subject).cmp(&(b.kind, &b.subject)));
    findings.dedup_by(|a, b| stable_id(a) == stable_id(b));
    let cycle_pairs = crate::graph::cycle_import_pairs(doc);
    let rules = crate::rules::compile(&doc.config)?;
    let mut generated = Vec::new();
    let mut diagnostics = Vec::new();
    for mut f in findings {
        f.finding_id = stable_id(&f);
        match generate(doc, &f, &cycle_pairs, rules.as_ref())? {
            Some(mut recipe) => {
                if let Evidence::ForbiddenEdge { edge } = &f.evidence
                    && let Some(rules) = &doc.config.rules
                {
                    for reason in rules.doc.reasons(edge.rule.as_deref().unwrap_or_default()) {
                        recipe.risk_notes.push(format!("Policy reason: {reason}"));
                    }
                    for source in rules.doc.sources(edge.rule.as_deref().unwrap_or_default()) {
                        recipe.risk_notes.push(format!("Policy source: {source}"));
                    }
                }
                forecast(doc, &mut recipe)?;
                f.recipe = Some(recipe);
                generated.push(f);
            }
            None => {
                f.recipe = None;
                let reason = match &f.evidence {
                    Evidence::Cycle(c) if c.level == crate::model::CycleLevel::Module => {
                        "Folder dependencies alone do not establish a policy violation or an import to remove. Inspect concrete cyclic file/package imports and declared architecture before changing wiring."
                    }
                    Evidence::DeepImport { .. } => {
                        "An inferred entrypoint is an observation, not a declared public boundary. Declare the intended public entries before requesting a facade repair."
                    }
                    _ => {
                        "Evidence does not determine a safe, concrete graph edit; inspect its evidence before changing architecture."
                    }
                };
                diagnostics.push(Diagnostic {
                    finding: f,
                    reason: reason.into(),
                });
            }
        }
    }
    doc.findings = generated;
    doc.diagnostics = diagnostics;
    rank(doc, Rank::Recipe);
    Ok(())
}
fn finding(kind: FindingKind, subject: String, evidence: Evidence) -> Finding {
    Finding {
        finding_id: String::new(),
        recipe: None,
        kind,
        subject,
        evidence,
        suggestion: None,
        simulation: None,
        age_days: None,
        introduced_by: None,
        provenance: None,
        hotspot_priority: None,
    }
}
fn recipe(
    steps: Vec<Step>,
    confidence: Confidence,
    structured: Predicate,
    risk_notes: Vec<String>,
) -> Recipe {
    Recipe {
        steps,
        expected: Expected::default(),
        verify: Verification {
            predicate: "recipe_deltas_match && all(structured)".into(),
            structured,
        },
        confidence,
        risk_notes,
    }
}
fn move_step(m: &Move) -> Step {
    Step {
        kind: StepKind::MoveFile,
        file: m.file.clone(),
        line: None,
        from: m.file.clone(),
        to: m.to.clone(),
        detail: json!({"operation":"move_file"}),
        mechanical: true,
    }
}
fn external_matches(doc: &Document, id: &str, package: &str) -> bool {
    doc.external_nodes
        .iter()
        .any(|n| n.id == id && n.package == package)
}
fn cut_step(doc: &Document, e: &Edge) -> Step {
    let records: Vec<_> = doc
        .imports
        .iter()
        .flatten()
        .filter(|i| {
            i.from == e.from
                && (i.targets.contains(&e.to)
                    || i.external
                        .as_ref()
                        .is_some_and(|p| external_matches(doc, &e.to, p)))
        })
        .collect();
    let record = (records.len() == 1).then(|| records[0]);
    let unique = record.is_some_and(|i| {
        i.targets.len() <= 1
            && i.line.is_some()
            && !i.specifier.is_empty()
            && e.line.is_none_or(|l| i.line == Some(l))
            && e.specifier.as_ref().is_none_or(|s| &i.specifier == s)
    });
    Step {
        kind: StepKind::CutImport,
        file: e.from.clone(),
        line: e.line.or_else(|| record.and_then(|i| i.line)),
        from: e.from.clone(),
        to: e.to.clone(),
        detail: json!({"operation":"remove_import","target":e.to,"specifier":e.specifier.as_ref().or_else(||record.map(|i|&i.specifier)),"edge_kind":e.kind,"original_to":e.original_to}),
        mechanical: unique
            && matches!(
                doc.source.language,
                crate::model::Language::Typescript
                    | crate::model::Language::Python
                    | crate::model::Language::Go
            ),
    }
}
fn cycle_cuttable(doc: &Document, edge: &Edge) -> bool {
    !edge.is_reexport
        && !matches!(
            edge.kind,
            crate::model::EdgeKind::Reexport | crate::model::EdgeKind::TypeReexport
        )
        && [&edge.from, &edge.to].into_iter().all(|path| {
            !crate::modules::is_test(path, &doc.config)
                && !doc
                    .files
                    .iter()
                    .any(|f| f.path == *path && f.class == Some(crate::model::ModuleClass::Tests))
        })
}

fn cut_recipe(doc: &Document, edges: Vec<&Edge>, scope: Vec<String>) -> Option<Recipe> {
    if edges.is_empty() {
        return None;
    }
    let mut steps: Vec<_> = edges.iter().map(|e| cut_step(doc, e)).collect();
    steps.sort_by(|a, b| (&a.file, a.line, &a.to).cmp(&(&b.file, b.line, &b.to)));
    steps.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.to == b.to);
    let absent_edges = edges
        .into_iter()
        .map(|e| Link {
            from: e.from.clone(),
            to: e.to.clone(),
        })
        .collect();
    Some(recipe(steps,Confidence::Medium,Predicate{absent_edges,scope,..Predicate::default()},vec!["Removing an import can leave references undefined or remove initialization side effects; run language checks and tests after editing.".into()]))
}

fn generate(
    doc: &Document,
    f: &Finding,
    cycle_pairs: &BTreeSet<(String, String)>,
    rules: Option<&crate::rules::Compiled>,
) -> Result<Option<Recipe>> {
    if let Evidence::Cycle(cycle) = &f.evidence {
        let in_scope = |path: &str| match cycle.level {
            crate::model::CycleLevel::File => cycle.members.iter().any(|member| member == path),
            crate::model::CycleLevel::Module => doc
                .files
                .iter()
                .any(|file| file.path == path && cycle.members.contains(&file.module)),
        };
        if !cycle_pairs
            .iter()
            .any(|(from, to)| in_scope(from) && in_scope(to))
        {
            return Ok(None);
        }
    }
    if let Evidence::DeepImport { to, .. } = &f.evidence
        && rules.and_then(|rules| rules.public_for(to)).is_none()
    {
        return Ok(None);
    }
    if let Some(s) = &f.suggestion {
        return Ok(Some(recipe(vec![move_step(&s.move_to)],Confidence::High,Predicate{files_present:vec![s.move_to.to.clone()],files_absent:vec![s.move_to.file.clone()],..Predicate::default()},vec!["Import-level simulation does not prove build configuration or runtime behavior; run language checks and tests.".into()])));
    }
    let edges_between = |from: &str, to: &str| {
        doc.edges
            .iter()
            .filter(|e| {
                let module = |path: &str| {
                    doc.files
                        .iter()
                        .find(|f| f.path == path)
                        .map(|f| f.module.as_str())
                };
                module(&e.from) == Some(from)
                    && module(&e.to) == Some(to)
                    && (doc.config.all_kinds || e.kind.is_runtime())
                    && cycle_cuttable(doc, e)
                    && (!matches!(f.evidence, Evidence::Cycle(_))
                        || cycle_pairs.contains(&(e.from.clone(), e.to.clone())))
            })
            .collect::<Vec<_>>()
    };
    Ok(match &f.evidence {
        Evidence::Cycle(c) => {
            let cut = c.two_way.as_ref().and_then(|pairs| {
                pairs
                    .iter()
                    .filter(|p| !p.registry && !p.entry && !p.resolved_by_move)
                    .filter(|p| !edges_between(&p.from, &p.to).is_empty())
                    .min_by_key(|p| (p.imports, &p.from, &p.to))
            });
            if let Some(p) = cut {
                let mut proposal =
                    cut_recipe(doc, edges_between(&p.from, &p.to), c.members.clone());
                if p.even
                    && let Some(r) = &mut proposal
                {
                    r.confidence = Confidence::Low;
                    r.risk_notes.push("Both dependency directions are balanced; neither side is architecturally preferred. Confirm this cut preserves behavior before applying.".into());
                }
                proposal
            } else {
                let selected = c
                    .break_set
                    .iter()
                    .flat_map(|link| {
                        if c.level == crate::model::CycleLevel::File {
                            doc.edges
                                .iter()
                                .filter(|e| e.from == link.from && e.to == link.to)
                                .collect()
                        } else {
                            edges_between(&link.from, &link.to)
                        }
                    })
                    .filter(|e| {
                        cycle_cuttable(doc, e)
                            && cycle_pairs.contains(&(e.from.clone(), e.to.clone()))
                            && !doc.files.iter().any(|f| f.path == e.from && f.barrel)
                    })
                    .collect();
                cut_recipe(doc, selected, c.members.clone())
            }
        }
        Evidence::ForbiddenEdge { edge } => {
            if edge.names.is_empty() {
                cut_recipe(doc, vec![edge], Vec::new())
            } else {
                interface_recipe(doc, edge, f)?
            }
        }
        Evidence::DeepImport {
            from,
            to,
            entrypoint,
            symbols,
        } => {
            let edges: Vec<_> = doc
                .edges
                .iter()
                .filter(|e| e.from == *from && (e.to == *to || e.original_to.as_ref() == Some(to)))
                .collect();
            if edges.len() != 1 || symbols.is_empty() {
                None
            } else {
                let e = edges[0];
                let mut rewrite = cut_step(doc, e);
                rewrite.to = entrypoint.clone();
                rewrite.detail = json!({"operation":"rewrite_specifier","target":e.to,"entrypoint":entrypoint,"specifier":e.specifier,"old_target":e.original_to.as_ref().unwrap_or(&e.to)});
                let supported = matches!(
                    doc.source.language,
                    crate::model::Language::Typescript | crate::model::Language::Python
                );
                rewrite.mechanical &= supported && !symbols.iter().any(|s| s == "*");
                let reexport = Step {
                    kind: StepKind::AddReexport,
                    file: entrypoint.clone(),
                    line: None,
                    from: entrypoint.clone(),
                    to: e.to.clone(),
                    detail: json!({"operation":"reexport","target":e.to,"symbols":symbols,"entrypoint":entrypoint}),
                    mechanical: rewrite.mechanical,
                };
                Some(recipe(vec![reexport,rewrite],Confidence::Medium,Predicate{findings_absent:vec![f.finding_id.clone()],..Predicate::default()},vec!["Re-exporting may expose names or introduce an initialization dependency; confirm the intended public API.".into()]))
            }
        }
        Evidence::ReexportCycle { edges, .. } => {
            let mut proposal = cut_recipe(
                doc,
                edges
                    .iter()
                    .filter_map(|link| {
                        doc.edges
                            .iter()
                            .find(|e| e.from == link.from && e.to == link.to)
                    })
                    .collect(),
                Vec::new(),
            );
            if let Some(r) = &mut proposal {
                r.confidence = Confidence::Low;
                for step in &mut r.steps {
                    step.mechanical = false;
                }
                r.risk_notes.push("Removing a re-export changes the public API; choose and migrate its consumers before cutting this chain.".into());
            }
            proposal
        }
        Evidence::UnstableDependency { from, to, .. } => {
            let candidates = edges_between(from, to);
            candidates.first().map(|e| {
                let extension=e.from.rsplit('.').next().unwrap_or("ts");
                let parent=e.from.rsplit_once('/').map_or("",|(p,_)|p);
                let location=if parent.is_empty(){format!("ports/dependency.{extension}")}else{format!("{parent}/ports/dependency.{extension}")};
                recipe(vec![Step{kind:StepKind::ExtractInterface,file:e.from.clone(),line:e.line,from:from.clone(),to:location.clone(),detail:json!({"operation":"extract_contract","interface_location":location,"dependency":e.to}),mechanical:false}],Confidence::Medium,Predicate{findings_absent:vec![f.finding_id.clone()],..Predicate::default()},vec!["Define a contract in the consumer layer and inject the implementation; symbol use and runtime wiring require agent work.".into()])
            })
        }
        Evidence::Grabbag(g) => {
            let files: Vec<_> = g.candidates.iter().flat_map(|c| c.files.clone()).collect();
            split_recipe(doc, f, &files, None)
        }
        Evidence::SinkSplit { components, .. } => {
            let groups: Vec<Vec<String>> = components.iter().map(|c| c.files.clone()).collect();
            let files = groups.iter().flatten().cloned().collect::<Vec<_>>();
            split_recipe(doc, f, &files, Some(groups))
        }
        Evidence::Seam { clusters, .. } => {
            let groups: Vec<Vec<String>> = clusters.iter().map(|c| c.files.clone()).collect();
            let files = groups.iter().flatten().cloned().collect::<Vec<_>>();
            split_recipe(doc, f, &files, Some(groups))
        }
        Evidence::InternalsReach { edges, entries, .. } => {
            // Without symbol evidence a facade rewrite cannot preserve the imported bindings.
            let _ = (edges, entries);
            None
        }
        Evidence::MisplacedFile { .. }
        | Evidence::HiddenCoupling { .. }
        | Evidence::SmallRepo { .. } => None,
    })
}

fn split_recipe(
    doc: &Document,
    f: &Finding,
    files: &[String],
    structural: Option<Vec<Vec<String>>>,
) -> Option<Recipe> {
    if files.len() < 2 {
        return None;
    }
    let selected: BTreeSet<_> = files.iter().cloned().collect();
    let mut groups: BTreeMap<String, BTreeSet<String>> = selected
        .iter()
        .map(|p| (p.clone(), BTreeSet::from([p.clone()])))
        .collect();
    let cochange: Vec<_> = doc
        .cochange
        .iter()
        .flat_map(|c| &c.pairs)
        .filter(|p| selected.contains(&p.files[0]) && selected.contains(&p.files[1]))
        .collect();
    let evidence_source = if cochange.is_empty() {
        "structural"
    } else {
        "cochange"
    };
    for pair in cochange {
        let keys: Vec<_> = groups
            .iter()
            .filter(|(_, g)| g.contains(&pair.files[0]) || g.contains(&pair.files[1]))
            .map(|(k, _)| k.clone())
            .collect();
        let mut merged = BTreeSet::new();
        for key in keys {
            merged.extend(groups.remove(&key).unwrap_or_default());
        }
        if let Some(first) = merged.first().cloned() {
            groups.insert(first, merged);
        }
    }
    let groups: Vec<Vec<String>> = if evidence_source == "structural" {
        structural.unwrap_or_else(|| selected.into_iter().map(|p| vec![p]).collect())
    } else {
        groups
            .into_values()
            .map(|g| g.into_iter().collect())
            .collect()
    };
    if groups.len() < 2 {
        return None;
    }
    let file = files.first()?.clone();
    Some(recipe(
        vec![Step {
            kind: StepKind::SplitModule,
            file,
            line: None,
            from: f.subject.clone(),
            to: String::new(),
            detail: json!({"operation":"group_files","groups":groups.iter().enumerate().map(|(i,g)|json!({"files":g,"suggested_module":format!("split-{}",i+1)})).collect::<Vec<_>>(),"evidence_source":evidence_source}),
            mechanical: false,
        }],
        Confidence::Low,
        Predicate {
            findings_absent: vec![f.finding_id.clone()],
            ..Predicate::default()
        },
        vec![format!(
            "Groups use {evidence_source} evidence. Choose meaningful folders and update their consumers; no numerical delta is promised before destinations are chosen."
        )],
    ))
}

/// Apply only fully specified graph edits, without findings/recipe generation.
pub fn project(doc: &Document, recipe: &Recipe) -> Result<Document> {
    ensure!(
        recipe.steps.iter().all(|s| s.mechanical),
        "recipe contains agent steps: apply its instructions and rescan before verification"
    );
    let rules = crate::rules::compile(&doc.config)?;
    let moves: Vec<_> = recipe
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::MoveFile)
        .map(|s| Move {
            file: s.from.clone(),
            to: s.to.clone(),
        })
        .collect();
    let moved = if moves.is_empty() {
        doc.clone()
    } else {
        crate::simulate::apply(doc, rules.as_ref(), &moves)?
    };
    if recipe.steps.iter().all(|s| s.kind == StepKind::MoveFile) {
        return Ok(moved);
    }
    let mut edges = moved.edges.clone();
    let mut config = moved.config.clone();
    config.import_records = moved.imports.clone();
    for step in &recipe.steps {
        match step.kind {
            StepKind::MoveFile => {}
            StepKind::CutImport if step.detail["operation"] == "remove_import" => {
                edges.retain(|e| {
                    !(e.from == step.file
                        && e.to == step.to
                        && step.line.is_none_or(|l| e.line == Some(l)))
                });
                if let Some(records) = &mut config.import_records {
                    records.retain(|i| {
                        !(i.from == step.file
                            && step.line.is_none_or(|l| i.line == Some(l))
                            && (i.targets.contains(&step.to)
                                || i.external
                                    .as_ref()
                                    .is_some_and(|p| external_matches(&moved, &step.to, p))))
                    });
                }
            }
            StepKind::CutImport if step.detail["operation"] == "rewrite_specifier" => {
                let target = step.detail["target"]
                    .as_str()
                    .context("rewrite recipe requires detail.target")?;
                for edge in edges.iter_mut().filter(|e| {
                    e.from == step.file
                        && e.to == target
                        && step.line.is_none_or(|l| e.line == Some(l))
                }) {
                    edge.original_to = Some(step.to.clone());
                }
                // Named imports retain their defining target after facade resolution.
            }
            StepKind::AddReexport => {
                let target = step.detail["target"]
                    .as_str()
                    .context("re-export recipe requires detail.target")?;
                ensure!(
                    moved.files.iter().any(|f| f.path == step.file),
                    "entrypoint {} is not in the scan: create it before requesting a mechanical recipe",
                    step.file
                );
                if !edges
                    .iter()
                    .any(|e| e.from == step.file && e.to == target && e.is_reexport)
                {
                    edges.push(Edge {
                        from: step.file.clone(),
                        to: target.into(),
                        kind: if moved.files.iter().any(|f| f.path == step.file && f.barrel) {
                            crate::model::EdgeKind::Reexport
                        } else {
                            crate::model::EdgeKind::Static
                        },
                        weight: None,
                        is_reexport: true,
                        confidence: None,
                        original_to: None,
                        specifier: None,
                        line: None,
                        names: step.detail["symbols"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|s| s.as_str().map(String::from))
                            .collect(),
                        resolution_reason: None,
                        verdict: None,
                        rule: None,
                        known: false,
                        reach: None,
                    });
                }
            }
            _ => bail!("unsupported mechanical recipe operation: {}", step.detail),
        }
    }
    let owners = config
        .codeowners
        .as_ref()
        .map(crate::codeowners::compile)
        .transpose()?;
    let mut after = crate::build(
        moved.source.clone(),
        config,
        moved.files.iter().map(|f| f.path.clone()).collect(),
        edges,
        moved.package_entries.clone(),
        rules.as_ref(),
        owners.as_ref(),
    );
    crate::resolution::attach(&mut after);
    after.symbols = moved.symbols.clone();
    Ok(after)
}
fn pairs(doc: &Document) -> usize {
    doc.cycles
        .iter()
        .filter_map(|c| c.two_way.as_ref())
        .map(Vec::len)
        .sum()
}
fn cycle_size(doc: &Document, scope: &[String]) -> usize {
    doc.cycles
        .iter()
        .filter(|c| scope.is_empty() || c.members.iter().any(|m| scope.contains(m)))
        .map(|c| c.members.len())
        .max()
        .unwrap_or(0)
}
/// Populate deltas only after every source edit is proven mechanical by the caller.
pub fn forecast(doc: &Document, recipe: &mut Recipe) -> Result<()> {
    if recipe.steps.iter().all(|s| s.mechanical) {
        let after = project(doc, recipe)?;
        recipe.expected = measure(doc, &after, recipe);
        recipe.verify.structured.expected = Some(recipe.expected.clone());
    } else {
        recipe.expected = Expected::default();
        recipe.verify.structured.expected = None;
    }
    Ok(())
}

fn interface_recipe(doc: &Document, edge: &Edge, finding: &Finding) -> Result<Option<Recipe>> {
    let parent = edge.from.rsplit_once('/').map_or("", |(p, _)| p);
    let ext = edge.from.rsplit('.').next().unwrap_or("ts");
    let filename = match doc.source.language {
        crate::model::Language::Go => "dependency.go".into(),
        crate::model::Language::Python => "dependency.py".into(),
        crate::model::Language::Java => "Dependency.java".into(),
        _ => format!("dependency.{ext}"),
    };
    let mut location = if parent.is_empty() {
        format!("ports/{filename}")
    } else {
        format!("{parent}/ports/{filename}")
    };
    for i in 1.. {
        if !doc.files.iter().any(|f| f.path == location) {
            break;
        }
        location = if parent.is_empty() {
            format!("ports/dependency-{i}.{ext}")
        } else {
            format!("{parent}/ports/dependency-{i}.{ext}")
        };
    }
    let rules = crate::rules::compile(&doc.config)?;
    let allowed = rules
        .as_ref()
        .is_none_or(|r| r.layer_for(&edge.from) == r.layer_for(&location));
    if !allowed {
        let mut cut = cut_recipe(doc, vec![edge], Vec::new());
        if let Some(r) = &mut cut {
            for step in &mut r.steps {
                step.mechanical = false;
            }
            r.risk_notes.push("No consumer-owned interface location in the same allowed layer could be established; choose one before editing.".into());
        }
        return Ok(cut);
    }
    Ok(Some(recipe(vec![Step {
        kind:StepKind::ExtractInterface,file:edge.from.clone(),line:edge.line,from:edge.from.clone(),to:location.clone(),
        detail:json!({"operation":"extract_contract","interface_location":location,"dependency":edge.to,"symbols":edge.names,"consumer_layer":rules.as_ref().and_then(|r|r.layer_for(&edge.from)).map(|(_,name)|name)}),mechanical:false,
    }],Confidence::Medium,Predicate{findings_absent:vec![finding.finding_id.clone()],absent_edges:vec![Link{from:edge.from.clone(),to:edge.to.clone()}],..Predicate::default()},vec!["Define the contract in the consumer's allowed layer, replace the forbidden dependency with that contract, and inject its implementation from composition code. Interface contents require agent work.".into()])))
}

fn measure(before: &Document, after: &Document, recipe: &Recipe) -> Expected {
    let predicate = &recipe.verify.structured;
    let scope = if predicate.scope.is_empty() {
        &predicate.cycles_in
    } else {
        &predicate.scope
    };
    let scope: Vec<String> = scope
        .iter()
        .map(|path| {
            recipe
                .steps
                .iter()
                .find(|s| s.kind == StepKind::MoveFile && s.from == *path)
                .map_or_else(|| path.clone(), |s| s.to.clone())
        })
        .collect();
    let renames: BTreeMap<_, _> = recipe
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::MoveFile)
        .map(|s| (&s.to, &s.from))
        .collect();
    let new_cycles = after
        .cycles
        .iter()
        .filter(|cycle| {
            !before.cycles.iter().any(|old| {
                let mut members: Vec<_> = cycle
                    .members
                    .iter()
                    .map(|path| {
                        if cycle.level == crate::model::CycleLevel::File {
                            renames.get(path).copied().unwrap_or(path)
                        } else {
                            path
                        }
                    })
                    .collect();
                members.sort();
                old.level == cycle.level
                    && old.members.iter().collect::<BTreeSet<_>>() == members.into_iter().collect()
            })
        })
        .count();
    let instability_delta = before
        .summary
        .modules
        .keys()
        .chain(after.summary.modules.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| {
            let old = before.summary.modules.get(id).and_then(|m| m.instability);
            let new = after.summary.modules.get(id).and_then(|m| m.instability);
            (old != new).then(|| {
                (
                    id.clone(),
                    old.zip(new).map(|(a, b)| crate::quality::round4(b - a)),
                )
            })
        })
        .collect();
    Expected {
        pairs_removed: Some(pairs(before) as i64 - pairs(after) as i64),
        cycle_size_after: Some(cycle_size(after, &scope)),
        new_cycles: Some(new_cycles),
        instability_delta,
    }
}
pub fn check(before: &Document, after: &Document, recipe: &Recipe) -> VerificationResult {
    let p = &recipe.verify.structured;
    let mut checks = Vec::new();
    let mut push = |predicate: String, expected: Value, actual: Value| {
        checks.push(VerificationCheck {
            predicate,
            pass: expected == actual,
            expected,
            actual,
        });
    };
    for id in &p.findings_absent {
        push(
            format!("finding_absent:{id}"),
            json!(false),
            json!(after.findings.iter().any(|f| &f.finding_id == id)),
        );
    }
    for edge in &p.absent_edges {
        push(
            format!("edge_absent:{} -> {}", edge.from, edge.to),
            json!(false),
            json!(
                after
                    .edges
                    .iter()
                    .chain(&after.external_edges)
                    .any(|e| e.from == edge.from && e.to == edge.to)
            ),
        );
    }
    for file in &p.files_present {
        push(
            format!("file_present:{file}"),
            json!(true),
            json!(after.files.iter().any(|f| &f.path == file)),
        );
    }
    for file in &p.files_absent {
        push(
            format!("file_absent:{file}"),
            json!(false),
            json!(after.files.iter().any(|f| &f.path == file)),
        );
    }
    if let Some(equals) = p.equals {
        let count = after
            .cycles
            .iter()
            .filter(|c| c.members.iter().any(|m| p.cycles_in.contains(m)))
            .count();
        push("cycles_in".into(), json!(equals), json!(count));
    }
    if let Some(forbidden) = p.forbidden {
        push(
            "forbidden".into(),
            json!(forbidden),
            json!(after.summary.forbidden),
        );
    }
    let measured = measure(before, after, recipe);
    let expected = p.expected.as_ref().unwrap_or(&recipe.expected);
    for (name, was, is) in [
        (
            "pairs_removed",
            json!(expected.pairs_removed),
            json!(measured.pairs_removed),
        ),
        (
            "cycle_size_after",
            json!(expected.cycle_size_after),
            json!(measured.cycle_size_after),
        ),
        (
            "new_cycles",
            json!(expected.new_cycles),
            json!(measured.new_cycles),
        ),
    ] {
        if !was.is_null() {
            push(name.into(), was, is);
        }
    }
    for (module, delta) in &expected.instability_delta {
        if delta.is_none() {
            continue;
        }
        let actual = measured.instability_delta.get(module).copied().flatten();
        push(
            format!("instability_delta:{module}"),
            json!(delta),
            json!(actual),
        );
    }
    VerificationResult {
        pass: !checks.is_empty() && checks.iter().all(|c| c.pass),
        checks,
    }
}
