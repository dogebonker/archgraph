---
name: archgraph
description: Measures a TypeScript/JavaScript or Python repository's import graph with the archgraph CLI and turns it into architecture evidence - module coupling and cohesion, cycles with the fewest imports to break them, layer and boundary rule verdicts, and file moves backed by a before/after simulation. Use when asked to review or improve a codebase's architecture or module boundaries, find or break import cycles, check imports against layer rules, decide where a file belongs, write or enforce a layout.yaml, render a dependency graph, or compare architecture between two commits.
license: MIT OR Apache-2.0
compatibility: Needs the archgraph binary on PATH. TypeScript/JavaScript scans need dependency-cruiser 18.4.0 and typescript 6.0.3 installed in the scanned repository (run through npx). Python scans need a Python with grimp 3.17.
---

# archgraph

archgraph reads a repository's file imports, groups files into modules (folders, or rules), and reports metrics, cycles, rule verdicts and findings as JSON (`schema: archgraph/v1`). Every claim you make about the architecture must come from its output.

## Setup

1. Run `archgraph --version`. If it is missing, install it: `cargo install --git https://github.com/OWNER/archgraph --locked` (needs a Rust toolchain).
2. TypeScript/JavaScript: the scanned repository needs `dependency-cruiser@18.4.0` and `typescript@6.0.3` as dev dependencies. If `scan` fails with an install hint, ask the user before running `npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3`, since it changes their lockfile.
3. Python: find or create a Python with `grimp==3.17` (`pip install grimp==3.17` in a venv, or `uv run --with grimp==3.17 python`) and pass it with `--python <interpreter>`.

Write archgraph output to a temporary directory outside the repository unless the user asks to keep it. archgraph itself never writes into the scanned repository.

**Scan only repositories you trust.** A TypeScript/JavaScript scan runs `npx --no-install depcruise` inside the repository, and npx resolves the repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`, so the repository decides what runs. For a repository you did not write or do not trust, tell the user and scan it inside a sandbox or container. (Python scans run the interpreter with `-I` and keep the repository off `sys.path` until grimp is loaded; git runs with `core.fsmonitor=false`.)

## Workflow

1. **Scan.** `archgraph scan <root> -o <tmp>/graph.json`. Add `--rules layout.yaml` when the repository has one, `--cochange` to read the last 300 commits for files that change together, `--depth N` when modules sit deeper than two folders below `src/`, `--package <pkg>` when a Python root holds several packages. A warning about unresolved imports means some edges are missing: report it. When the root has `src/`, only `src/` is cruised: say so if the user expects other folders covered. Type-only imports count as edges in metrics, cycles and verdicts. Each file directly in the source root is a module of its own, named with its extension (`main.ts`, `errors.py`).
2. **Read the summary.** Look at `findings` first. If a `small-repo` finding exists, say the signal is weak before anything else. Then read `summary` (forbidden, unplanned and cycle counts, per-module metrics), `cycles`, `modules` and `module_edges`.
3. **Check rules.** With rules: `archgraph check <tmp>/graph.json [--rules layout.yaml]`. Exit 1 means forbidden imports exist; each line names the deciding rule. `unplanned` imports are ones no rule covers: either the rules are incomplete or the import is unexpected. Ask which.
4. **Simulate before proposing.** For every move you want to propose, run `archgraph simulate <tmp>/graph.json --move <file> <dest-path>` and read `before`, `after`, `new_forbidden` and `removed_forbidden`. Findings that suggest a move already carry a `simulation`: `simulation.after` holds the counts after the move and only the modules whose metrics change (`null` for a module the move empties); the numbers before are the scan's `summary`.
5. **Render for the user.** `archgraph render <tmp>/graph.json -o <tmp>/graph.html` writes one self-contained page. Give the user its path. Colors are a reading aid, not a score.
6. **Compare over time.** After changes, scan again and run `archgraph diff old.json new.json` to show resolved and new cycles, forbidden imports and metric pairs `[old, new]`.

## Reading findings

Each finding has `kind`, `subject`, `evidence`, and optionally `suggestion.move` plus `simulation`.

- `cycle`: modules (or files inside one module) that import each other. `evidence.break` lists the fewest file imports whose removal breaks the cycle. Propose removing or inverting those imports, not others. With `exact: false` the break set comes from a heuristic and can be large: do not propose all of it at once. Start with the imports behind the module edges with the fewest imports (count them in `module_edges[].imports`), remove or invert one, re-scan, and repeat while the cycle remains.
- `misplaced-file`: a file that `imports_from_target` files of `target_module` import (at least 2), more than its `own_module_edges` (in + out within its own module), and that no third module imports. What the file itself imports never counts: a file is pulled by its importers, not by what it uses. archgraph reports it only when the move adds no forbidden import or cycle, gives the target no new dependency and raises the two modules' combined cohesion; quote the simulation. The suggested destination keeps the folder the file sits in within its module (`chains/domain/hd.py` into signer is `signer/domain/hd.py`) when a file of the target module sits in that folder, else the target's root. A finding without `suggestion` means a file of the same name already exists at that destination, or the destination would fall in another module: pick a new name there, run `archgraph simulate … --move <file> <target dir>/<new name>`, and propose it only if that simulation passes the same checks.
- `sink-split`: a widely imported module whose files serve separate groups of importers (`components`). Propose splitting along the components; single-consumer components come with moves.
- `internals-reach`: an import of a module's internal file instead of its public entry (`entries`). Propose importing the entry.
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

## Writing layout.yaml from an approved layout

When the user approves a target layout, write it as rules so `check` can enforce it:

```yaml
version: 1
modules:                      # only when folders do not match the intended modules
  - { id: "domains/<name>", path: "src/domains/<name>/**" }
layers:                       # lowest first; a file may import its own or a lower layer
  - "src/shared/**"
  - "src/domains/*/model.server.ts"
  - "src/domains/*/ui/**"
public:                       # module pattern and its public entries
  - { module: "src/domains/<name>/**", entries: ["src/domains/<name>/index.ts"] }
forbid:
  - { id: shared-no-domain, from: "src/shared/**", to: "src/domains/**", reason: "shared imports no domain" }
  - { id: browser-no-server, from: "src/domains/*/client.ts", to: "**/*.server.ts", reachable: true }
allow:
  - { from: "app/**", to: "**", reason: "composition" }
default: forbidden            # optional: only when the user wants a whitelist
```

- `*` matches one path segment, `**` any depth, `{a,b}` alternatives. `<name>` matches one segment and binds it within the rule: in `forbid` and `allow` the target must have the same value as the importer, so `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }` means "within one module". `layers` take no capture: where a layout writes `<pkg>` or `<domain>` in a layer, write `*`. Unknown keys are an error.
- Verdict order, first match wins: forbid, allow, public (internal file from outside the module), layers, same module, public entry, otherwise unplanned (rule `none`), or forbidden (rule `default`) with `default: forbidden`. Write `default: forbidden` only when the user wants every import the rules do not cover to fail `check`; an older archgraph ignores the key, so the version must be pinned where it gates CI.
- Give every forbid an `id` and a `reason` taken from the approved layout. Use `reachable: true` when the rule is about what a file can reach transitively (for example, browser code reaching server code).
- After writing it, run `archgraph scan <root> --rules layout.yaml -o <tmp>/graph.json` and `archgraph check <tmp>/graph.json`. Walk the user through every forbidden and unplanned import: fix the rules where they misstate the layout, and list the rest as violations to fix.

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
