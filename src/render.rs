//! One self-contained HTML page per scan document.

use anyhow::Result;

use crate::model::Document;

const TEMPLATE: &str = include_str!("../assets/graph.html");

/// The page with `doc` embedded as JSON. Every `<` is escaped, so no path can close the script
/// element it sits in.
pub fn render(doc: &Document) -> Result<String> {
    let json = serde_json::to_string(doc)?.replace('<', "\\u003c");
    Ok(TEMPLATE.replace("{{GRAPH}}", &json))
}
