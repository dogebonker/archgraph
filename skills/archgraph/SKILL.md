---
name: archgraph
description: Measures a TypeScript/JavaScript or Python repository's import graph with the archgraph CLI and turns it into architecture evidence - module coupling and cohesion, cycles with the fewest imports to break them, layer and boundary rule verdicts, and file moves backed by a before/after simulation. Use when asked to review or improve a codebase's architecture or module boundaries, find import cycles, check imports against layer rules, decide where a file belongs, render a dependency graph, or compare architecture between two commits.
license: MIT OR Apache-2.0
compatibility: Needs the archgraph binary on PATH (install.sh, npx archgraph-cli, or cargo install). TypeScript/JavaScript scans need dependency-cruiser 18.4.0 and typescript 6.0.3 installed in the scanned repository (run through npx), or installed elsewhere and passed with --depcruise. Python scans need a Python with grimp 3.17.
---

# archgraph

archgraph reads a repository's file imports, groups files into modules (folders, or rules), and reports metrics, cycles, rule verdicts and findings as JSON (`schema: archgraph/v1`). Every claim you make about the architecture must come from its output.

## Setup

1. Run `archgraph --version`. If it is missing, install the prebuilt binary: `curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/dogebonker/archgraph/main/install.sh | sh` (checks the release's SHA-256, installs to `~/.local/bin`), or run it through npm as `npx archgraph-cli`. Without a prebuilt binary for the platform, `cargo install --git https://github.com/dogebonker/archgraph --locked` (needs a Rust toolchain).
2. TypeScript/JavaScript: dependency-cruiser 18.4.0 parses TypeScript with `typescript@6.0.3` installed beside it, both as dev dependencies of the scanned repository by default. If `scan` fails with an install hint, ask the user before running `npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3`, since it changes their lockfile. Installing both outside the repository (`npm install --prefix <dir> dependency-cruiser@18.4.0 typescript@6.0.3`) and passing `--depcruise <dir>/node_modules/.bin/depcruise` leaves the repository untouched.
3. Python: find or create a Python with `grimp==3.17` (`pip install grimp==3.17` in a venv, or `uv run --with grimp==3.17 python`) and pass it with `--python <interpreter>`.

Write archgraph output to a temporary directory outside the repository unless the user asks to keep it. archgraph writes into the scanned repository only when `report` runs without `--out-dir` (into `<root>/.archgraph/`), so always pass `--out-dir <tmp>`.

**Scan only repositories you trust.** A TypeScript/JavaScript scan runs `npx --no-install depcruise` inside the repository, and npx resolves the repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`, so the repository decides what runs (`--depcruise <path>` runs that executable instead). For a repository you did not write or do not trust, tell the user and scan it inside a sandbox or container. (Python scans run the interpreter with `-I` and keep the repository off `sys.path` until grimp is loaded; git runs with `core.fsmonitor=false`.)

## Workflow

For a quick overview a person reads, `archgraph report <root> --out-dir <tmp>` runs steps 1, 3 and 5 at once: it scans (same options as `scan`), applies `--rules` or `<root>/layout.yaml` and says which, writes `archgraph.json` and `archgraph.html`, and prints counts, forbidden imports by rule, module cycles and findings by kind. Exit 1 means forbidden imports exist. For evidence you quote, read the JSON it wrote, as below.

1. **Scan.** `archgraph scan <root> -o <tmp>/graph.json`. Add `--rules layout.yaml` when the repository has one, `--cochange` to read the last 300 commits for files that change together, `--depth N` when modules sit deeper than two folders below `src/`, `--package <pkg>` only when a Python root holds several packages (a single package is found on its own). A warning about unresolved imports means some edges are missing: report it. When the root has `src/`, only `src/` is cruised: say so if the user expects other folders covered. Type-only imports count as edges in metrics, cycles and verdicts. Each file directly in the source root is a module of its own, named with its extension (`main.ts`, `errors.py`).
2. **Read the summary.** Look at `findings` first. If a `small-repo` finding exists, say the signal is weak before anything else. Then read `summary` (forbidden, unplanned and cycle counts, per-module metrics), `cycles`, `modules` and `module_edges`.
3. **Check rules.** With rules: `archgraph check <tmp>/graph.json [--rules layout.yaml]` (exit 2 when the scan has no rules and none are passed). Exit 1 means forbidden imports exist; each line names the deciding rule. `unplanned` imports are ones no rule covers: either the rules are incomplete or the import is unexpected. Ask which.
4. **Simulate before proposing.** For every move you want to propose, run `archgraph simulate <tmp>/graph.json --move <file> <dest-path>` and read `before`, `after`, `new_forbidden` and `removed_forbidden`. Findings that suggest a move already carry a `simulation`: `simulation.after` holds the counts after the move and only the modules whose metrics change (`null` for a module the move empties); the numbers before are the scan's `summary`.
5. **Render for the user.** `archgraph render <tmp>/graph.json -o <tmp>/graph.html` writes one self-contained page. Give the user its path. Colors are a reading aid, not a score.
6. **Compare over time.** After changes, scan again and run `archgraph diff old.json new.json` to show resolved and new cycles, new and resolved forbidden imports, new unplanned imports and metric pairs `[old, new]`. In CI, `--fail-on new-forbidden,new-cycles,new-unplanned` (any subset) exits 1 when the diff has one, and `--format markdown` prints a pull-request summary instead of JSON. A cycle is new when no cycle of the old scan has its member set, so a cycle that grew counts as new.

## Reading findings

Each finding has `kind`, `subject`, `evidence`, and optionally `suggestion.move` plus `simulation`.

- `cycle`: modules (or files inside one module) that import each other. `evidence.break` lists the fewest file imports whose removal breaks the cycle. Propose removing or inverting those imports, not others. With `exact: false` the break set comes from a heuristic and can be large: do not propose all of it at once. Start with `evidence.two_way` (module cycles): pairs of modules that import each other, lighter direction first with its file imports (`edges`, at most 20; `imports` counts all). Propose removing or inverting the first pair's lighter side, re-scan, and repeat while the cycle remains. When `two_way` is empty (a ring such as a -> b -> c -> a), start with the module edge in the cycle that has the fewest imports (`module_edges[].imports`).
- `misplaced-file`: a file that `imports_from_target` files of `target_module` import (at least 2), more than its `own_module_edges` (in + out within its own module), and that no third module imports. What the file itself imports never counts: a file is pulled by its importers, not by what it uses. archgraph reports it only when the move adds no forbidden import or cycle, gives the target no new dependency and raises the two modules' combined cohesion; quote the simulation. The suggested destination keeps the folder the file sits in within its module (`chains/domain/hd.py` into signer is `signer/domain/hd.py`) when a file of the target module sits in that folder, else the target's root. A finding without `suggestion` means a file of the same name already exists at that destination, or the destination would fall in another module: pick a new name there, run `archgraph simulate … --move <file> <target dir>/<new name>`, and propose it only if that simulation passes the same checks.
- `sink-split`: a widely imported module whose files serve separate groups of importers (`components`). Propose splitting along the components; single-consumer components come with moves.
- `internals-reach`: an import of a module's internal file instead of its public entry (`entries`). Propose importing the entry. Without rules a barrel counts as the entry only when at least half the imports into its module use it (`barrel_share`).
- `unstable-dependency`: a stable module depends on a less stable one (through barrels, as Ca and Ce count). Propose an abstraction owned by the stable module, or moving the depended-on code.
- `seam`: label propagation found file clusters, holding at least half of the files, that fit the imports better than the folders (`modularity` vs `folder_modularity`); `hubs` are the most connected files, left out. Present the clusters as candidate boundaries, not as a plan.
- `hidden-coupling`: two files in different modules changed together in `commits` commits without importing each other. Evidence only: ask the user what links them (events, DI, shared data).
- `small-repo`: too few files or modules for the other findings to mean much.
- `via_barrel: true` in evidence means an index/`__init__` re-export was counted against every re-exported file: the real coupling may be smaller.

Ignore finding kinds you do not recognize.

A scan document is input to `check`, `simulate`, `render` and `diff`: do not edit its `files` or `edges` by hand. One whose edges name files it does not list, or whose modules disagree with its config, is rejected with exit 2.

## Proposing moves

- Propose a move only with its simulation evidence. Quote the numbers: forbidden and unplanned counts, cycle count, and the affected modules' cohesion, Ca, Ce and instability before and after.
- Reject a move whose simulation adds forbidden imports or cycles, unless the user accepts that cost explicitly.
- Prefer few moves with clear gains over many small ones. Run the simulation for each move separately; moves interact, so re-scan after applying any.
- Do not move files yourself unless the user approves the specific move.

## Acting on the evidence

Three skills next to this one take the next step:

- [archgraph-guard](../archgraph-guard/SKILL.md): before a coding task is reported done, compares the change with its base commit and resolves new forbidden imports and cycles.
- [archgraph-fix](../archgraph-fix/SKILL.md): fixes one finding end to end (simulate, apply, test, re-scan, keep or revert).
- [archgraph-rules](../archgraph-rules/SKILL.md): writes and calibrates a `layout.yaml` from an approved architecture and wires the CI gate.

## Limits

Tell the user these when they matter:

- `scan` runs tooling the scanned repository controls; scan untrusted repositories only in a sandbox.
- With a `src/` directory only `src/` is cruised.
- Type-only imports count in metrics, cycles and verdicts (Factory's FDA layout keeps them).
- Imports are not semantics: DI containers, event buses, string-based lookups, reflection and dynamic imports without static strings are invisible. `--cochange` partly covers this.
- Small repositories have little signal; the `small-repo` finding says so.
- Findings are moves with evidence. There is no single score to optimize; module colors in `render` are a reading aid.
- Module boundaries come from folders (`--depth`) or rules `modules:`; wrong boundaries give wrong metrics.
- Barrel attribution over-approximates: a consumer of an index is counted against every re-exported file.
- Only TypeScript/JavaScript and Python are supported; Go and Rust are not yet.
