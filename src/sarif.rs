//! SARIF 2.1.0 for GitHub code scanning: one result per import that breaks a claim, at its line,
//! then one per other forbidden import. GitHub tells new results from existing ones itself.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::{Document, Verdict};

const SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";

pub fn sarif(doc: &Document) -> Value {
    let rules = doc.config.rules.as_ref();
    let reasons = |id: &str| rules.map_or_else(Vec::new, |r| r.doc.reasons(id));
    let sources = |id: &str| rules.map_or_else(Vec::new, |r| r.doc.sources(id));
    // (rule id, from, to, message)
    let mut found: Vec<(&str, &str, &str, String)> = Vec::new();
    // Named slices go first so a link also in slice `*` (a rule without a `<name>` capture
    // sharing the id) keeps the named one; later repeats of (id, from, to) are skipped.
    let mut slices: Vec<_> = doc
        .target
        .iter()
        .flat_map(|t| &t.claims)
        .flat_map(|c| c.slices.iter().map(move |s| (c, s)))
        .collect();
    slices.sort_by_key(|(_, s)| s.slice == "*");
    let mut claimed: BTreeSet<(&str, &str, &str)> = BTreeSet::new();
    for (claim, slice) in slices {
        for link in &slice.broken {
            if !claimed.insert((&claim.id, &link.from, &link.to)) {
                continue;
            }
            let mut text = format!(
                "{} imports {}, breaking claim {}",
                link.from, link.to, claim.id
            );
            if slice.slice != "*" {
                text += &format!(" in slice {}", slice.slice);
            }
            text += &format!(": {}", claim.reason.trim_end_matches('.'));
            if let Some(source) = &claim.source {
                text += &format!(" ({source})");
            }
            found.push((&claim.id, &link.from, &link.to, text + "."));
        }
    }
    // A pair under a broken claim, or already listed under another kind, is not repeated.
    let mut seen: BTreeSet<(&str, &str)> = found.iter().map(|(_, f, t, _)| (*f, *t)).collect();
    for e in doc.edges.iter().chain(&doc.external_edges) {
        if e.verdict != Some(Verdict::Forbidden) || !seen.insert((&e.from, &e.to)) {
            continue;
        }
        let rule = e.rule.as_deref().unwrap_or_default();
        let mut text = format!("{} imports {}, forbidden by {rule}", e.from, e.to);
        let (reasons, sources) = (reasons(rule), sources(rule));
        if !reasons.is_empty() {
            text += &format!(": {}", reasons.join("; ").trim_end_matches('.'));
        }
        if !sources.is_empty() {
            text += &format!(" ({})", sources.join(", "));
        }
        found.push((rule, &e.from, &e.to, text + "."));
    }
    let used: BTreeSet<&str> = found.iter().map(|(id, ..)| *id).collect();
    let descriptors: Vec<Value> = used
        .into_iter()
        .map(|id| {
            let reasons = reasons(id);
            let text = if reasons.is_empty() {
                format!("{id} forbids this import")
            } else {
                reasons.join("; ")
            };
            let sources = sources(id);
            let help = if sources.is_empty() {
                text.clone()
            } else {
                format!("Source: {}", sources.join(", "))
            };
            json!({
                "id": id,
                "shortDescription": {"text": text},
                "fullDescription": {"text": text},
                "help": {"text": help},
                "defaultConfiguration": {"level": "error"},
                "properties": {"tags": ["architecture"]},
            })
        })
        .collect();
    let known: BTreeSet<(&str, &str)> = doc
        .edges
        .iter()
        .chain(&doc.external_edges)
        .filter(|e| e.known)
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    let known_file = rules
        .and_then(|r| r.known.as_ref())
        .map_or("the known file", |k| k.path.as_str());
    let root = doc.source.scan_root.as_deref().filter(|r| *r != ".");
    let results: Vec<Value> = found
        .into_iter()
        .map(|(id, from, to, mut text)| {
            let path = root.map_or_else(|| from.to_owned(), |r| format!("{r}/{from}"));
            let mut location =
                json!({"artifactLocation": {"uri": uri(&path), "uriBaseId": "%SRCROOT%"}});
            match crate::target::line(doc, from, to) {
                Some(line) => location["region"] = json!({"startLine": line}),
                None => {
                    text += &format!(
                        " Line unknown: the {} collector records none.",
                        doc.source.collector.name
                    )
                }
            }
            // No line in the hash, so an alert survives the import moving within its file.
            let hash: String = Sha256::digest(format!("{id}\0{from}\0{to}").as_bytes())
                .iter()
                .take(16)
                .map(|b| format!("{b:02x}"))
                .collect();
            let mut result = json!({
                "ruleId": id,
                "level": "error",
                "message": {"text": text},
                "locations": [{"physicalLocation": location}],
                "partialFingerprints": {"primaryLocationLineHash": hash},
            });
            if known.contains(&(from, to)) {
                result["suppressions"] = json!([{
                    "kind": "external",
                    "justification": format!("Listed in {known_file}."),
                }]);
            }
            result
        })
        .collect();
    json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                // GitHub code scanning matches existing results by this persisted identity.
                "name": "archgraph",
                "fullName": "Boundrail",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": env!("CARGO_PKG_REPOSITORY"),
                "rules": descriptors,
            }},
            "results": results,
        }],
    })
}

/// `path` as a URI reference: every byte outside `A-Za-z0-9-._~/` percent-encoded.
fn uri(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri_percent_encodes_every_byte_outside_unreserved_and_slash() {
        assert_eq!(super::uri("src/a b#1.ts"), "src/a%20b%231.ts");
    }
}
