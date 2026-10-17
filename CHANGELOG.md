# Changelog

## [Unreleased]

### Added

- Public surface reports for TypeScript/JavaScript, Python and Go: symbol exports and external usage, deep imports, advisory unused exports and oversized surfaces. Optional symbol evidence keeps the `archgraph/v1` document format compatible with earlier scans.
- Strict `entrypoints` rules for module boundaries, integrated with `check`, plus surface metrics and ring thickness in the architecture map. Incomplete symbol coverage withholds unused-export suggestions.
- Java source-reference collection through the JDK and Rust workspace collection through `rust-analyzer`, with build scripts and proc macros disabled by default.
- CODEOWNERS summaries and ownership-aware placement findings.
- Architecture bisect with verified introduction commits, age, author and optional PR attribution.
- Explainable hotspots from Git churn, module coupling, cycles and author counts, with finding re-ranking and churn maps.
- Cached Git architecture timelines, incremental append, release comparisons, drift charts and a scheduled monthly history workflow.
- Diff impact reports with affected modules, test selection, reviewer suggestions and cycle/hotspot risk; TypeScript barrel resolution, Go package test evidence, JSON exports and optional PR comments.
- Repeatable collector benchmarks and a pinned 20-repository hotspot study: the top 10% of files captured 72.2% of fix-message commits versus 19.6% adjusted random expectation. This is an in-sample association, not a prediction or causal claim; see [the study](benchmarks/HOTSPOTS.md).
