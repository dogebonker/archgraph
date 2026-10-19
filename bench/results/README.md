# First paired results: 2026-10-19

All 24 predeclared runs are retained: one synthetic utility feature per repository,
three repetitions per arm, Codex 0.163.0 with `gpt-6.1-sol`. Flask and tRPC are
showcase repos; HTTPX and Pydantic substitute for the unavailable Perf tasks.
These are isolated feature oracles, not full upstream suites.

| Repo | Feature checks bare / guarded | Final new violations bare / guarded | Verified guarded activation | Agent seconds bare / guarded, all-run mean |
| --- | --- | --- | --- | --- |
| [Flask](flask-2026-10-19.md) | 3/3 / 3/3 | 0 / 0 | 3/3 | 15.2 / 25.5 |
| [tRPC](trpc-2026-10-19.md) | 3/3 / 3/3 | 0 / 0 | 2/3 | 20.1 / 98.8 |
| [HTTPX](httpx-2026-10-19.md) | 3/3 / 3/3 | 0 / 0 | 3/3 | 14.9 / 25.7 |
| [Pydantic](pydantic-2026-10-19.md) | 3/3 / 0/3 | 0 / 0 | 0/3 | 18.7 / 15.8* |

No feature-run architecture blocks or verified auto-apply receipts occurred. These
tasks show **no architecture benefit**, with additional guarded runtime and three
guarded failures. *Pydantic's short guarded time reflects blocked work, not faster
completion. Its failures and tRPC's unknown activation are excluded from efficacy
means but remain in every per-run table. All-run times above deliberately include
them; each report also gives eligible-only means and sample standard deviations.

One tRPC guarded run attempted concurrent commands while a long-running shell MCP
client held the edit lease. Two PreToolUse calls returned UNAVAILABLE; the agent
recovered, and final tests and graph checks passed. The predeclared requirement
of no unavailable checks leaves its activation unknown. Pydantic's three guarded
runs could not start feature work because snapshots rejected the tracked contained
directory symlink `tests/pydantic_core → ../pydantic-core/tests`. That repository-wide
inventory restriction applies before the selected Python package is scanned. The
full graph check still returns zero new violations because no feature edit occurred.
No failed run was replaced.

After these frozen runs, contained directory links were supported without traversing
aliases, and generated instructions gained an explicit CLI fallback when MCP is not
connected. A [release-binary Pydantic CLI regression](pydantic-dirlink-fix-2026-10-19.json)
now passes capture → BLOCK → same-baseline repair → PASS, while explicit alias edits
remain refused. This is a targeted regression check; the feature tasks were not rerun
or rescored, and no post-fix efficacy improvement is claimed.

The frozen binary is a debug build from source commit `8075782`, SHA-256
`67e5ac4fe800772c2d2ab21930027187d62913ab96a2694195677216bb979a52`.
At most two repository benchmarks ran concurrently, with alternating arm order;
no warm daemon was started. Times include the model and cold hook checks, so they
are separate from the release-binary warm latency measurements. Seeds are not
supported by this runner. Native token totals, cached-input subsets, command
output, source/policy hashes and every diff size are in the JSON beside each report.

The [case provenance and oracle checks](public-cases-2026-10-19.json) record public
upstream pins, deterministic baseline commits and the oracle digest. The canonical
oracle stayed outside the agent workspace and its before/after digest matched.
Every pristine baseline failed its intended assertion, and a discarded minimal
implementation passed. Reproduce with the [case preparation](../cases/README.md)
and [runner instructions](../README.md); model output and runtime can vary.

The separate [native loop smoke](codex-live-loop-2026-10-19.json) did observe an
intentional forbidden import blocked with a recipe, same-snapshot repair and PASS.
It verifies the integration path and is not added to the feature efficacy counts.
Live Claude, Cursor and OpenCode sessions and Perf-specific coverage remain untested.
