# Architecture context delivery

Requested order: CODEOWNERS, architecture bisect, hotspots, incremental drift history.

1. CODEOWNERS: auto-discover at repository root, preserve matching rules in scan documents, assign file owners, summarize module responsibility. Qualify misplaced-file findings with ownership support/conflict. Keep dependency/rule/simulation gates; conflicting ownership suppresses automatic move suggestions. Old scans without ownership retain their behavior. Verify syntax precedence, clearing, subdirectory scans, shared ownership, recomputation, history and simulation.
2. Bisect: identify the selected cycle/forbidden edge, use pickaxe candidates followed by graph verification, fall back to bounded commit probing. Attribute author/date and offline PR references. Distinguish verified introduction, already-present-at-bound, unavailable and uncertain results; avoid earliest-ever claims for nonmonotonic history. Preserve checkout and cache repeated probes. Expose provenance and finding age without guessing unsupported finding types.
3. Hotspots: collect bounded, rename-aware churn within the scanned root; exclude mechanical commits; show normalized factors and score, attach finding priorities and cut candidates, expose map churn mode. Shallow history has an explicit data gap. Validate known synthetic cases and measure bug-fix concentration on the showcase repositories; report coverage and failures alongside numbers.
4. Drift: monthly/every-N/tag sampling, cached full snapshots and summaries, append only missing points, skip/log failed scans, show absolute and normalized trends with maps and provenance annotations. Support release diff and scheduled CI append. Cache identity includes scan options and collector/tool identity; historical snapshots use their own rules/ownership by default. No automatic target build/install execution.

Validation: targeted regressions first; then full Rust tests, live relevant collectors, JSON schemas, formatting/Clippy, Node UI tests, repeatable source benchmarks. Each stage is integrated and verified before proceeding to the next; independent engine preparation can run in parallel. No new dependencies. Preserve the existing uncommitted work.

Final cleanup: use the shared library scan validator from CLI input reads; existing inconsistent-scan and rule-override regressions lock the contract. Preserve historical context only when the configuration is unchanged.

## Completion evidence

- CODEOWNERS discovery, matching, responsibility summaries and placement/move guards implemented. Embedded ownership survives recompute and simulated moves.
- Bisect/provenance CLI and automatic report context implemented, including generic findings and historical cochange. Actual-parent verification prevents false attribution through backdated date-filter gaps; ambiguous/unsupported boundaries remain explicit.
- Hotspots collection, readable factors, finding ranking, churn map and commit-size-adjusted random comparison implemented. All 20 original showcase repositories completed: 3,794/5,253 fix-message commits touched the top approximately 10% of scored files (72.2%); adjusted random expectation19.6%, ratio3.69x.
- Monthly/interval/tag drift, full scan cache, append, failure records, release markers, normalized metrics, cached inflection provenance and release diff implemented. Monthly CI definition added.
- Final local evidence:187 default Rust tests,17 live Rust tests,10 UI tests,10 npm wrapper tests,5 Python benchmark tests; Clippy, formatting, actionlint and diff checks. Independent review approved. Browser layout, authenticated gh lookup and deployed Actions were not executed.

Artifacts: benchmarks/HOTSPOTS.md and benchmarks/results/hotspots-validation.json. The statistical claim is in-sample association, not causal or predictive evidence.
