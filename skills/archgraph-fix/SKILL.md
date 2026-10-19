---
name: archgraph-fix
description: Fixes one archgraph finding end to end, keeping the change only when a re-scan and the tests prove it. Use when asked to fix, apply or act on an archgraph finding, break an import cycle, move a misplaced file, or route an import through a module's public entry.
license: MIT OR Apache-2.0
metadata:
  compatibility: Needs git, the archgraph binary (setup in the archgraph skill), and the project's own type checker and test command.
---

# archgraph-fix

## Input-quality interpretation

Check the saved source root, grouping depth and collector limitations before comparing SCC counts. Native Rust excludes definite test-only references and keeps containment structural; unknown cfg, macro expansion and type resolution remain incomplete. Its cycles measure source coupling, not initialization failure. A shared state container is a hub; do not attribute an entire component to it without a counterfactual. Rust/Java/Elixir semantic cuts stay non-mechanical. For Python, empty initializers do not establish a facade, and unresolved installed dependencies need actual package metadata. Explicit layout policies remain authoritative. Restart a running daemon and rescan after a collector upgrade.

## MCP tools

Prefer MCP `check_diff`/`verify` to obtain current findings, then `propose({finding_id})` for ordered evidence-backed steps. Use `simulate({moves:[{from,to}]})` before moving a file; it does not edit imports or move code. After applying the approved change, call `verify` with the proposal's complete `verify.structured` predicate, repeat `check_diff` against the same base, and run project tests. Use explicit `expect` predicates: `verify` without them only confirms that inspection succeeded. An absent old finding alone does not prove no new violation. Inspect truncation markers and fall back to the complete CLI workflow below when needed or when MCP is unavailable. MCP does not relax the scope and approval rules below.

One run fixes one finding, as one reviewable change. Every step ends on evidence; the change survives only if the last step is green.

## Steps

1. **Pick one actionable finding.** Scan with the intended rules, symbols, engine and exclusions, then select the requested `findings[].finding_id`. Otherwise choose a relevant ranked finding and state its evidence. Every actionable v1.2 finding carries `recipe`; advisory `diagnostics[]` retain unsupported or weak-signal evidence and a reason. Read those diagnostics too, especially `small-repo`. Default ranking is hotspot × confidence × pair reduction, not proof of priority; use `--rank legacy` for the previous hotspot-first order.
2. **Check the baseline.** Inspect both `git diff` and `git diff --cached`; preserve existing changes. Discover and run the project's build/type check and tests. Record pre-existing failures rather than claiming a green baseline. Work only within the user's authorized change and leave unrelated cleanup and policy edits out.
3. **Get a verification contract before editing.** Prefer MCP `propose({finding_id})`; otherwise run `archgraph propose <finding_id> --root <repo> <matching scan flags>`. Keep the complete returned `verify.structured`, including `baseline_id` and any `expected` deltas. Live proposals save the before-scan locally. Unknown/null predictions are unavailable, not zero. Steps marked non-mechanical, including interface extraction and splits, are instructions for the agent; graph evidence does not implement them.
4. **Inspect and apply.** Run `archgraph apply <finding_id> --root <repo> <matching scan flags> --dry-run`. Review `can_apply`, `pending`, exact source buffers and destinations. For a fully mechanical plan within the task, run the same command with `--yes`. It rescans, verifies and rolls back on failure; `--yes` cannot bypass pending agent work. For a semantic plan, follow the instructions manually, preserving public bindings and behavior. Do not edit `layout.yaml` to make a code fix pass. TS/JS and Python import edits preserve every byte outside the captured specifier; Go package changes and Rust/Java declaration changes require manual work. Ambiguous, escaped, wildcard or live-binding cuts are refused rather than guessed.
5. **Verify the exact contract and regressions.** After mechanical application, run `archgraph verify <finding_id> --root <repo> <matching scan flags>` against its receipt. After manual work, call MCP `verify({expect: <saved verify.structured>})` or CLI `verify <finding_id> --expect '<saved JSON>'`. Use the original proposal baseline; do not regenerate it after editing to erase a mismatch. A predicted delta must match actual evidence. Then repeat `check_diff` against the same base (or `archgraph diff <before> <after> --fail-on new-forbidden,new-cycles,new-unplanned`) and run the build/type check and tests. MCP verification without `expect` only confirms inspection; omitted findings are not absent findings.
6. **Leave a reviewable result.** Successful mechanical moves are staged; rewritten import contents remain worktree edits. Inspect both diffs before staging the complete fix. The CLI never commits. Report actual deltas, predicate results and language test results. On failure, preserve concurrent/pre-existing edits and revert only the changes this run owns; never restore whole files from HEAD over another change. If the apply command reports a rollback conflict, inspect the named files and retain the user's edits.

See [recipe semantics and limits](https://github.com/dogebonker/archgraph/blob/main/docs/guide.md#fix-recipes-and-verification). One run fixes one finding; rescan before starting another because moves interact.

## Scope

- One finding per run. When the user asks for several, repeat the whole loop per finding and re-scan between them: moves interact.
- Code changes stay within the plan from step 3. Leave unrelated cleanup, renames and formatting out of the change.
- `layout.yaml` is policy: a fix never edits it. If the right fix is a rule change, say so and stop.
