# Changelog

## [Unreleased]

### Edge quality: kinds and weights

- New `archgraph/v1.1` exports distinguish runtime, lazy, type and structural re-export imports; TS/JS import tokenization supplements collector metadata, including function-local `require`.
- Default cycles use runtime edges only. `type_only_cycles` records other cyclic components, including lazy/re-export/mixed cycles and whether they contain a runtime cycle; `--all-kinds` enables broad comparison.
- Configurable `edge_weights` default to runtime 1.0, lazy 0.3, type 0.1 and reexport 0.0. Weighted coupling/cohesion ratios accompany retained integer counts; `forbid`, `allow` and `public` rules accept optional kind filters, including throughout reachable paths.
- Upgraded commands read v1 and v1.1. `--legacy-v1` writes the v1 compatibility shape and old kind names for frozen consumers; analysis still uses the new methodology. See [migration details](docs/guide.md#edge-quality-and-migration).

### Edge quality: barrel resolution

- Named TypeScript/JavaScript and Python imports follow pure forwarding chains to defining files. Python resolution handles relative imports, aliases, static `__all__`, star-bound package attributes and package/submodule precedence without executing source.
- Pure forwarding nodes carry `barrel: true`; initializers containing executable statements retain runtime load edges. Resolved edges retain `original_to` so layout rules enforce the written boundary.
- Ambiguous namespace/star imports and chains exceeding ten hops retain their barrel target with low confidence and a diagnostic reason. Forwarding loops produce `reexport-cycle` findings; old collector recordings remain readable.

### Edge quality: exclusions and tests

- Shared defaults exclude test, generated/build, fixture/story/example and dependency/vendor content. `show-excludes` lists the policy; `--include-tests`, `--include-generated`, layout `exclude`/`include`, `.gitignore` and `.archgraphignore` control selection.
- Explicit excludes take precedence; layout includes can override default and ignore-file exclusions. Included tests occupy separate test modules, stay out of cycle detection and provide a minimal importing-test coverage summary.
- Python test inclusion discovers files outside the selected package through AST inspection, retaining their import edges without executing tests.

### Edge quality: resolution accounting

- Explicit import ledgers retain specifiers, source lines, kinds and resolution evidence independently of file-reference edges; `report` shows resolution coverage and `unresolved` lists failures with reasons. Legacy recordings keep accounting unavailable until rescanned.
- Common TypeScript aliases/package maps/workspaces and Python relative, `src/`, namespace and repeatable `--python-path` imports gain resolution evidence. Python inspects external module specs without executing package initializers.
- Known externals use separate `external_nodes`/`external_edges` arrays and stay out of internal cycle/cohesion metrics. Known destinations excluded by scan policy remain resolved in the ledger.

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
