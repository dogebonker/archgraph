# Per-edit loop: scripted acceptance and latency

The edit gate compares current code with the graph captured immediately before an edit. A new forbidden import, cycle, declared surface bypass, or change outside the supplied scope blocks progression with a recipe. Existing findings remain advisory. A block retains its original baseline through repair attempts; capturing another repair does not turn the violation into pre-existing debt.

Install with `archgraph hook install --agent claude --root . --scope 'src/**'` (or `--agent codex`) and start `archgraph serve . --engine native --symbols` with matching scan options/environment. Installer output names the files it wrote and the supported capabilities. Existing settings and foreign hooks are retained; repeated installation produces identical contents. For shell tools without known paths, pre-edit context states the transaction contract; actual workspace changes determine the post verdict. Known read-only tools skip transactions.

Installation selects an existing layout. A first layout introduced later is adopted at the next clean Pre event; introducing it mid-edit makes the pending transaction unavailable. Changing an explicit rule path still requires reinstall. Generated/dependency directories are outside inventory. Explicit edits there are refused, while opaque shell writes there are not scope-enforced. Snapshot artifacts currently accumulate under `.archgraph/hooks/`; there is no automatic retention policy.

Contained directory symlinks are recorded as link identities; snapshots do not traverse alias trees, and changes to the physical target files are inventoried at their ordinary repository paths. Retargeting the link is also a change. Explicit edit paths through directory aliases are refused: edit the physical path so scope and generated-output checks apply to it. External, broken, unresolved cyclic and non-regular symlink targets remain unavailable; links back to an ancestor directory are refused. Dependency/build-output exclusions still apply to physical paths. The original Pydantic benchmark failures from directory symlinks remain in the report; this subsequent compatibility fix does not rescore them.

| Host | Project configuration | Supported contract and limits |
| --- | --- | --- |
| Claude Code | `.claude/settings.json` | Pre context/deny, successful and failed post checks, Stop continuation gate. Failure blocking requires v2.1.295+; Stop crash/timeout handling and continuation limits remain host constraints. |
| Codex | `.codex/hooks.json` | Pre context/deny, Post/Stop block responses. Review and explicitly trust the installed hooks through `/hooks`; the installer never changes trust. |
| Cursor | `.cursor/hooks.json` | Requires `--feedback-only`: pre denial, post feedback and a bounded stop follow-up; allowed pre-tool context injection is not established. |
| OpenCode | `.opencode/plugins/archgraph.ts` | Requires `--feedback-only`: before-tool denial, after-tool feedback and idle logging; no verified completion veto or pre-tool context API. |

These contracts follow the official [Claude hook reference](https://code.claude.com/docs/en/hooks), [Codex hook reference](https://learn.chatgpt.com/docs/hooks), [Cursor hook reference](https://cursor.com/docs/hooks) and [OpenCode plugin reference](https://opencode.ai/docs/plugins/). Protocol regressions use subprocess payloads. One live Codex session is validated below; live Claude Code, Cursor and OpenCode sessions have not been run. Context returned at PreToolUse is consumed on the next model request and does not replan an already-generated tool call. Blocking after a write does not roll it back. Concurrent edits are refused; Stop can validate and retire a same-session transaction left by cancellation or a denied permission. Exact `hook reset --baseline ID --reason reason` is audited recovery when the host does not reach Stop.

A live Codex 0.163.0 session with `gpt-6.1-sol` on 2026-10-19 validated the native loop on a private TypeScript fixture. After exact installed-hook review and hash trust in an isolated profile, PostToolUse blocked a temporary `domain → adapters` import and returned a semantic `extract_interface` recipe. The agent repaired the code with `apply_patch`; its repair Pre event retained the blocked edit’s snapshot, the next Post returned PASS, Stop completed, and a full graph comparison plus the corrected source assertion passed. One rule fire was recorded; no automatic recipe application was claimed. The layout and frozen binary remained unchanged, and the temporary auth link was removed before evaluation. An initial preflight rejected the default `codex_apps` MCP transport before any feature turn; disabling Apps preserved the empty-MCP requirement. The successful retry used the only feature turn. The [safe live summary](../../bench/results/codex-live-loop-2026-10-19.json) retains model/version/pins, token counters, hook verdicts, that preflight failure and a local assertion quoting error followed by its corrected pass. Raw prompts, RPC and auth paths remain private. This smoke verifies one host’s protocol; it is not a paired efficacy result or a latency measurement.

## Run the scripted demo

With the current-main binary installed as shown in the README, clone the repository outside your project to obtain the demo script. From an existing source checkout, skip the clone and `cd` steps:

```sh
git clone https://github.com/dogebonker/archgraph
cd archgraph
python3 ci/bench_edit_loop.py --archgraph "$(command -v archgraph)" --fixture-only
```

The script creates its own temporary fixture. No project initialization, separately started daemon or live agent is needed for this scripted block/repair check. A binary installation alone does not contain the repository's `ci/` scripts.

To also reproduce the release-binary 2,000-file latency benchmark from that checkout:

```sh
cargo +stable build --release --locked
python3 ci/bench_edit_loop.py --archgraph target/release/archgraph --repeats 5
```

Use `--fixture-only` to run just the small acceptance session. The script prints the artifact directory's results as JSON and writes `results.json`, `transcript.json`, and the daemon log there. An explicit `--work /path/to/new-directory` makes the artifact location predictable. It does not overwrite existing fixtures.

The session uses CLI hook payloads and source edits, not a live Claude, Codex, Cursor, or OpenCode process. It captures a clean domain file, adds an import from `domain` to `adapters`, checks for `BLOCK` with a recipe, captures an unchanged repair attempt and confirms it still blocks against the original baseline, removes the import, and checks for `PASS`. It also checks that an unchanged pre-existing violation passes and that `fires --top` counts the two blocks.

Equivalent core commands, with the snapshot ID returned by `capture`:

```sh
archgraph hook capture --root . --agent claude --session demo --tool-id edit-1 \
  --files src/domain/a.ts --rules "$PWD/layout.yaml" --scope 'src/**'
# Add: import "../adapters/b";
archgraph hook check --root . --baseline SNAPSHOT_ID --brief
# Remove the forbidden import, then recheck the same baseline.
archgraph hook check --root . --baseline SNAPSHOT_ID --brief
archgraph fires --top --root .
```

`--brief` prints one verdict line followed by recipe JSON. `BLOCK` exits 1; `PASS` exits 0. Native agent hooks wrap that information in each host's documented JSON protocol. A post-edit block leaves the source edit on disk so the agent can repair it.

An actual debug-binary session on 2026-10-19 produced the following selected fields from the block report; the full recipe is retained in `transcript.json`:

```json
{
  "verdict": "BLOCK",
  "violations": [{
    "file": "src/domain/a.ts",
    "kind": "forbidden-edge",
    "rule": "domain-no-adapters",
    "recipe": {"steps": [{"kind": "cut_import", "mechanical": true}]}
  }]
}
```

After removing the import, the same baseline returned:

```text
PASS
{"schema":"archgraph/edit-check/v1","verdict":"PASS","violations":[]}
```

`fires --top` returned `{"rules":[{"count":2,"rule":"domain-no-adapters"}],"total":2}`. The second fire came from deliberately checking an unchanged repair attempt. The separate pre-existing-violation check passed.

The latency fixture reuses `ci/bench_daemon_2k.py`'s deterministic Git-backed chain: 2,000 TypeScript files, 100 folders, and 1,999 initial edges. Each measured check follows a newly added forbidden direct import from `src/m000/f00.ts` to `src/m050/f00.ts`. Cold samples force `--no-daemon`; warm samples start a native daemon with symbols and the same rules. Timings include the entire `hook check` process, source freshness, graph comparison, recipe output, and audit append. Snapshot capture, source-write time, and the subsequent successful repair are excluded. The JSON reports every sample, median, nearest-rank p95, maximum, machine details, fixture commit, and binary SHA-256. Untimed daemon status records before each edit and after each check establish that the daemon stayed alive and advanced its graph generation.

Measured on 2026-10-19 with the release build on macOS arm64, five independent edit/check/repair samples per path and mode, after validation jobs finished:

| Path | Mode | Median | p95 / maximum |
| --- | --- | ---: | ---: |
| Complete `hook check` | Warm daemon | 200.4 ms | 208.9 ms |
| Complete `hook check` | Cold standalone | 492.2 ms | 505.8 ms |
| Snapshot `check_diff` through `mcp-call` | Warm daemon | 264.5 ms | 279.7 ms |
| Snapshot `check_diff` through `mcp-call` | Cold standalone | 484.5 ms | 495.9 ms |

Every measured sample met the 300 ms warm / 3,000 ms cold targets. With five samples, nearest-rank p95 equals the maximum. See [raw results, all samples, host, binary hash and fixture commit](loop-results.json). The explicit MCP measurements include the CLI bridge and complete result serialization; stdio envelope processing, model time and host scheduling are excluded. Read-only MCP calls do not append fires. These measurements concern the pre-edit snapshot path; Git-baseline checks still scan a historical checkout. They establish this fixture's performance, not a latency guarantee for every language, graph or audit-log size.

An earlier development run exceeded the warm target (588 ms maximum); the figures above use the final runtime on a quiet machine. Architecture enforcement does not depend on hitting the latency target: collection/configuration errors remain unavailable rather than clean. The scripted acceptance reports two blocked checks, a successful repair, an unchanged pre-existing violation passing, and two fire records. Live Claude Code, Cursor and OpenCode sessions remain validation gaps; the Codex smoke above does not replace a paired agent benchmark.
