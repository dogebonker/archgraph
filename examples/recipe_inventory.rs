//! Recompute ranked findings from a saved graph for the pinned showcase recipe audit.
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: recipe_inventory graph.json")?;
    let input = serde_json::from_slice(&std::fs::read(path)?)?;
    let doc = archgraph::restore_scan(input)?;
    println!("{}", serde_json::to_string(&doc.findings)?);
    Ok(())
}
