---
name: archgraph
description: Checks declared architecture rules against the import graph of a TypeScript/JavaScript, Python, Go, Rust, Java or Elixir repository. Use to initialize reasoned policies without architecture docs, install per-edit agent checks, synchronize agent rule files, inspect fix recipes, or review boundaries, cycles and file moves with graph evidence. Also supports budgeted context, ADR rule proposals with provenance, and before/after architecture comparisons.
license: MIT OR Apache-2.0
metadata:
  compatibility: Needs the archgraph binary on PATH (install.sh, npx archgraph-cli, or cargo install). TypeScript/JavaScript scans need dependency-cruiser 18.4.0 and typescript 6.0.3 installed in the scanned repository (run through npx), or installed elsewhere and passed with --depcruise. Python scans need a Python with grimp 3.17. Go scans need Go 1.21 or later (tested with 1.27.1). Elixir scans need Elixir 1.14+ / OTP 25+ (`--elixir <path>`); no Mix build or dependency installation runs.
---

# archgraph

## Input-quality interpretation

Folder loops are dependency observations, not proof of circular file imports or defects. Normal app → router → dependency wiring must remain intact. Implicit cycle gates compare newly cyclic concrete file imports (physical compiler packages for Go), not display-folder SCCs. Inferred facade bypasses remain diagnostic until a public boundary is declared. Never cut an import merely to improve a graph metric; preserve callers and behavior.

Check the saved source root, grouping depth and collector limitations before comparing SCC counts. Native Rust excludes definite test-only references and keeps containment structural; unknown cfg, macro expansion and type resolution remain incomplete. Its cycles measure source coupling, not initialization failure. A shared state container is a hub; do not attribute an entire component to it without a counterfactual. Rust/Java/Elixir semantic cuts stay non-mechanical. For Python, empty initializers do not establish a facade, and unresolved installed dependencies need actual package metadata. Explicit layout policies remain authoritative. Restart a running daemon and rescan after a collector upgrade.

## MCP tools

When the client's archgraph MCP tools are available, prefer them to spawning CLI commands: `context` before reading, `check_diff` for regressions, `propose` for a stable finding ID, `simulate` before moving, `verify` after editing, and `explain`/`rules` for local evidence. Tool names may carry a client/server prefix. All accept `brief` (default true); inspect truncation markers, narrow requests when needed, and use the CLI for complete evidence. `brief: false` retains the MCP hard cap. MCP does not change code or replace project tests. Follow the CLI workflows below when the tools are unavailable. See [MCP setup and semantics](https://github.com/dogebonker/archgraph/blob/main/docs/guide.md#mcp-server).

archgraph reads a repository's file imports, groups files into modules (folders, or rules), and reports metrics, cycles, rule verdicts and findings as JSON (`schema: archgraph/v1.2`; v1/v1.1 inputs remain supported). Every claim you make about the architecture must come from its output. The experimental `--engine native` supports all listed languages without external parsers; native Go still requires read-only `go list` metadata. Reference collectors remain the default.

## Repeated agent calls

Prefer `--brief` when consuming answers in a context window. Structured output is a separate `archgraph/brief/v1` wrapper containing `result`, full top-level array `counts`, explicit `omitted` records and a hint to request full evidence. The cap is 20 array entries across the result, 240 nodes, depth 8 and 512 characters per string. Do not feed the projection back as a scan document; graph output written with `-o` stays complete. If relevant evidence was omitted, rerun without `--brief` before making a claim.

For repeated edits, `archgraph serve <root> --engine native --symbols` keeps a watched graph in `<root>/.archgraph/daemon-graph.json`. It writes local daemon metadata and graph snapshots there; start it only within the requested workspace. Use the same collection flags for cached scans. CLI commands discover the daemon automatically; `--no-daemon` forces a standalone run. Existing saved graphs retain their own snapshot semantics, so pass the daemon graph explicitly for current edits:

```sh
archgraph status <root> --brief
archgraph check <root>/.archgraph/daemon-graph.json --brief
archgraph explain src/billing/invoice.ts --graph <root>/.archgraph/daemon-graph.json --brief
```

Native TS/JS, Python and Go incrementally reparse changed files; global resolution and connected analysis regions may still require repository-wide work. Other engines/languages log full-refresh fallbacks. No sub-100 ms latency guarantee is established by the feature itself. A refresh error requires fixing the source/configuration and retrying; do not cite the last saved daemon graph as current evidence after an error. Stop a daemon you launched when the task no longer needs it. `serve --once` warms and answers one request without watching, then exits.

Scan timestamps identify source time (`SOURCE_DATE_EPOCH`, Git HEAD time, then epoch), not the current clock. Check graph provenance and current source rather than deciding freshness from `generated_at` alone. Keep full v1/v1.1/v1.2 scan evidence for comparisons.

## Setup

For a repository without a layout, `archgraph init --preset layered --scope internal` creates a reasoned draft from a preset plus observed imports and prints the first findings. Other presets are hexagonal, feature-sliced, modular-monolith and minimal; other scopes are demo, production and regulated. It preserves existing layouts unless explicitly forced. Review conflicts, then use `archgraph check <root>` for a fresh native check. Saved graph checks cannot evaluate mandatory live size limits and report INCOMPLETE/exit 2. Follow [archgraph-rules](../archgraph-rules/SKILL.md) to calibrate the policy.

1. Run `archgraph --version` and check the required command's `--help`. For the current init, edit-hook, sync-rules and ADR workflows, build current main with `cargo +stable install --git https://github.com/dogebonker/archgraph --locked`. Released binaries and `npx archgraph-cli` can lag main; verify capabilities before relying on them. The [install guide](https://github.com/dogebonker/archgraph/blob/main/docs/guide.md#install) covers prebuilt downloads and checksums.
2. TypeScript/JavaScript: dependency-cruiser 18.4.0 parses TypeScript with `typescript@6.0.3` installed beside it, both as dev dependencies of the scanned repository by default. If `scan` fails with an install hint, ask the user before running `npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3`, since it changes their lockfile. Installing both outside the repository (`npm install --prefix <dir> dependency-cruiser@18.4.0 typescript@6.0.3`) and passing `--depcruise <dir>/node_modules/.bin/depcruise` leaves the repository untouched.
3. Python: find or create a Python with `grimp==3.17` (`pip install grimp==3.17` in a venv, or `uv run --with grimp==3.17 python`) and pass it with `--python <interpreter>`.
4. Go: `go version` must print 1.21 or later (1.27.1 is tested); otherwise ask the user to install Go from https://go.dev/dl, or pass `--go <path>` to a Go elsewhere. Nothing is installed into the repository and no module downloads are needed. If the scan fails because the repository's `go.mod` asks for a newer Go, say so: archgraph never downloads a toolchain.
5. Elixir: use Elixir 1.14+ / OTP 25+ on `PATH`, or pass `--elixir <path>`. `--language elixir` selects it explicitly in mixed repositories. Scans parse `.ex`/`.exs` across the scan root, including umbrella apps, without loading Mix projects, evaluating source, compiling or expanding macros.

Write archgraph output to a temporary directory outside the repository unless the user asks to keep it. `report` writes into `<root>/.archgraph/` without `--out-dir`; pass `--out-dir <tmp>` for reports. `serve` maintains its local endpoint and live graph in `<root>/.archgraph/`.

**Scan only repositories you trust.** A TypeScript/JavaScript scan runs `npx --no-install depcruise` inside the repository, and npx resolves the repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`, so the repository decides what runs (`--depcruise <path>` runs that executable instead). For a repository you did not write or do not trust, tell the user and scan it inside a sandbox or container. (Python scans run the interpreter with `-I` and keep the repository off `sys.path` until grimp is loaded; Go scans compile only archgraph's own helper, outside the repository, and run `go list` with no network, no toolchain switch and no `go.work`; git runs with `core.fsmonitor=false`.)

## Workflow

For a quick overview a person reads, `archgraph report <root> --out-dir <tmp>` runs steps 1, 3 and 5 at once: it scans (same options as `scan`), applies `--rules` or `<root>/layout.yaml` and says which, writes `archgraph.json` and `archgraph.html`, and prints counts, forbidden imports by rule, module cycles and findings by kind. Exit 1 means forbidden imports exist. For evidence you quote, read the JSON it wrote, as below.

1. **Scan.** `archgraph scan <root> -o <tmp>/graph.json`. Add `--rules layout.yaml` when the repository has one, `--cochange` to read the last 300 commits for files that change together, `--depth N` when modules sit deeper than two folders below `src/`, `--package <pkg>` only when a Python root holds several packages (a single package is found on its own). A warning about unresolved imports means some edges are missing: report it. When the root has `src/`, only `src/` is cruised: say so if the user expects other folders covered. Type-only imports and re-exports (`type`, `type-reexport`) and dynamic imports count as edges in metrics and verdicts, not in cycles or `unstable-dependency`. In Python, an import only under `if TYPE_CHECKING:` is `type`, one only inside function bodies `dynamic`; in TypeScript an import of types or interfaces only, even without `import type`, is `type`. `build/` and `dist/` below the root are scanned unless git ignores them. Each file directly in the source root is a module of its own, named with its extension (`main.ts`, `errors.py`). Go: modules group packages by folder like other languages (`--depth 99` makes each package a module; a cycle between merged packages is a grouping cycle, not one Go compiled), the root's own files form the module `.`, every `go.mod` below the root is scanned, edges join files that use what another file declares (same-package ones included), test support counts as tests (packages named `*testing` or `*testutil`, and non-test files using `testing`; the warning names them), and a warning that `go list` failed in a module means it was read with `go/build`.
2. **Read the summary.** Look at `findings` and advisory `diagnostics` first. If a `small-repo` diagnostic exists, say the signal is weak before anything else. Then read `summary` (forbidden, unplanned and cycle counts, per-module metrics), `cycles`, `modules` and `module_edges`.
3. **Check rules.** With rules: `archgraph check <tmp>/graph.json [--rules layout.yaml]` (exit 2 when the scan has no rules and none are passed). Exit 1 means forbidden imports exist; each line names the deciding rule. `unplanned` imports are ones no rule covers: either the rules are incomplete or the import is unexpected. Ask which.
4. **Simulate before proposing.** For every move you want to propose, run `archgraph simulate <tmp>/graph.json --move <file> <dest-path>` and read `before`, `after`, `new_forbidden` and `removed_forbidden`. Findings that suggest a move already carry a `simulation`: `simulation.after` holds the counts after the move and only the modules whose metrics change (`null` for a module the move empties); the numbers before are the scan's `summary`.
5. **Render for the user.** `archgraph render <tmp>/graph.json -o <tmp>/graph.html` writes one self-contained page. Give the user its path. Colors are a reading aid, not a score.
6. **Compare over time.** After changes, scan again and run `archgraph diff old.json new.json` to show resolved and new cycles, new and resolved forbidden imports, new unplanned imports and metric pairs `[old, new]`. In CI, `--fail-on new-forbidden,new-cycles,new-unplanned` (any subset) exits 1 when the diff has one, and `--format markdown` prints a pull-request summary instead of JSON. A cycle is new when no cycle of the old scan has its member set, so a cycle that grew counts as new.

## Executable recipes

Use a current `findings[].finding_id` with MCP `propose` or `archgraph propose <finding_id> --root <repo> <matching scan flags>`. Every actionable finding has ordered recipe steps, explicit confidence/risk notes and `verify.structured`; observations without a concrete edit live in `diagnostics[]` with their reasons. IDs exclude ranking, timestamps and deltas. Rank defaults to hotspot × confidence weight × nonnegative pairs removed (high/medium/low: 1.0/0.6/0.3; absent hotspot: 1; unknown reduction: 0); `--rank legacy` restores hotspot-first ordering.

Call `propose` before editing to save a baseline. Inspect `archgraph apply <finding_id> <matching flags> --dry-run`; only fully mechanical plans can use `--yes`. Otherwise follow agent instructions and pass the saved `recipe.verify.structured` to MCP `verify({expect: ...})` or CLI `verify <finding_id> --expect '<JSON>'`. Mechanical application can be checked with CLI `verify <finding_id>` using its receipt. Unknown predictions are unavailable, not zero. Successful `git mv` changes are staged while rewritten import contents remain worktree edits; inspect both diffs. Builds/tests and a base comparison remain required. See [fix workflow](../archgraph-fix/SKILL.md) and [language limits](https://github.com/dogebonker/archgraph/blob/main/docs/guide.md#fix-recipes-and-verification).

## Reading findings

Each finding has `kind`, `subject`, `evidence`, and optionally `suggestion.move` plus `simulation`.

- `cycle`: modules (or files inside one module) that import each other at runtime (type-only imports and re-exports and dynamic imports are left out). `evidence.break` lists the fewest file imports whose removal breaks the cycle. Propose removing or inverting those imports, not others. With `exact: false` the break set comes from a heuristic and can be large: do not propose all of it at once. Start with `evidence.two_way` (module cycles): pairs of modules that import each other, the direction to cut first with its file imports; when the cycle lists `registrar_moves`, propose its move to `to` first if `pairs_removed` is above 1 and `cycle_after` is below `cycle_before` (quote both), else after the single-import cut; skip pairs flagged `resolved_by_move` (the registrar move resolves them), `even` (both sides tiny and close: a mutual dependency, no lighter side), `registry` (only composition roots and registrars wire them) or `entry` (public re-exports) (`edges`, at most 20; `imports` counts all). Propose removing or inverting the first pair's lighter side, re-scan, and repeat while the cycle remains. When `two_way` is empty (a ring such as a -> b -> c -> a), start with the module edge in the cycle that has the fewest imports (`module_edges[].imports`).
- `misplaced-file`: a file that `imports_from_target` files of `target_module` import (at least 2), more than twice its `own_module_edges` (in + out within its own module), and that no third module imports. Never between two folders named by kind (`repositories`, `services`, `components`, `menus`, ...), nor between a module and its own subfolder, nor for a leaf utility (imports nothing, in `utils`, `helpers`, `lib`, `shared`, `common` or `storage`); the move must raise the target module's cohesion by at least 0.02. What the file itself imports never counts: a file is pulled by its importers, not by what it uses. archgraph reports it only when the move adds no forbidden import or cycle, gives the target no new dependency and raises the two modules' combined cohesion; quote the simulation. The suggested destination keeps the folder the file sits in within its module (`chains/domain/hd.py` into signer is `signer/domain/hd.py`) when a file of the target module sits in that folder, else the target's root. A finding without `suggestion` means a file of the same name already exists at that destination, or the destination would fall in another module: pick a new name there, run `archgraph simulate … --move <file> <target dir>/<new name>`, and propose it only if that simulation passes the same checks.
- `sink-split`: a widely imported module whose files serve separate groups of importers (`components`), at least two of them with 2 or more files, never a folder named by kind (`utils`, `services`, `errors`), grouped by runtime imports without composition roots, registrars, the sink's parent folder or files that only import its entry; never reported when the groups import each other. Moves leave public API (package entries and what they re-export; Python modules without a `_` segment) in place. Propose splitting along the components; single-consumer components come with moves.
- `internals-reach`: one finding per module whose internal files, ones its entries (barrels, package.json entries) re-export, other modules import instead of the entry (`entries`), per bound `public` module with rules (subject: its pattern): `imports` counts the bypassing imports, `importers` and `importing_files` their modules and files, `edges` lists up to 20. Propose importing the entry. Without rules a barrel counts as the entry only when at least half the imports into its module use it (`barrel_share`). Imports from a subfolder of the target's module are not reported.
- `unstable-dependency`: a stable module depends on a less stable one (through barrels, as Ca and Ce count), with at least 2 runtime file imports (`imports`), never between a module and its own subfolder, nor into a private module (`_internal`) of the importer's own package. Propose an abstraction owned by the stable module, or moving the depended-on code.
- `seam`: label propagation found file clusters, holding at least half of the files, that fit the imports better than the folders (`modularity` vs `folder_modularity`), not reported when more than half the files sit in single-file modules or more than half the folder modules are named by kind (`controllers`, `services`, ...); `hubs` are the most connected files, left out. Present the clusters as candidate boundaries, not as a plan.
- `hidden-coupling`: two files in different modules changed together in `commits` commits without importing each other. Evidence only: ask the user what links them (events, DI, shared data).
- `small-repo`: too few files or modules for the other findings to mean much.
- `via_barrel: true` in evidence means an index/`__init__` re-export was counted against every re-exported file: the real coupling may be smaller.

The list above describes evidence kinds retained from earlier schemas; observations without a concrete recipe are now advisory `diagnostics`, rather than actionable `findings`. Ignore kinds you do not recognize, while retaining their evidence in full exports.

A scan document is input to `check`, `simulate`, `render` and `diff`: do not edit its `files` or `edges` by hand. One whose edges name files it does not list, or whose modules disagree with its config, is rejected with exit 2.

## Proposing moves

- Propose a move only with its simulation evidence. Quote the numbers: forbidden and unplanned counts, cycle count, and the affected modules' cohesion, Ca, Ce and instability before and after.
- Reject a move whose simulation adds forbidden imports or cycles, unless the user accepts that cost explicitly.
- Prefer few moves with clear gains over many small ones. Run the simulation for each move separately; moves interact, so re-scan after applying any.
- Do not move files yourself unless the user approves the specific move.

For onboarding, `archgraph tour graph.json` prints a dependency-first reading order; `--reverse` starts from entrypoints and `--agent` adds saved export names without opening bodies. Read cyclic stops together and follow their suggested starting module. Missing or partial export evidence is labeled.

For a declared service inventory, `archgraph services services.yaml` reports HTTP/gRPC/queue cycles, hubs and policy violations with confidence and source evidence. `--format json` emits a standalone services report. Treat declarations as unverified and incomplete; a cycle is not proof of deadlock. Service reports are separate from import scans and cannot be fed to import `check` or `render`.

## Acting on the evidence

For agent efficacy claims, use the [paired benchmark harness](https://github.com/dogebonker/archgraph/tree/main/bench): matching models, three runs per arm, native hook activation evidence, and an independent feature oracle. Retain failures and zero-benefit results. A scripted block/repair demo proves the loop works; it does not establish that agents produce better changes. The public synthetic cases use isolated feature checks, not full upstream suites.

The [controlled boundary challenge](https://github.com/dogebonker/archgraph/tree/main/bench/usefulness) tests adapter direction, facade access and cycles against full-package business checks, with a neutral control. Count every planned attempt in the denominator and require both working behavior and zero new violations. Report unknown activation and time/token cost; authored challenges cannot establish broad real-world usefulness.

Related skills take the next step:

- [archgraph-context](../archgraph-context/SKILL.md): before editing, prepares a token-budgeted reading plan from a saved scan, with constraints first and export summaries for large or distant files.
- [archgraph-guard](../archgraph-guard/SKILL.md): before a coding task is reported done, compares the change with its base commit and resolves new forbidden imports and cycles.
- [archgraph-fix](../archgraph-fix/SKILL.md): fixes one finding end to end (simulate, apply, test, re-scan, keep or revert).
- [archgraph-rules](../archgraph-rules/SKILL.md): writes and calibrates a `layout.yaml` from an approved architecture and wires the CI gate.

## Limits

Tell the user these when they matter:

- `scan` runs tooling the scanned repository controls; scan untrusted repositories only in a sandbox.
- With a `src/` directory only `src/` is cruised.
- Type-only imports and re-exports and dynamic imports count in metrics and verdicts, so layouts that forbid type imports across boundaries still catch them; they do not count in cycles.
- Imports are not semantics: DI containers, event buses, string-based lookups, reflection and dynamic imports without static strings are invisible. `--cochange` partly covers this.
- Small repositories have little signal; the `small-repo` diagnostic says so.
- Findings are moves with evidence. There is no single score to optimize; module colors in `render` are a reading aid.
- Module boundaries come from folders (`--depth`) or rules `modules:`; wrong boundaries give wrong metrics.
- Named barrel imports resolve to defining files; ambiguous namespace/star imports retain low-confidence barrel evidence.
- Rust collection needs rust-analyzer; Java needs a JDK 11 or newer. Elixir needs Elixir 1.14+ / OTP 25+; aliases, static calls, structs and type references are collected, but macro-generated/quoted dependencies and dynamic dispatch remain incomplete. Unknown modules are unresolved; external identities are OTP application names proven by installed `.app` module-ownership metadata, not guessed Hex names. Elixir symbol surfaces are unavailable. See the guide for collector flags and limits.
- Go: a use reached only through the types of a dependency outside the repository is missing, every platform's files are merged, and reflection, `go:linkname` and `go:embed` are invisible. Files that cannot leave their package and still compile (`source.collector.bound_files`: methods of a type in another file, uses of another file's unexported names) are never proposed for a move.
