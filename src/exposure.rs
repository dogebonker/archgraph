//! External import exposure uses collector occurrences, independently of internal coupling.
use crate::model::Document;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub max_percent: f64,
}
impl Default for Thresholds {
    fn default() -> Self {
        Self { max_percent: 80.0 }
    }
}
impl Thresholds {
    pub fn validate(self) -> Result<()> {
        if !self.max_percent.is_finite() || !(0.0..=100.0).contains(&self.max_percent) {
            bail!("external_exposure.max_percent must be finite and between 0 and 100");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub threshold_percent: f64,
    pub modules: Vec<ModuleExposure>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageExposure {
    pub package: String,
    pub imports: usize,
    pub weight: f64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleExposure {
    pub module: String,
    pub imports: usize,
    pub external_imports: usize,
    pub external_packages: usize,
    pub external_weight: f64,
    pub total_weight: f64,
    pub external_percent: Option<f64>,
    pub weighted_external_percent: Option<f64>,
    pub unresolved_imports: usize,
    pub exceeds_threshold: bool,
    pub packages: Vec<PackageExposure>,
}
pub fn report(doc: &Document) -> Option<Report> {
    let imports = doc.imports.as_ref()?;
    let rules = doc.config.rules.as_ref().map(|r| &r.doc);
    let threshold_percent = rules
        .and_then(|r| r.external_exposure)
        .unwrap_or_default()
        .max_percent;
    let weights = rules.and_then(|r| r.edge_weights).unwrap_or_default();
    let modules: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.module.as_str()))
        .collect();
    let mut rows: BTreeMap<String, ModuleExposure> = doc
        .modules
        .iter()
        .map(|m| {
            (
                m.id.clone(),
                ModuleExposure {
                    module: m.id.clone(),
                    ..Default::default()
                },
            )
        })
        .collect();
    let mut packages: BTreeMap<(String, String), PackageExposure> = BTreeMap::new();
    for import in imports {
        let Some(module) = modules.get(import.from.as_str()) else {
            continue;
        };
        let row = rows.get_mut(*module).expect("file module exists");
        row.imports += 1;
        let weight = weights.weight(import.kind);
        row.total_weight += weight;
        row.unresolved_imports += usize::from(import.unresolved.is_some());
        if let Some(package) = &import.external {
            row.external_imports += 1;
            row.external_weight += weight;
            let item = packages
                .entry(((*module).to_owned(), package.clone()))
                .or_insert_with(|| PackageExposure {
                    package: package.clone(),
                    ..Default::default()
                });
            item.imports += 1;
            item.weight += weight;
        }
    }
    for ((module, _), package) in packages {
        rows.get_mut(&module)
            .expect("module exists")
            .packages
            .push(package);
    }
    let percent = |numerator: f64, denominator: f64| {
        (denominator > 0.0).then(|| (numerator / denominator * 1_000_000.0).round() / 10_000.0)
    };
    for row in rows.values_mut() {
        row.external_packages = row.packages.len();
        row.external_percent = percent(row.external_imports as f64, row.imports as f64);
        row.weighted_external_percent = percent(row.external_weight, row.total_weight);
        row.exceeds_threshold = row.imports > 0
            && row.external_imports as f64 / row.imports as f64 * 100.0 > threshold_percent;
    }
    Some(Report {
        threshold_percent,
        modules: rows.into_values().collect(),
    })
}

/// Aggregates can overflow even when each configured edge weight is finite.
pub fn validate(doc: &Document) -> Result<()> {
    crate::resolution::validate_external_identities(
        &doc.source,
        doc.imports.as_deref(),
        &doc.files,
    )?;
    if doc.external_exposure.as_ref().is_some_and(|report| {
        report.modules.iter().any(|row| {
            !row.external_weight.is_finite()
                || !row.total_weight.is_finite()
                || row.external_percent.is_some_and(|v| !v.is_finite())
                || row
                    .weighted_external_percent
                    .is_some_and(|v| !v.is_finite())
                || row.packages.iter().any(|p| !p.weight.is_finite())
        })
    }) {
        bail!("edge_weights overflow the external exposure metrics; use smaller finite weights");
    }
    Ok(())
}
