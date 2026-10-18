//! Bounded projections for agent context windows; full documents keep their existing schema.
use serde_json::{Map, Value, json};

const ITEMS: usize = 20;
const NODES: usize = 240;
const TEXT: usize = 512;

/// At most twenty array entries across the result, with explicit JSON-pointer omissions.
pub(super) fn project(value: &Value) -> Value {
    let mut omitted = Map::new();
    let mut items = ITEMS;
    let mut nodes = NODES;
    let result = compact(value, "", 0, &mut items, &mut nodes, &mut omitted);
    let counts: Map<String, Value> = value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            value
                .as_array()
                .map(|items| (key.clone(), json!(items.len())))
        })
        .collect();
    json!({
        "schema": "archgraph/brief/v1",
        "kind": value.get("kind").cloned().unwrap_or(Value::Null),
        "result": result,
        "counts": counts,
        "omitted": omitted,
        "hint": "Remove --brief for the complete output."
    })
}

fn compact(
    value: &Value,
    path: &str,
    depth: usize,
    items: &mut usize,
    nodes: &mut usize,
    omitted: &mut Map<String, Value>,
) -> Value {
    if matches!(
        path,
        "/summary/modules" | "/before/modules" | "/after/modules"
    ) && let Some(values) = value.as_object()
    {
        omitted.insert(
            path.into(),
            json!({"fields": values.len(), "reason": "per-module metrics; use full output"}),
        );
        return json!({});
    }
    if *nodes == 0 || depth == 8 {
        omitted.insert(path.into(), json!({"reason": "size limit"}));
        return Value::Null;
    }
    *nodes -= 1;
    match value {
        Value::Array(values) => {
            let mut result = Vec::new();
            for (index, value) in values.iter().enumerate() {
                if *items == 0 || *nodes == 0 {
                    omitted.insert(path.into(), json!({"items": values.len() - index}));
                    break;
                }
                *items -= 1;
                result.push(compact(
                    value,
                    &format!("{path}/{index}"),
                    depth + 1,
                    items,
                    nodes,
                    omitted,
                ));
            }
            Value::Array(result)
        }
        Value::Object(values) => {
            // Evidence and counts precede inventories, regardless of input object ordering.
            let priority = [
                "schema",
                "kind",
                "summary",
                "grouping",
                "source_semantics",
                "import_resolution",
                "risk",
                "status",
                "subject",
                "task",
                "budget",
                "used_tokens",
                "limitations",
                "findings",
                "entries",
                "changed_files",
                "affected_modules",
                "selected_tests",
                "reviewers",
                "cycles",
            ];
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_by_key(|key| {
                (
                    priority
                        .iter()
                        .position(|p| p == key)
                        .unwrap_or(priority.len()),
                    *key,
                )
            });
            let mut result = Map::new();
            for (index, key) in keys.iter().enumerate() {
                let child = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                if *nodes == 0 {
                    omitted.insert(
                        path.into(),
                        json!({"fields": keys.len() - index, "reason": "size limit"}),
                    );
                    break;
                }
                result.insert(
                    (*key).clone(),
                    compact(&values[*key], &child, depth + 1, items, nodes, omitted),
                );
            }
            Value::Object(result)
        }
        Value::String(text) if text.chars().count() > TEXT => {
            omitted.insert(
                path.into(),
                json!({"characters": text.chars().count() - TEXT}),
            );
            Value::String(text.chars().take(TEXT).collect())
        }
        _ => value.clone(),
    }
}
