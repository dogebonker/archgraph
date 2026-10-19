# Interoperation

## Shared agent instruction files

`archgraph sync-rules` renders `layout.yaml` into owned blocks in `CLAUDE.md`, `AGENTS.md`, `.cursorrules`, and `.github/copilot-instructions.md`. It preserves bytes outside the fences and refuses to overwrite a hand-edited block. `archgraph init` generates these blocks; `archgraph sync-rules --check` detects stale blocks in CI. Default blocks aim for 600 tokens; `--full` includes the complete rule summary. The blocks direct agents to the graph tools and installed edit hook; native hook support and trust requirements are described in [the edit loop](agents/loop.md).

## ADR proposals and provenance

`archgraph adr import docs/adr` prints a reviewable YAML fragment. It never executes ADR code, rewrites ADRs, or changes architecture rules by default. `--json` prints the complete JSON inventory; `--brief` uses the normal bounded JSON envelope; `--apply` explicitly appends new proposals to `layout.yaml`. Review the draft first. Proposals carry the raw ADR SHA-256; applying rechecks source bytes and lifecycle before planning and again after the final layout comparison. Changed ADRs require a fresh import. Existing comments and rule sections are preserved; ambiguous flow-style sections and YAML anchors are refused. Repeated application skips existing proposal IDs.

The supported decision sentence is deliberately narrow: `` `src/domain/**` must not import `src/adapters/**`. `` Existing module IDs can replace the quoted paths and resolve to their unique declared file glob. Duplicate IDs and captured module patterns remain informational. Only direct text in `## Decision` or `## Decision Outcome` is considered; alternatives, code fences, call restrictions, and ambiguous natural language remain informational. Proposal IDs include the source path, so ADR numbering reused in different directories stays distinct. Every proposal carries `reason: ADR-NNN <title>` and a repository-relative `source:` path.

MADR headings and optional YAML frontmatter are recognized. ADR-kit frontmatter, Status, and Status History must agree. Proposed, rejected, deprecated, and superseded decisions do not generate active rules. Archgate's `id`, `title`, `domain`, `rules`, and `files` identify decisions and scope; its standard format supplies no lifecycle status, so these decisions are explicitly reported as lifecycle unknown. Scope metadata does not imply an import prohibition.

`archgraph adr check` reports imported rules with deprecated/superseded/proposed/rejected sources as potentially stale, missing sources separately, and unknown lifecycle without claiming acceptance. Check output and fix recipe risk notes retain the source citation. Source paths must remain inside the repository and must not traverse symlinks.

A reviewed, deliberately narrow adapter recognizes the exact visible normative Decision contract of pinned Archgate ARCH-018 with `files: ["src/**/*.ts"]`. It proposes `kinds: [runtime]` forbids for `pkg:inquirer`, `pkg:posthog-node`, and `pkg:@sentry/*`; type-only and dynamic imports remain permitted. The named package classes map mechanically, while the clause about unspecified future heavy dependencies remains informational. Changed wording or additional exceptions require a new review; phrase similarity does not activate the adapter.

ADR-kit `forbid_import.pattern` is a source-text regular expression, not an import specifier. Archgate companion `.rules.ts` files are arbitrary checks. Neither is automatically equivalent to a graph prohibition. Keep those original checks; an orchestration adapter can run archgraph alongside them. Archgate's restricted rule runtime should consume an externally generated report rather than invoke an archgraph subprocess from a rule.

Format references: [ADR-kit](https://github.com/rvdbreemen/adr-kit), [ADR-kit Enforcement schema](https://github.com/rvdbreemen/adr-kit/blob/9a0b9b4c22509f27e43d056b8b0ecf44a519403a/schemas/adr-enforcement.schema.json), [Archgate ADR schema](https://github.com/archgate/cli/blob/6075eafb55fc2425537a52690d2d2486e452811d/docs/src/content/docs/reference/adr-schema.mdx).

### Measured compatibility gaps

The three authored formats each have six fixtures and a complete JSON golden report. One additional fixture copies the pinned Archgate ARCH-018 decision verbatim and has a three-rule golden plus live import-kind regression. They demonstrate supported translations and deliberately unsupported cases; they are not representative coverage claims.

Pinned upstream corpus measurements on 2026-10-19:

| Corpus | Commit | Markdown documents | Mapped documents | Enforceable proposals | 50% target |
| --- | --- | ---: | ---: | ---: | --- |
| adr-kit | `9a0b9b4c2250` | 51 | 0 | 0 | **unmet** |
| archgate | `6075eafb55fc` | 35 | 1 | 3 | **unmet** |

These counts include all Markdown files in the pinned ADR directories, including index/administrative documents. Archgate has 26 architecture ADRs (`ARCH-*.md`): one is partially mapped (1/26, 3.85%; 1/35, 2.86% across all documents). ADR-kit has zero mapped documents, including when excluding its index. Scope suggestions are not counted as enforcement. No paid agent or model was used. Reproduce with `python3 ci/test_adr_interop.py --binary target/debug/archgraph`; this clones the exact pins into a temporary directory. Before and after import, the verifier compares corpus inventory and bytes with the pinned Git tree, including ignored/untracked Markdown and any root layout.yaml; matching HEAD alone is insufficient. `--checkouts DIR` uses existing checkouts named `adr-kit` and `archgate`. The JSON result exposes `acceptance_met: false`; a successful script exit proves the recorded inventory, not the unmet product target. The full machine-readable gap inventory is in `ci/adr-corpora.json`.

The remaining documents need a maintained, reviewed mapping or additional graph predicates. The importer currently does not meet Task D's ≥50% upstream mapping acceptance criterion.

| Corpus | Source | Gaps |
| --- | --- | --- |
| adr-kit | `docs/adr/ADR-001-llm-gates-opt-in.md` | inactive lifecycle |
| adr-kit | `docs/adr/ADR-002-adr-guardian-session-start-staleness-detector.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-003-template-profile-compatibility.md` | inactive lifecycle |
| adr-kit | `docs/adr/ADR-004-layered-adr-context-injection.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-005-selectable-agent-friendly-adr-formats.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-006-prepare-platform-local-marketplaces-for-native-installs.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-007-json-adr-graph-index-for-agent-retrieval.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-008-resolve-the-enforcement-engine-from-a-version-ranked-root-set-including-the-checkout.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-009-bound-heuristic-gates-to-findings-an-author-can-act-on.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-010-certify-three-native-cli-clients-through-one-outcome-contract.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-011-adopt-deterministic-readiness-and-human-gated-grilling-across-the-adr-lifecycle.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-012-release-to-the-three-coding-agent-marketplaces-from-the-public-repository.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-013-declare-version-sites-in-one-registry-and-bump-by-writing.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-014-use-the-generated-adr-graph-as-the-selective-context-query-engine.md` | inactive lifecycle |
| adr-kit | `docs/adr/ADR-015-enforce-a-two-second-deterministic-latency-budget-as-a-test-fixture-contract.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-016-serve-both-mcp-protocol-eras-from-one-hand-rolled-stdio-server.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-017-run-the-llm-judge-by-default-on-the-host-agent-model.md` | inactive lifecycle |
| adr-kit | `docs/adr/ADR-018-add-a-local-precomputed-vector-layer-for-adr-retrieval.md` | inactive lifecycle |
| adr-kit | `docs/adr/ADR-019-keep-the-end-of-session-hooks-deliberately-silent.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-020-embed-the-query-where-the-query-is-asked-and-read-authority-from-the-index.md` | inactive lifecycle |
| adr-kit | `docs/adr/ADR-021-let-the-session-scoped-hooks-regenerate-a-stale-adr-index.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-022-make-open-questions-append-only-for-a-proposed-adr.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-023-record-the-pull-request-guard-as-a-fail-closed-tier.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-024-ask-for-a-missing-adr-at-the-pull-request-moment-inside-the-guard.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-025-separate-what-tracked-configuration-may-select-from-what-only-a-machine-may-introduce.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-026-record-the-combined-audit-command-and-its-five-way-exit-contract.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-027-derive-the-lifecycle-signer-from-a-person-named-git-identity-announced.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-028-gate-the-declared-related-link-and-leave-the-prose-one-alone.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-029-retire-the-native-hook-binary-rather-than-maintain-a-second-retrieval-engine.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-030-recalibrate-the-hook-latency-budgets-to-the-python-host-that-actually-ships.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-031-name-the-pull-request-moment-as-a-deliberately-slower-user-initiated-event.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-032-treat-a-generated-client-tree-as-a-first-class-doctor-context.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-033-name-the-user-initiated-whole-repository-commands-as-deliberately-slower.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-034-declare-the-hook-network-property-per-event-rather-than-for-the-hook-set.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-035-run-the-suggestion-pass-by-default-on-the-same-terms-as-the-judge.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-036-retire-the-vector-layer-and-run-the-judge-on-the-host-model-only.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-037-keep-per-adr-judge-verdicts-in-the-advisory-per-machine-guardian-state.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-038-cost-an-unusable-llm-verdict-to-that-adr-alone-not-the-whole-pass.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-039-add-a-native-opencode-plugin-without-expanding-the-certified-cli-gate.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-040-grow-the-mcp-tool-surface-only-with-read-only-deterministic-cycle-tools.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-041-automatically-hand-off-unfinished-proposed-adrs-to-interactive-grilling.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-042-drive-the-release-from-the-maintainer-s-machine-and-create-the-tag-from-the-merge.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-043-forbid-a-pinned-model-or-a-default-llm-command-in-every-model-calling-entry-point.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-044-resolve-the-interpreter-and-the-engine-through-one-resolver-at-every-entry-point.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-045-let-the-judge-acquire-its-own-diff-and-give-every-diff-shape-an-explicit-verdict.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-046-match-every-path-glob-in-the-judge-s-dialect-wherever-it-is-read.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-047-cap-supporting-adrs-per-response-and-let-them-fill-only-the-slots-left-under-the-limit.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-048-certify-each-native-client-on-every-platform-it-runs-on.md` | scope only, no supported import constraint |
| adr-kit | `docs/adr/ADR-049-record-the-judge-host-client-at-the-first-session-and-ask-per-llm-layer-at-upgrade.md` | scope only, source regex, no supported import constraint |
| adr-kit | `docs/adr/ADR-INDEX.md` | unknown lifecycle, no supported import constraint |
| adr-kit | `docs/adr/README.md` | unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-001-command-structure.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-002-error-handling.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-003-output-formatting.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-004-no-barrel-files.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-005-testing-standards.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-006-dependency-policy.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-007-cross-platform-subprocess-execution.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-008-typed-command-options.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-009-platform-detection-helper.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-010-prefer-bun-built-in-json-parsing.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-011-consistent-project-root-resolution.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-012-command-error-boundaries.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-013-version-synchronization.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-014-prefer-bun-env.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-015-cli-command-documentation-coverage.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-016-cli-subcommand-documentation-accuracy.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-017-multi-ecosystem-distribution.md` | companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-018-lazy-load-heavy-dependencies.md` | partial runtime package mapping; future dependencies need review, companion TypeScript, unknown lifecycle |
| archgate | `.archgate/adrs/ARCH-019-inquirer-prompt-fix.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-020-glob-scan-include-dotfiles.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-021-authored-text-integrity.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-022-ast-aware-rule-context.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-023-engine-file-listing-via-in-memory-git-tracked-matching.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-024-rule-file-sandbox-boundary.md` | scope only, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-025-idiomatic-bun-test-parametrization-and-matchers.md` | scope only, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/ARCH-026-strict-mode-and-check-severity-tiers.md` | scope only, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/CI-001-pin-github-actions-by-hash.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/CI-002-validate-workflow-syntax-with-actionlint.md` | unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/GEN-001-documentation-site.md` | unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/GEN-002-docs-i18n.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/GEN-003-tool-invocation-via-scripts.md` | companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/GEN-004-concise-forward-only-code-comments.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/GEN-005-repository-root-contents-allowlist.md` | companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/LEGAL-001-spdx-license-headers.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
| archgate | `.archgate/adrs/LEGAL-002-dependency-license-compatibility.md` | scope only, companion TypeScript, unknown lifecycle, no supported import constraint |
