//! Explicit compatibility export; analysis always uses the current schema.
pub fn v1(value: &mut serde_json::Value) -> anyhow::Result<()> {
    if value["schema"] == crate::model::SCHEMA
        && value["kind"] == "scan"
        && (value["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|f| f["class"] == "tests"))
            || value["config"]["granularity"] == "file")
    {
        let mut input: crate::model::ScanInput = serde_json::from_value(value.clone())?;
        input.config.legacy_module_classes = true;
        *value = serde_json::to_value(crate::restore_scan(input)?)?;
        value["config"]
            .as_object_mut()
            .unwrap()
            .remove("granularity");
    }

    match value {
        serde_json::Value::Object(object) => {
            if matches!(
                object.get("schema").and_then(|v| v.as_str()),
                Some("boundrail/v1.1" | "boundrail/v1.2")
            ) {
                object.insert("schema".into(), "boundrail/v1".into());
            }
            if let Some(serde_json::Value::Array(diagnostics)) = object.remove("diagnostics")
                && let Some(findings) = object.get_mut("findings").and_then(|v| v.as_array_mut())
            {
                findings.extend(
                    diagnostics
                        .into_iter()
                        .filter_map(|mut diagnostic| diagnostic.as_object_mut()?.remove("finding")),
                );
            }
            if let Some(findings) = object.get_mut("findings").and_then(|v| v.as_array_mut()) {
                findings.retain(|finding| {
                    !matches!(
                        finding["kind"].as_str(),
                        Some("forbidden-edge" | "deep-import")
                    )
                });
                findings.sort_by(|a, b| {
                    (a["kind"].as_str(), a["subject"].as_str())
                        .cmp(&(b["kind"].as_str(), b["subject"].as_str()))
                });
            }
            object.remove("recipe");
            object.remove("finding_id");
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
