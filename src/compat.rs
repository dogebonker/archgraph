//! Explicit compatibility export; analysis always uses the current schema.
pub fn v1(value: &mut serde_json::Value) -> anyhow::Result<()> {
    if value["schema"] == crate::model::SCHEMA
        && value["kind"] == "scan"
        && value["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|f| f["class"] == "tests"))
    {
        let mut input: crate::model::ScanInput = serde_json::from_value(value.clone())?;
        input.config.legacy_module_classes = true;
        *value = serde_json::to_value(crate::restore_scan(input)?)?;
    }

    match value {
        serde_json::Value::Object(object) => {
            if object.get("schema").and_then(|v| v.as_str()) == Some(crate::model::SCHEMA) {
                object.insert("schema".into(), "archgraph/v1".into());
            }
            if object.contains_key("from") && object.contains_key("to") {
                let legacy = match object.get("kind").and_then(|v| v.as_str()) {
                    Some("runtime") => Some("static"),
                    Some("lazy") => Some("dynamic"),
                    Some("type")
                        if object.get("is_reexport").and_then(|v| v.as_bool()) == Some(true) =>
                    {
                        Some("type-reexport")
                    }
                    _ => None,
                };
                if let Some(kind) = legacy {
                    object.insert("kind".into(), kind.into());
                }
            }
            for child in object.values_mut() {
                v1(child)?;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                v1(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
