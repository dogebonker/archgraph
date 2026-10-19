//! Evidence-only MCP operations. Transport, freshness and output budgets live at the caller.
use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{Cycle, Document, Finding, Move};

pub struct ToolSchema {
    pub name: &'static str,
    pub description: &'static str,
    pub input: Value,
    pub output: Value,
}
fn yes() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextArgs {
    task: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    symbols: Vec<String>,
    budget_tokens: usize,
    #[serde(default = "yes")]
    brief: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffArgs {
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    snapshot: Option<String>,
    #[serde(default = "yes")]
    brief: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposeArgs {
    finding_id: String,
    #[serde(default = "yes")]
    brief: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveArgs {
    from: String,
    to: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulateArgs {
    moves: Vec<MoveArgs>,
    #[serde(default = "yes")]
    brief: bool,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    #[serde(default)]
    cycles_in: Option<Vec<String>>,
    #[serde(default)]
    equals: usize,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    expected: Option<crate::recipes::Expected>,
    #[serde(default)]
    baseline_id: Option<String>,
    #[serde(default)]
    forbidden: Option<usize>,
    #[serde(default)]
    findings_absent: Option<Vec<String>>,
    #[serde(default)]
    absent_edges: Option<Vec<MoveArgs>>,
    #[serde(default)]
    files_present: Option<Vec<String>>,
    #[serde(default)]
    files_absent: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyArgs {
    #[serde(default)]
    expect: Option<Expect>,
    #[serde(default = "yes")]
    brief: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplainArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    module: Option<String>,
    #[serde(default = "yes")]
    brief: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesArgs {
    paths: Vec<String>,
    #[serde(default = "yes")]
    brief: bool,
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value)
        .context("invalid tool arguments: use tools/list for the accepted fields and types")
}
/// Validate typed argument fields before a transport spends time collecting a graph.
pub fn validate(name: &str, arguments: &Value) -> Result<()> {
    match name {
        "context" => {
            let a: ContextArgs = parse(arguments.clone())?;
            if a.budget_tokens == 0 || a.budget_tokens > 1_000_000 {
                bail!("budget_tokens must be between 1 and 1000000");
            }
        }
        "check_diff" => {
            let a: DiffArgs = parse(arguments.clone())?;
            if let Some(snapshot) = &a.snapshot
                && (a.base.is_some()
                    || snapshot.len() != 64
                    || !snapshot
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            {
                bail!("snapshot must be a 64-character lowercase hex capture ID, without base");
            }
            if a.base.as_ref().is_some_and(|b| b.trim().is_empty()) {
                bail!("base must be a non-empty Git revision");
            }
        }
        "propose" => {
            let _: ProposeArgs = parse(arguments.clone())?;
        }
        "simulate" => {
            let a: SimulateArgs = parse(arguments.clone())?;
            if a.moves.is_empty() {
                bail!("moves must contain at least one {{from,to}} repository-relative path");
            }
        }
        "verify" => {
            let _: VerifyArgs = parse(arguments.clone())?;
        }
        "explain" => {
            let a: ExplainArgs = parse(arguments.clone())?;
            if !matches!((&a.path, &a.module), (Some(p), None) | (None, Some(p)) if !p.is_empty()) {
                bail!("provide exactly one of path or module");
            }
        }
        "rules" => {
            let a: RulesArgs = parse(arguments.clone())?;
            if a.paths.is_empty() {
                bail!("paths must contain at least one repository-relative path");
            }
        }
        _ => bail!("unknown tool {name:?}: use tools/list for available tools"),
    }
    Ok(())
}
pub fn is_brief(arguments: &Value) -> bool {
    arguments
        .get("brief")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// Identity excludes rankings, provenance and measurements; collections are canonicalized.
pub fn finding_id(finding: &Finding) -> String {
    crate::recipes::stable_id(finding)
}

fn sorted(values: &[String]) -> Vec<&str> {
    values
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn findings(doc: &Document) -> Vec<Value> {
    doc.findings.iter().map(|f| json!({"finding_id":finding_id(f),"kind":f.kind,"subject":f.subject,"evidence":f.evidence,"suggestion":f.suggestion,"recipe":f.recipe})).collect()
}

fn cycle_key(c: &Cycle) -> String {
    format!("{:?}:{}", c.level, sorted(&c.members).join("\0"))
}
fn pairs(doc: &Document) -> usize {
    doc.cycles
        .iter()
        .filter_map(|c| c.two_way.as_ref())
        .map(Vec::len)
        .sum()
}
fn new_cycles(before: &Document, after: &Document, moves: &[Move]) -> Vec<Value> {
    let old: BTreeSet<_> = before
        .cycles
        .iter()
        .map(|cycle| {
            if cycle.level == crate::model::CycleLevel::File {
                let members: Vec<_> = cycle
                    .members
                    .iter()
                    .map(|path| {
                        moves
                            .iter()
                            .find(|m| m.file == *path)
                            .map_or_else(|| path.clone(), |m| m.to.clone())
                    })
                    .collect();
                format!("{:?}:{}", cycle.level, sorted(&members).join("\0"))
            } else {
                cycle_key(cycle)
            }
        })
        .collect();
    let mut cycles: Vec<_> = after
        .cycles
        .iter()
        .filter(|c| !old.contains(&cycle_key(c)))
        .collect();
    cycles.sort_by_key(|c| cycle_key(c));
    cycles.into_iter().map(|c| json!(c)).collect()
}
fn relevant(doc: &Document, paths: &[String]) -> Result<Value> {
    match (&doc.config.rules, crate::rules::compile(&doc.config)?) {
        (Some(r), Some(c)) => c.relevant(&r.doc, paths),
        _ => Ok(
            json!({"configured":false,"paths":paths,"hint":"Create layout.yaml or pass --rules to define architectural boundaries."}),
        ),
    }
}

pub fn call(
    doc: &Document,
    root: &Path,
    name: &str,
    arguments: Value,
    baseline: Option<&Document>,
) -> Result<Value> {
    validate(name, &arguments)?;
    match name {
        "context" => {
            let a: ContextArgs = parse(arguments)?;
            let _ = a.brief;
            if a.budget_tokens == 0 || a.budget_tokens > 1_000_000 {
                bail!(
                    "budget_tokens must be between 1 and 1000000; narrow files or task for a smaller context"
                );
            }
            if a.task.trim().is_empty() && a.files.is_empty() && a.symbols.is_empty() {
                bail!("provide a task, files or symbols to seed context selection");
            }
            let input = serde_json::from_value(serde_json::to_value(doc)?)?;
            let mut report = crate::context::select(
                &input,
                root,
                &crate::context::Options {
                    task: a.task,
                    files: a.files,
                    symbols: a.symbols,
                    budget: a.budget_tokens,
                    ..Default::default()
                },
            )?;
            for entry in &mut report.entries {
                entry.content = None;
            }
            Ok(serde_json::to_value(report)?)
        }
        "check_diff" => {
            let a: DiffArgs = parse(arguments)?;
            let _ = a.brief;
            if a.snapshot.is_some() {
                bail!("pre-edit snapshots require the live MCP or mcp-call transport");
            }
            let old=baseline.context("check_diff requires a baseline scan; run inside a Git repository and provide base=<commit> if HEAD has no parent")?;
            let diff = crate::diff::diff(old, doc);
            let mut grown = Vec::new();
            for cycle in &doc.cycles {
                let members: BTreeSet<_> = cycle.members.iter().collect();
                for prior in &old.cycles {
                    if prior.level == cycle.level
                        && prior.members.len() < members.len()
                        && prior.members.iter().all(|m| members.contains(m))
                    {
                        grown.push(json!({"before":prior,"after":cycle}));
                    }
                }
            }
            grown.sort_by_key(Value::to_string);
            Ok(
                json!({"base":a.base,"new_forbidden":diff.new_forbidden,"new_cyclic_imports":diff.new_cyclic_imports,"new_cycles":new_cycles(old,doc,&[]),"grown_cycles":grown,"findings":findings(doc),"limitations":["Module cycles describe display-folder coupling; new_cyclic_imports supplies concrete runtime cycle evidence. Recipes use current findings; semantic code changes require review."]}),
            )
        }
        "propose" => {
            let a: ProposeArgs = parse(arguments)?;
            let _ = a.brief;
            let mut recipe = crate::recipes::recipe_for(doc, &a.finding_id)?.clone();
            recipe.verify.structured.baseline_id = Some(a.finding_id.clone());
            crate::apply::remember_proposal(root, doc, &a.finding_id, &recipe)?;
            let mut result = serde_json::to_value(recipe)?;
            result["finding_id"] = json!(a.finding_id);
            Ok(result)
        }
        "simulate" => {
            let a: SimulateArgs = parse(arguments)?;
            let _ = a.brief;
            if a.moves.is_empty() {
                bail!("moves must contain at least one {{from,to}} repository-relative path");
            }
            let moves: Vec<_> = a
                .moves
                .into_iter()
                .map(|m| Move {
                    file: m.from,
                    to: m.to,
                })
                .collect();
            let rules = crate::rules::compile(&doc.config)?;
            let after = crate::simulate::apply(doc, rules.as_ref(), &moves)?;
            Ok(simulation(doc, &after, &moves))
        }
        "verify" => {
            let a: VerifyArgs = parse(arguments)?;
            let _ = a.brief;
            let checks = verify(doc, root, a.expect.as_ref())?;
            let passed = checks.iter().all(|c| c["pass"] == true);
            Ok(
                json!({"pass":passed,"expectations_supplied":a.expect.is_some(),"checks":checks,"summary":{"cycles":doc.cycles.len(),"forbidden":doc.summary.forbidden},"findings":findings(doc),"diagnostics":doc.diagnostics}),
            )
        }
        "explain" => {
            let a: ExplainArgs = parse(arguments)?;
            let _ = a.brief;
            let subject = match (&a.path, &a.module) {
                (Some(p), None) | (None, Some(p)) if !p.is_empty() => p,
                _ => bail!(
                    "provide exactly one of path or module; use a repository-relative path or an exact module ID"
                ),
            };
            let file = doc.files.iter().find(|f| &f.path == subject);
            let module = doc
                .modules
                .iter()
                .find(|m| &m.id == subject || file.is_some_and(|f| f.module == m.id))
                .context(
                    "unknown path or module: call context or rules with an exact scanned path",
                )?;
            if a.path.is_some() && file.is_none() {
                bail!(
                    "path is not a scanned file: use module for a module ID, or scan with the relevant inclusion flags"
                );
            }
            let member = |p: &str| {
                file.map_or_else(
                    || {
                        doc.files
                            .iter()
                            .any(|f| f.path == p && f.module == module.id)
                    },
                    |f| f.path == p,
                )
            };
            let imports: Vec<_> = doc
                .edges
                .iter()
                .chain(&doc.external_edges)
                .filter(|e| member(&e.from))
                .map(|e| json!({"from":e.from,"to":e.to,"kind":e.kind}))
                .collect();
            let importers: Vec<_> = doc
                .edges
                .iter()
                .filter(|e| member(&e.to))
                .map(|e| json!({"from":e.from,"to":e.to,"kind":e.kind}))
                .collect();
            let paths: Vec<_> = doc
                .files
                .iter()
                .filter(|f| file.map_or(f.module == module.id, |file| file.path == f.path))
                .map(|f| f.path.clone())
                .collect();
            let compiled = crate::rules::compile(&doc.config)?;
            let classifications: BTreeSet<_> = paths
                .iter()
                .map(|path| compiled.as_ref().and_then(|c| c.layer_for(path)))
                .collect();
            let layers: Vec<_> = classifications.iter().filter_map(Clone::clone).collect();
            let layer = if classifications.len() == 1 {
                layers.first().cloned()
            } else {
                None
            };
            let layer_status = if layer.is_some() {
                "uniform"
            } else if layers.is_empty() {
                "unclassified"
            } else {
                "mixed"
            };
            let hotspots: Vec<_> = doc
                .hotspots
                .iter()
                .flat_map(|h| &h.files)
                .filter(|h| paths.contains(&h.path))
                .collect();
            let history:Vec<_>=doc.findings.iter().filter(|f|f.subject==*subject || f.subject==module.id).filter(|f|f.age_days.is_some()||f.introduced_by.is_some()).map(|f|json!({"finding_id":finding_id(f),"age_days":f.age_days,"introduced_by":f.introduced_by})).collect();
            Ok(
                json!({"subject":subject,"module":module.id,"grouping":crate::modules::grouping(&doc.config),"source_semantics":crate::modules::dependency_semantics(doc.source.language),"layer":layer,"layers":layers,"layer_status":layer_status,"imports":imports,"importers":importers,"rules":relevant(doc,&paths)?,"hotspots":hotspots,"history":history,"facts_available":{"hotspots":doc.hotspots.is_some(),"history":!history.is_empty()},"limitations":["At least these static dependencies; dynamic dispatch may add others."]}),
            )
        }
        "rules" => {
            let a: RulesArgs = parse(arguments)?;
            let _ = a.brief;
            if a.paths.is_empty() {
                bail!("paths must include at least one repository-relative path");
            }
            relevant(doc, &a.paths)
        }
        _ => bail!("unknown tool {name:?}: use tools/list for available tools"),
    }
}
fn simulation(before: &Document, after: &Document, moves: &[Move]) -> Value {
    json!({"before":{"cycles":before.cycles.len(),"forbidden":before.summary.forbidden,"two_way_pairs":pairs(before)},"after":{"cycles":after.cycles.len(),"forbidden":after.summary.forbidden,"two_way_pairs":pairs(after)},"pairs_removed":pairs(before) as i64-pairs(after) as i64,"new_cycles":new_cycles(before,after,moves),"limitations":["Import-level simulation does not rewrite imports, build configuration or runtime assumptions."]})
}

fn verify(doc: &Document, root: &Path, expect: Option<&Expect>) -> Result<Vec<Value>> {
    let mut checks = Vec::new();
    let Some(e) = expect else {
        return Ok(checks);
    };
    if e.expected.is_some() || e.baseline_id.is_some() {
        let id=e.baseline_id.as_deref().context("expected deltas need baseline_id: call propose before editing and pass recipe.verify.structured")?;
        let predicate = crate::recipes::Predicate {
            findings_absent: e.findings_absent.clone().unwrap_or_default(),
            absent_edges: e
                .absent_edges
                .iter()
                .flatten()
                .map(|edge| crate::model::Link {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                })
                .collect(),
            files_present: e.files_present.clone().unwrap_or_default(),
            files_absent: e.files_absent.clone().unwrap_or_default(),
            cycles_in: e.cycles_in.clone().unwrap_or_default(),
            scope: e.scope.clone(),
            equals: e.cycles_in.as_ref().map(|_| e.equals),
            forbidden: e.forbidden,
            baseline_id: Some(id.into()),
            expected: e.expected.clone(),
        };
        return Ok(crate::apply::verify_predicate(root,id,doc,&predicate)?.checks.iter().map(|check|json!({"predicate":check.predicate,"pass":check.pass,"expected":check.expected,"actual":check.actual})).collect());
    }
    if e.cycles_in.is_none()
        && e.forbidden.is_none()
        && e.findings_absent.is_none()
        && e.absent_edges.is_none()
        && e.files_present.is_none()
        && e.files_absent.is_none()
    {
        bail!(
            "expect must specify cycles_in, forbidden or findings_absent; omit expect to inspect current findings"
        );
    }
    if e.cycles_in.is_none() && e.equals != 0 {
        bail!("expect.equals requires cycles_in");
    }
    if let Some(modules) = &e.cycles_in {
        if modules.is_empty() {
            bail!("expect.cycles_in must contain at least one known module ID");
        }
        for module in sorted(modules) {
            if !doc.modules.iter().any(|m| m.id == module) {
                bail!(
                    "unknown module {module:?} in expect.cycles_in: use explain to obtain an exact module ID"
                );
            }
            let count = doc
                .cycles
                .iter()
                .filter(|c| {
                    c.members.iter().any(|m| m == module)
                        || c.members
                            .iter()
                            .any(|p| doc.files.iter().any(|f| f.path == *p && f.module == module))
                })
                .count();
            checks.push(json!({"predicate":"cycles_in","module":module,"expected":e.equals,"actual":count,"pass":count==e.equals}));
        }
    }
    if let Some(n) = e.forbidden {
        checks.push(json!({"predicate":"forbidden","expected":n,"actual":doc.summary.forbidden,"pass":n==doc.summary.forbidden}));
    }
    if let Some(ids) = &e.findings_absent {
        if ids.is_empty() {
            bail!("expect.findings_absent must contain at least one finding_id");
        }
        for id in sorted(ids) {
            if !id.starts_with("ag-")
                || id.len() != 67
                || !id[3..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("invalid finding_id {id:?}: use an ag- ID returned by check_diff or verify");
            }
            let present = doc.findings.iter().any(|f| finding_id(f) == id);
            checks.push(json!({"predicate":"finding_absent","finding_id":id,"pass":!present}));
        }
    }
    if let Some(edges) = &e.absent_edges {
        if edges.is_empty() {
            bail!("expect.absent_edges must contain at least one edge");
        }
        let mut edges: Vec<_> = edges.iter().collect();
        edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        for edge in edges {
            checks.push(json!({"predicate":"import_absent","from":edge.from,"to":edge.to,"pass":!doc.edges.iter().chain(&doc.external_edges).any(|e|e.from==edge.from&&e.to==edge.to)}));
        }
    }
    for (paths, present) in [(&e.files_present, true), (&e.files_absent, false)] {
        if let Some(paths) = paths {
            if paths.is_empty() {
                bail!("file expectations must contain at least one repository-relative path");
            }
            for path in sorted(paths) {
                if !crate::collect::is_relative_path(path, false) {
                    bail!("expected file {path:?} must be repository-relative");
                }
                let found = doc.files.iter().any(|f| f.path == path);
                checks.push(json!({"predicate":if present {"file_present"} else {"file_absent"},"path":path,"pass":found==present}));
            }
        }
    }
    Ok(checks)
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn array(items: Value) -> Value {
    json!({"type":"array","items":items})
}
fn strings() -> Value {
    array(json!({"type":"string"}))
}
fn number() -> Value {
    json!({"type":"integer","minimum":0})
}
fn schema(
    name: &'static str,
    description: &'static str,
    mut input: Value,
    required: &[&str],
    output: Value,
) -> ToolSchema {
    input.as_object_mut().expect("schema properties").insert("brief".into(),json!({"type":"boolean","default":true,"description":"Prefer compact output. All responses retain a hard output cap."}));
    ToolSchema {
        name,
        description,
        input: object(input, required),
        output,
    }
}
/// Explicit wire schemas are also exercised against real tool results in tests.
pub fn schemas() -> Vec<ToolSchema> {
    let edge = object(
        json!({"from":{"type":"string"},"to":{"type":"string"}}),
        &["from", "to"],
    );
    let cycle = json!({"type":"object","required":["level","members"],"properties":{"level":{"enum":["file","module"]},"members":strings()}});
    let finding = json!({"type":"object","required":["finding_id","kind","subject","evidence"],"properties":{"finding_id":{"type":"string","pattern":"^ag-[0-9a-f]{64}$"},"kind":{"type":"string"},"subject":{"type":"string"},"evidence":{"type":"object"},"suggestion":{"type":["object","null"]}}});
    let graph_schema: Value =
        serde_json::from_str(include_str!("../schema/archgraph.v1.2.schema.json"))
            .expect("recipe schema");
    let expect = inline_schema(
        &graph_schema["$defs"]["recipe-predicate"],
        &graph_schema["$defs"],
    );
    let counts = object(
        json!({"cycles":number(),"forbidden":number(),"two_way_pairs":number()}),
        &["cycles", "forbidden", "two_way_pairs"],
    );
    let mut tools = vec![
        schema(
            "context",
            "Select an ordered, budgeted file list with tiers and reasons; source bodies are omitted.",
            json!({"task":{"type":"string"},"files":strings(),"symbols":strings(),"budget_tokens":{"type":"integer","minimum":1,"maximum":1000000}}),
            &["task", "budget_tokens"],
            json!({"type":"object","required":["entries","budget","used_tokens"],"properties":{"entries":array(json!({"type":"object","required":["path","tier","reason","tokens"],"properties":{"path":{"type":"string"},"tier":{"type":"string"},"reason":{"type":"string"},"tokens":number(),"content":{"type":"null"}}})),"budget":number(),"used_tokens":number()}}),
        ),
        schema(
            "check_diff",
            "Compare the current scan with a Git base or pre-edit snapshot; report new forbidden imports, concrete newly cyclic imports, advisory folder cycles and stable finding IDs.",
            json!({"base":{"type":"string","minLength":1},"snapshot":{"type":"string","pattern":"^[0-9a-f]{64}$","description":"Pre-edit capture ID; mutually exclusive with base."}}),
            &[],
            json!({"type":"object","required":["new_forbidden","new_cycles","grown_cycles","findings"],"properties":{"new_forbidden":array(json!({"type":"object","required":["from","to"]})),"new_cyclic_imports":array(json!({"type":"object","required":["from","to"],"properties":{"from":{"type":"string"},"to":{"type":"string"}}})),"new_cycles":array(cycle.clone()),"grown_cycles":array(object(json!({"before":cycle.clone(),"after":cycle.clone()}),&["before","after"])),"findings":array(finding.clone())}}),
        ),
        schema(
            "propose",
            "Return evidence-backed ordered move/cut proposals with explicit expected deltas and verify expectations.",
            json!({"finding_id":{"type":"string","pattern":"^ag-[0-9a-f]{64}$"}}),
            &["finding_id"],
            {
                let mut schema =
                    inline_schema(&graph_schema["$defs"]["recipe"], &graph_schema["$defs"]);
                schema["properties"]["finding_id"] =
                    json!({"type":"string","pattern":"^ag-[0-9a-f]{64}$"});
                schema["required"]
                    .as_array_mut()
                    .expect("recipe required")
                    .push(json!("finding_id"));
                schema
            },
        ),
        schema(
            "simulate",
            "Simulate a batch of file moves without editing files; compare cycle and two-way-pair counts.",
            json!({"moves":{"type":"array","minItems":1,"items":edge}}),
            &["moves"],
            object(
                json!({"before":counts.clone(),"after":counts,"pairs_removed":{"type":"integer"},"new_cycles":array(cycle),"limitations":strings()}),
                &[
                    "before",
                    "after",
                    "pairs_removed",
                    "new_cycles",
                    "limitations",
                ],
            ),
        ),
        schema(
            "verify",
            "Rescan and evaluate optional explicit expectations before output truncation; inspect current findings otherwise.",
            json!({"expect":expect}),
            &[],
            json!({"type":"object","required":["pass","expectations_supplied","checks","summary","findings"],"properties":{"pass":{"type":"boolean"},"expectations_supplied":{"type":"boolean"},"checks":array(json!({"type":"object","required":["predicate","pass"],"properties":{"predicate":{"type":"string"},"pass":{"type":"boolean"}}})),"summary":object(json!({"cycles":number(),"forbidden":number()}),&["cycles","forbidden"]),"findings":array(finding)}}),
        ),
        schema(
            "explain",
            "Explain one exact path or module using imports, importers, layer, rules and available history facts.",
            json!({"path":{"type":"string","minLength":1},"module":{"type":"string","minLength":1}}),
            &[],
            json!({"type":"object","required":["subject","module","imports","importers","rules","facts_available"],"properties":{"subject":{"type":"string"},"module":{"type":"string"},"grouping":{"type":"string"},"source_semantics":{"enum":["rust-source-references","import-dependencies"]},"layer_status":{"enum":["uniform","mixed","unclassified"]},"layers":{"type":"array","items":{"type":"array","prefixItems":[{"type":"integer","minimum":0},{"type":"string"}],"minItems":2,"maxItems":2}},"imports":array(json!({"type":"object","required":["from","to","kind"]})),"importers":array(json!({"type":"object","required":["from","to","kind"]})),"rules":{"type":"object"},"hotspots":{"type":"array"},"history":{"type":"array"},"facts_available":object(json!({"hotspots":{"type":"boolean"},"history":{"type":"boolean"}}),&["hotspots","history"])}}),
        ),
        schema(
            "rules",
            "Return the configured layout and surface constraints relevant to repository-relative paths.",
            json!({"paths":{"type":"array","minItems":1,"items":{"type":"string","minLength":1}}}),
            &["paths"],
            json!({"type":"object"}),
        ),
    ];
    tools
        .iter_mut()
        .find(|t| t.name == "explain")
        .unwrap()
        .input["oneOf"] = json!([
        {"required":["path"],"not":{"required":["module"]}},
        {"required":["module"],"not":{"required":["path"]}}
    ]);
    tools.iter_mut().find(|t| t.name == "rules").unwrap().output = json!({
        "type":"object","anyOf":[{"required":["version","modules","layers","public","entrypoints","forbid","allow"]},{"required":["configured","paths","hint"]}],
        "properties":{"version":{"type":"integer"},"modules":{"type":"array","items":{"type":"object"}},"layers":{"type":"array","items":{"type":"object"}},"public":{"type":"array","items":{"type":"object"}},"entrypoints":{"type":"array","items":{"type":"object"}},"forbid":{"type":"array","items":{"type":"object"}},"allow":{"type":"array","items":{"type":"object"}},"configured":{"const":false},"paths":strings(),"hint":{"type":"string"}}
    });
    tools
}

fn inline_schema(value: &Value, defs: &Value) -> Value {
    if let Some(reference) = value
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix("#/$defs/"))
    {
        return inline_schema(&defs[reference], defs);
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), inline_schema(value, defs)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|value| inline_schema(value, defs))
                .collect(),
        ),
        _ => value.clone(),
    }
}
