# Paired agent benchmark

This standard-library Python harness clones a pinned clean Git
commit for every task, arm and repetition; runs an explicit model through an argv runner;
installs hooks and generated rules only in the guarded arm; measures final architecture
regressions, edit blocks, apply receipts, tests, usage, duration and diff size; preserves
all results, including failures, and renders Markdown plus SVG. The full sequence is
tested with a mock architecture CLI and mock agent. The [first real results](results/README.md)
retain all 24 public synthetic runs. No performance claims come from mocks.

Run `python3 -B bench/run.py --repo /path/to/repo --tasks tasks.json --runner runner.json
--boundrail /absolute/path/to/boundrail --output bench/results`. Three repetitions are the
default and minimum for a published comparison. Use `--repeats 1` only for a smoke test.
Use `--package NAME` for a monorepo requiring explicit package selection (for example
`--package pydantic`). The same selection reaches hook installation, final graph comparison
and receipt verification, and is recorded in report JSON/Markdown.
The source must have a clean tracked/untracked worktree. Each run uses a private local
clone with no hard links; the source worktree and index are never changed. Setup and
private diff staging respect Git ignore rules and exclude `.boundrail` with an index-only
restore, including repositories that ignore that directory. Temporary
clones are deleted; command stdout/stderr and measurements remain in the raw report.

Tasks JSON: `[{"id":"feature", "prompt":"Implement ...", "area":["src/domain/**"],
"tests":["cargo","test","--locked"]}]`. Area is an expected touched-area annotation, not an enforcement scope. Changes outside it
are recorded; empty diffs or no change inside the expected area mark feature work unverified
and exclude the run from efficacy means. Passing project tests alone does not prove the task was implemented.
Runner JSON: `{"agent":"claude", "model":"EXACT_MODEL", "adapter":"claude"}` or
`{"agent":"codex", "model":"EXACT_MODEL", "adapter":"codex"}`. Built-in adapters
invoke `claude -p PROMPT --model MODEL --output-format json` and
`codex exec --model MODEL --sandbox workspace-write --json PROMPT`. Tools must be on PATH. For other runners use
`"argv":["wrapper","--model","{model}","--prompt","{prompt}","--seed","{seed}"]`.
Arguments are substituted individually and never interpreted by a shell. `agent` still
selects the hook adapter. Custom argv must contain `{model}` and `{prompt}`. Optional
`seed` is passed only when argv contains `{seed}`; unsupported seeds are explicitly
recorded. Optional `env` adds variables only to the actual agent process (values are never
serialized to results). Setup, evaluators and project tests do not receive these overrides
or inherited environment names containing TOKEN, SECRET, PASSWORD, API_KEY or ACCESS_KEY.
Known runner credential values and the auth-file path are redacted from retained command
output/arguments and metadata. Reports record auth_file_supplied without publishing the path.
This is environment separation, not an OS security sandbox for mutually hostile processes.

Agent configuration is isolated by fresh HOME, CODEX_HOME, CLAUDE_CONFIG_DIR and
XDG_CONFIG_HOME per run. Use a wrapper or explicit runner env to authenticate; any override
of those isolation paths is disclosed as global hook contamination risk in the report.
Do not bypass native hook trust checks: trust the exact generated Codex hook configuration
through a supported runner profile. A failed trust/auth/setup run remains a failed run.
Successful installation and synthetic fires do not prove host activation. The reviewed
`codex-app-server` adapter below requires correlated native notifications; other guarded
runners remain activation-unknown and excluded from efficacy means.
Configuration paths are checked before cleanup; symlink parents/targets are rejected before any mutation.
Both arms remove repository hook configs, every root/nested project `.codex` directory
(including config.toml, MCP declarations, skills and execution policies), and previously
generated boundrail rule blocks before setup; original files remain in the source
repo. Removed file paths and SHA-256 hashes are recorded without config values. Guarded setup requires `boundrail sync-rules` and fails visibly if it is unavailable.
A local setup commit separates installation changes from the reported agent diff. Tests and final checks always run after an
agent failure. Commands have timeouts (POSIX process groups are stopped together; Windows only stops the direct child); errors do not silently become zero violations.

Architecture metric: full live `mcp-call --tool check_diff --arguments '{"base":"COMMIT"}'`
against the original commit, counting new forbidden edges plus distinct new/grown cycles (growth is not counted twice).
The reviewed pre-agent layout.yaml identity/hash is recorded. If the agent changes,
adds or removes that policy, the run has unknown violations for scoring and is excluded
from efficacy means; raw current-policy diagnostic results remain visible.
The raw response preserves findings and policy notes. These counts are graph regressions,
not a claim that every possible smell is enforced. Fires count JSONL block events (including
repeated blocks); distinct finding IDs are also reported. Raw receipt JSON filenames/counts
are diagnostic only. Applied recipe counts use `boundrail verify <finding_id>` without an
explicit predicate: the Rust reader validates the complete Receipt, restores its baseline,
checks fingerprints and checks the recipe against current sources. No receipts means zero;
any malformed, stale or failed receipt means unknown. Manual fixes are not counted.
Tokens remain null unless a recognized runner response supplies usage, with its JSON
location recorded. Wall time covers the agent command only; setup/check/test times are
in command records. Diff size is captured before project tests to exclude test side effects; it includes
binary/untracked files and counts changed lines
as Git numstat additions/deletions where available (binary entries reported separately).

Reports include every run, per-arm mean and sample standard deviation both pooled and per task, and an SVG chart
of final regressions. Failed or missing measurements are shown as unknown, never dropped
from the raw report or silently averaged as zero. Aggregate means use comparison-eligible runs only;
setup failures, policy mutations, unverified feature work and unknown guarded activation
stay in the table rather than producing false improvements. A mock run proves plumbing only.

Verify: `python3 -B -m unittest discover -s bench -p 'test_*.py'`.
Perf's two repository/task sets remain unavailable. The first reports use two showcase
repos and two explicitly labeled public substitutes, matching authenticated models and
reviewed host activation evidence. These benign tasks did not establish an efficacy gain.

## Reviewed Codex app-server runner (0.163.0, POSIX)

Runner JSON: `{"agent":"codex","adapter":"codex-app-server","model":"EXACT_MODEL",
"trust_reviewed_boundrail":true,"auth_file":"/operator/reviewed/private/auth.json"}`.
`codex` optionally selects an explicit Codex executable. The adapter checks
`codex --version` equals `codex-cli 0.163.0`; its protocol was read from upstream commit
`83086188ae0614bd882818ff1aa23f3194962012` and the generated 0.163.0 schemas. No model
fallback is allowed. Both arms use workspace-write, approval policy never, ephemeral
threads, matching model and isolated profiles. Built-in seeds are unsupported and recorded.

The auth-file option is explicit operator authorization to link a private owned regular
file into the fresh private CODEX_HOME. The runner never reads/parses/copies its contents.
Codex may refresh the source auth file through that symlink as part of ordinary login
maintenance. Review that path before a real run. No global Codex config is changed.
After app-server exits, a finally block removes only the runner-created auth.json link or
its private owned SDK replacement in the pinned private profile. This runs on setup/turn
failures too. The harness skips evaluators/tests if any auth.json remains; evaluation never
reuses the live linked auth artifact. The source auth file is never unlinked by the runner.

The adapter starts `codex app-server --disable apps` in both arms to exclude the default
builtin codex_apps MCP transport, then initializes app-server and enables only the stable `features.hooks` flag in both
isolated profiles, and trusts only the private clone via `config/batchWrite`
with explicit isolated config.toml `filePath`, and inventories hooks. Guarded mode reviews
exactly three boundrail commands, their source path, matcher, key and current hash. Only
with `trust_reviewed_boundrail:true` does it upsert those exact `hooks.state` trusted hashes.
It relists and verifies enabled/trusted handlers with unchanged hashes before starting a
turn. Standalone use rejects remaining project Codex config/exec policies before linking
auth or granting trust. The effective thread response must retain the requested model/cwd,
OpenAI provider, workspaceWrite sandbox with no extra writable roots/network access, and
never approval policy; MCP inventory must be empty before turn/start. It then verifies
unchanged inventory/config/binary again afterwards. Unexpected hooks,
commands, trust changes or interactive server requests fail closed; no blanket bypass exists.

A guarded claim needs paired hook/started and hook/completed notifications for this exact
thread/turn/source, completed Pre/Post/Stop coverage, a pre-edit snapshot marker, a Post
PASS/BLOCK verdict and a completed Stop. Missing proof stays unknown; failed/stopped or
mis-correlated runs are invalid. Bare mode requires an empty inventory and no hook events.
The harness independently checks the runner schema, unique run ID, mode/model/canonical
clone, reviewed binary/config digests, inventory digest and correlated safe events before
using activation in an efficacy comparison. Custom runners cannot grant themselves activation.

Full RPC events and app-server stderr remain in 0600 files under
`OUTPUT/.private-events/<run>/` (0700 directories); auth/account RPC methods are not logged.
Only safe hook identity/status/snapshot/verdict fields and token totals enter reports. These
private artifacts may contain prompts/source and must not be published with result tables.
They are ignored by bench/.gitignore when results are written under bench/.
The offline fixture emulates the protocol; it is not evidence of real agent effectiveness.

Protocol checks use generated v2/ThreadStartResponse.json and v2/ListMcpServerStatus{Params,Response}.json;
project config/exec policy scope is removed as a complete `.codex` tree, without interpreting config values.
This does not claim the upstream provider sanitization permits repository provider redirects.

## Live guarded protocol smoke

On 2026-10-19 one real `gpt-6.1-sol` feature turn through Codex 0.163.0 completed the
controlled TypeScript domain/adapters loop: native Post BLOCK with a semantic recipe,
repair against the same snapshot, Post PASS, Stop completed, then a clean full graph check
and corrected exact-source assertion after auth cleanup. Seven hook runs and one rule fire
were observed; the repair was manual and produced no apply receipt. The initial preflight
failure (default Apps MCP), subsequent `--disable apps` fix, and a separate local assertion
quoting failure/corrected pass are retained in the
[safe summary](results/codex-live-loop-2026-10-19.json). Binary SHA-256, fixture/Codex pins,
model, token counters and verdicts are public there; prompts, RPC and auth paths are omitted.
The smoke used one feature turn and supports no bare-versus-guarded efficacy claim.
