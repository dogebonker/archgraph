//! Self-contained HTML pages for scan and service reports.

use anyhow::Result;

use crate::model::Document;

pub(crate) const TEMPLATE: &str = include_str!("../assets/graph.html");
const PREVIEW: &str = include_str!("../assets/whatif.js");
const PREVIEW_UI: &str = include_str!("../assets/whatif-ui.js");

pub(crate) fn template() -> String {
    TEMPLATE
        .replacen("/*__ARCHGRAPH_PREVIEW__*/", PREVIEW, 1)
        .replacen("/*__ARCHGRAPH_PREVIEW_UI__*/", PREVIEW_UI, 1)
}

/// The page with `doc` embedded as JSON. Every `<` is escaped, so no path can close the script
/// element it sits in.
pub fn render(doc: &Document) -> Result<String> {
    let json = serde_json::to_string(doc)?.replace('<', "\\u003c");
    Ok(template().replacen("{{GRAPH}}", &json, 1))
}

/// The self-contained service catalog, with repository strings embedded only as JSON data.
pub fn render_services(report: &crate::services::Report) -> Result<String> {
    let json = serde_json::to_string(report)?.replace('<', "\\u003c");
    Ok(include_str!("../assets/services.html").replacen("{{SERVICES}}", &json, 1))
}

/// An offline before/after view. Repository data cannot terminate the JSON script element.
pub fn render_comparison(view: &crate::comparison::View) -> Result<String> {
    let json = serde_json::to_string(view)?.replace('<', "\\u003c");
    Ok(include_str!("../assets/comparison.html").replacen("{{COMPARISON}}", &json, 1))
}
