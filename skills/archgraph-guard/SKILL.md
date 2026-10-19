---
name: archgraph-guard
description: Guards a code change against architecture regressions with archgraph, comparing the change with its base commit and resolving the forbidden imports and import cycles it introduced. Use when finishing a code change in a repository that has a layout.yaml or uses archgraph, or when asked whether a change breaks architecture rules or adds cycles.
license: MIT OR Apache-2.0
metadata:
  compatibility: Needs git, bash and the archgraph binary (setup in the archgraph skill). Works in any agent harness; the optional Stop hook is Claude Code specific.
---

# archgraph-guard

## Input-quality interpretation

Folder loops are dependency observations, not proof of circular file imports or defects. Normal app → router → dependency wiring must remain intact. Implicit cycle gates compare newly cyclic concrete file imports (physical compiler packages for Go), not display-folder SCCs. Inferred facade bypasses remain diagnostic until a public boundary is declared. Never cut an import merely to improve a graph metric; preserve callers and behavior.

Check the saved source root, grouping depth and collector limitations before comparing SCC counts. Native Rust excludes definite test-only references and keeps containment structural; unknown cfg, macro expansion and type resolution remain incomplete. Its cycles measure source coupling, not initialization failure. A shared state container is a hub; do not attribute an entire component to it without a counterfactual. Rust/Java/Elixir semantic cuts stay non-mechanical. For Python, empty initializers do not establish a facade, and unresolved installed dependencies need actual package metadata. Explicit layout policies remain authoritative. Restart a running daemon and rescan after a collector upgrade.

## MCP tools

Prefer MCP `check_diff({base})` when available; omit `base` for guard-style merge-base selection, or provide the intended base explicitly. It compares current edits under current rules and discloses policy changes. Use returned stable finding IDs with `propose` before editing. Keep its complete `verify.structured` contract and baseline, fix code, then call `verify({expect: <saved structured predicate>})`, repeat `check_diff` and run project tests. Unknown predictions remain unavailable; advisory diagnostics are not automatically fixable findings. For CLI mechanical edits, inspect `apply <finding_id> --dry-run` before `--yes`; pending agent work cannot be overridden. `verify` needs explicit `expect` predicates to assert a result; without them, success only confirms inspection. It does not replace the base comparison. Inspect truncation markers: a partial list is not a pass. Use the CLI guard below for its complete evidence, configurable failure policy and exit codes, or when MCP is unavailable.

A change passes the guard when `scripts/guard.sh` exits 0: compared with its base commit, the working tree adds no forbidden import and no concrete cyclic file/package import. New unplanned imports are listed too; `ARCHGRAPH_FAIL_ON=new-forbidden,new-cycles,new-unplanned` makes them fail, matching a CI gate set that way. Treat a red guard like a failing test: the task is not done.

## Warm graphs and concise evidence

`archgraph sync-rules --check` verifies the generated blocks match layout.yaml; it exits 1 for missing/stale blocks and writes nothing. Regenerate with `sync-rules` after a reviewed policy change. Custom instructions belong outside its owned fences; hand-edited blocks refuse replacement and show a diff. Stale-rule warnings from live `check` are separate from its architecture verdict. Synchronization does not install or trust native hooks.

For per-edit enforcement, run `archgraph hook install --agent claude|codex --root <root>` within the authorized workspace; add repeatable `--scope 'src/**'` as needed. Hooks capture pre-edit evidence, inject at most 400 UTF-8 bytes, and return new-only verdicts with recipes after edits and at Stop. Repair against the same baseline; a new capture cannot forgive a pending block. Known read-only tools skip transactions. Start `archgraph serve <root> --engine native --symbols` with matching flags/environment for warm scans. Codex requires host hook trust; Claude failure blocking needs v2.1.295+. Cursor/OpenCode require explicit `--feedback-only` and cannot provide equivalent completion gating. Post hooks leave source edits on disk. See [the loop and host limits](https://github.com/dogebonker/archgraph/blob/main/docs/agents/loop.md).

Use `hook capture --agent codex --files src/example.ts` for scripted transactions and `hook check --baseline ID --brief` or MCP `check_diff({snapshot: ID})` for their checks. `snapshot` and Git `base` are exclusive. MCP is read-only: inspect its truncation counts, then use the complete CLI check for missing recipes. CLI verdicts are followed by recipe JSON; exit 0 passes, 1 blocks, 2 is unavailable. Keep a pending baseline through repairs and run project tests. `ARCHGRAPH_BYPASS=reason`, `--allow-once ID --reason reason`, and exact `hook reset --baseline ID --reason reason` are audited escape hatches, not fixes. `fires --top` lists block counts. Snapshot inventory omits dependencies/build output; opaque shell writes there are outside scope checking. Preserve local `.archgraph/` state until the transaction is resolved.

For contained directory symlinks, snapshots record link identity and physical files without traversing aliases. Use physical paths for explicit edits; directory-alias paths are refused so scope/generated-output checks cannot be bypassed. Link retargets and physical target changes are detected. External, broken, unresolved cyclic or non-regular targets remain unavailable, as do directory links back to an ancestor. Keep the baseline through repair; do not bypass or reset merely to hide an unavailable link.

A daemon started with `archgraph serve <root> --engine native` can reuse matching working-tree scan options; `--no-daemon` forces standalone collection. Pass `--engine native` in `ARCHGRAPH_SCAN_ARGS` when using that daemon. The guard still scans its separate base worktree and compares complete scan documents; a warm working-tree graph does not eliminate the base scan. Reference and unsupported incremental paths use logged full-refresh fallbacks. The guard script keeps its existing output and exit semantics.

For a quick policy answer between guard runs, use `archgraph check <root>/.archgraph/daemon-graph.json --brief`. It reports cycles and violations in bounded `archgraph/brief/v1` JSON with explicit omissions. Policy exit 1 means forbidden imports, not merely a cycle; the regression guard remains necessary to fail on newly introduced cycles. Never use a brief projection as the graph input to `diff`. Scan `generated_at` is reproducible source time, not a measure of snapshot freshness; check provenance/current edits and daemon refresh status instead.

## Steps

1. **Set up.** archgraph must run; follow Setup in [the archgraph skill](../archgraph/SKILL.md#setup). Find the scan flags the repository needs: run `archgraph scan <root> -o <tmp>/probe.json` with no flags and add what its error names: `--depcruise <path>` when dependency-cruiser is installed outside the repository (one in the repository's `node_modules` is found on its own), `--python <interpreter>` for Python, `--package <pkg>` when a Python root holds several packages, `--depth N` when modules sit deeper than two folders. Pass them as `ARCHGRAPH_SCAN_ARGS`. Done when the probe scan exits 0.
2. **Run the guard** from the repository:
   ```bash
   ARCHGRAPH_SCAN_ARGS="<flags>" bash <this skill>/scripts/guard.sh
   ```
   It scans the working tree (uncommitted edits included) and the base commit (merge-base with `origin/HEAD`, else `main` or `master`; override with `ARCHGRAPH_BASE`) with the same rules (`<project>/layout.yaml`, or `ARCHGRAPH_RULES`) and prints notes (base equal to HEAD, a rules file added, edited or deleted by the change), a header line (`archgraph guard: N files scanned against <base commit> with rules <file>`) and the archgraph diff. With no change against the base it says so and exits 0 without scanning. Set `ARCHGRAPH_PATH` when the project sits in a subfolder. Exit 0 means pass, 1 means new violations, 2 means the guard could not run: fix the cause and rerun. Done when you have exit 0 or 1 and its summary.
3. **Resolve each new violation the change introduced.** Remove each listed import the way its rule intends, choosing the code change as step 3 ("Cut") of [archgraph-fix](../archgraph-fix/SKILL.md) orders the options. For a new concrete file/package cycle, inspect its import paths and redirect responsibilities while preserving behavior; a lighter folder direction alone is not a repair. A listed `unplanned` import (rule `none`) is one the rules do not cover: ask the user whether it is intended.
   
   Edit code, not `layout.yaml`: the rules are the team's policy, and changing them to pass the guard needs the user's explicit approval. Tell the user about any note the guard printed. Done when every listed import is fixed or put to the user with its evidence.
4. **Re-run the guard** after the fixes, then the project's tests. Done when the guard exits 0 and the tests pass (say so when the repository has no tests), or when you stop and report the violations you could not resolve, each with its import, rule and the fix you propose.
5. **Report** the guard's header line and its verdict, or the remaining violations.

## Stop hook (Claude Code, optional)

To run the guard whenever the agent tries to finish, add to `.claude/settings.json`, with the user's approval:

```json
{
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command",
      "command": "ARCHGRAPH_SCAN_ARGS='<flags>' bash \"$CLAUDE_PROJECT_DIR\"/.claude/skills/archgraph-guard/scripts/guard.sh --hook",
      "timeout": 300 }] }]
  }
}
```

Point the path at wherever the skill folder is installed. With `--hook`, new violations go to stderr with exit 2, so the agent continues and resolves them; when the agent already continued once for that stop, the hook lets it stop; a guard that cannot run reports on stderr with exit 1, which does not block. A turn without changes against the base costs nothing; otherwise a run takes two scans (seconds on most repositories, about 20 seconds on 3,000 TypeScript files).
