# Changelog

## [Unreleased]

- `archgraph/v1.2` adds stable finding IDs, typed fix recipes with structured verification and expected graph deltas, explicit forbidden-edge/deep-import findings, and advisory diagnostics for evidence without concrete edits. Existing v1/v1.1 inputs remain readable; compatibility exports retain the legacy finding shape.
- `propose`, `apply --dry-run|--yes` and `verify` share recipes with MCP. Conservative mechanical plans use exact AST byte ranges, checked Git moves, local proposal/apply baselines, post-edit rescans and rollback on verification failure. Interface extraction and splits remain agent instructions; unavailable predictions are explicit rather than fabricated.
- Finding ranking defaults to hotspot × confidence × pair reduction; `--rank legacy` retains the previous hotspot-first ordering. Guide and agent skills explain planning, staged moves versus worktree rewrites, verification predicates and language limits. Pinned showcase recipe outcomes are recorded separately.

- Git dirty-state proofs now compare text content before claiming a tracked edit; stat-only, binary and metadata-only cases defer to full status, avoiding false dirty provenance after rapid edit/revert.

- `mcp` exposes context, diff checks, proposals, move simulation, verification, explanations and relevant rules over stdio through the official Rust MCP SDK. Typed tool schemas, stable semantic finding IDs and bounded default answers support agent clients; a matching local daemon can supply live scans. Includes Claude Code, Cursor and Codex configs and MCP-first skill guidance. The MCP transport does not require graph-format changes; recipe support is described separately above.

- `serve` keeps a watched graph available over a local authenticated Unix socket or loopback TCP endpoint. CLI commands discover it automatically, `--no-daemon` bypasses it and `--once` warms/answers one request without a watcher. Native TS/JS, Python and Go reuse parsed source; conservative dirty-region analysis and logged full-refresh fallbacks preserve correctness. Retains two graph generations plus indexes; latency benchmarks are recorded separately from parser timings.
- Global `--brief` provides bounded agent output with explicit omissions; structured check/report/context/impact/surface outputs share their standalone and daemon execution paths. Adds graph-evidence `explain` and daemon `status` commands while preserving policy exit codes and complete output artifacts.
- Scan `source.generated_at` now uses `SOURCE_DATE_EPOCH`, Git HEAD commit time or the Unix epoch, making repeated scans reproducible for identical inputs. It no longer represents wall-clock execution time; v1/v1.1 graph shapes remain readable.

- Experimental `--engine native` collects all supported languages through embedded tree-sitter grammars and queries. TypeScript resolution uses oxc_resolver; Python, Rust, Java and Elixir resolution runs in Rust without language runtimes, analyzers or project builds. Go uses read-only `go list` package metadata and retains conservative move/symbol limits. Reference collectors remain the default; a pinned 20-repository TypeScript/Python comparison records endpoint, kind, scope and resolution disagreement rather than assuming a speed or correctness gain. `--engine depcruise` remains the TypeScript escape hatch. Release archives include grammar/runtime attributions.

- Elixir source-reference collection through `Code.string_to_quoted`, including umbrella apps, aliases, static module calls and type references. Scans do not load Mix projects, evaluate source, compile or expand macros; unresolved dynamic and unavailable dependency references remain visible. Adds runtime selection, shared exclusions and live collector CI coverage.

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
- Python local module probes require exact path casing, preventing imported class names from creating duplicate modules on case-insensitive filesystems.
- Known externals use separate `external_nodes`/`external_edges` arrays and stay out of internal cycle/cohesion metrics. Known destinations excluded by scan policy remain resolved in the ledger.

### Edge quality: grab-bags and granularity

- Configurable `grabbag` thresholds identify low-cohesion, high-fan-in folders with sparse internal edges. `cycle-via-grabbag` findings require a verified per-file split and retain involved-file evidence plus surviving real cycles/cuts; overlapping residuals that recreate the original cycle keep that cycle conservatively.
- Global `--granularity file|folder` selects graph grouping while preserving folder policy boundaries. The choice persists through historical replay and caches; legacy v1 export regroups file views into a compatible folder view.
- Included test modules stay out of grab-bag candidates and cycle probes; visible weighted fan-in continues to reflect included test consumers.

### Added

- External package policies via `pkg:<glob>` and declared `category:<name>` targets, including kind-filtered reachable checks; per-module occurrence/weighted exposure with an advisory threshold; external-free layer inference; and an inspectable package ring in maps. Missing import accounting remains unavailable, and live what-if previews explicitly defer external verdicts to Rust.

- Declarative `services` analysis from YAML/JSON: transport-aware HTTP/gRPC/queue edges with confidence and source evidence, service SCCs, distinct fan-in hubs, instability and protocol-filtered forbid rules. Adds a standalone v1.1 service report and input schema; automatic discovery and service maps remain separate milestones.

- Live What-if map previews: drag or enter a file move, compare cycles, two-way pairs, weighted instability and cycle/rule findings, clear transient state, and copy quoted Rust simulation and Git move commands. Rust-generated parity fixtures validate the dependency-free JS preview; unsupported refinements are labeled. Includes a recorded WebP demo.

- `tour` generates a Markdown reading order from saved graphs: dependency layers, fan-in priorities, grouped cycles with a suggested starting module, reversed layers, and optional saved export summaries for agents.

- Budgeted `context` reading plans from saved graphs, with required layout constraints, import/importer tiers, barrel deduplication, export summaries and omission reasons; project skill and opt-in Claude Code/Cursor adapters. Includes a frozen 30-PR retrieval benchmark against grep + folder, reporting discovery and full-body recall separately.

- `package-splits` ranks cohesive, externally used folder subtrees by importing modules × production files, with free/cheap classifications, full cut lists, workspace checks, uncertainty guards and cycle simulations. Optional graph attachments survive replay and power map island highlights; build and packaging compatibility remain unverified.
- New scans record an optional Git-relative `source.scan_root` to bind workspace metadata to the correct scanned directory. Older scans remain readable and receive conservative workspace-readiness labels.
- `infer-layout` drafts annotated layout rules from saved graphs: SCC condensation, longest-path layers, unresolved cycle groups, and sparse-boundary proposals with file-pair evidence and existing violations. It prints by default; `--apply` creates a new draft without overwriting existing files.
- Public surface reports for TypeScript/JavaScript, Python and Go: symbol exports and external usage, deep imports, advisory unused exports and oversized surfaces. Optional symbol evidence keeps the `archgraph/v1` document format compatible with earlier scans.
- Strict `entrypoints` rules for module boundaries, integrated with `check`, plus surface metrics and ring thickness in the architecture map. Incomplete symbol coverage withholds unused-export suggestions.
- Java source-reference collection through the JDK and Rust workspace collection through `rust-analyzer`, with build scripts and proc macros disabled by default.
- CODEOWNERS summaries and ownership-aware placement findings.
- Architecture bisect with verified introduction commits, age, author and optional PR attribution.
- Explainable hotspots from Git churn, module coupling, cycles and author counts, with finding re-ranking and churn maps.
- Cached Git architecture timelines, incremental append, release comparisons, drift charts and a scheduled monthly history workflow.
- Diff impact reports with affected modules, test selection, reviewer suggestions and cycle/hotspot risk; TypeScript barrel resolution, Go package test evidence, JSON exports and optional PR comments.
- Repeatable collector benchmarks and a pinned 20-repository hotspot study: the top 10% of files captured 72.2% of fix-message commits versus 19.6% adjusted random expectation. This is an in-sample association, not a prediction or causal claim; see [the study](benchmarks/HOTSPOTS.md).
