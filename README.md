# archgraph

archgraph turns a repository's import graph into evidence for architecture decisions. It collects file-level imports with an existing tool (dependency-cruiser for TypeScript/JavaScript, grimp for Python), groups files into modules, and reports per-module coupling and cohesion, cycles with their smallest break set, rule verdicts for every import, and findings that propose file moves backed by a simulation of each move.

It never writes into the scanned repository and never parses source code itself.

**Scan only repositories you trust, or scan inside a sandbox.** For TypeScript/JavaScript, `scan` runs `npx --no-install depcruise` in the scanned repository, and npx resolves that repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`: the repository controls what runs. The Python collector runs your interpreter with `-I` and adds the source root to `sys.path` last, so repository files cannot shadow the standard library or grimp; git runs with `core.fsmonitor=false`.

## Install

```sh
cargo install --git https://github.com/OWNER/archgraph --locked
archgraph --version
```

The collector runs inside the scanned repository:

- TypeScript/JavaScript: `npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3` in that repository. `scan` runs `npx --no-install depcruise`.
- Python: a Python with `grimp==3.17` (`pip install grimp==3.17`, or `uv run --with grimp==3.17`), passed with `--python`.

## Commands

`scan` is the only command that runs a collector. The others read a scan document and recompute modules, metrics, cycles, verdicts and findings from its files, edges and config with the same code, so their output always agrees with the rules in effect.

**scan** writes the scan document (`kind: scan`).

```sh
archgraph scan . -o graph.json                        # language detected, modules = 2 folders below src/
archgraph scan . --rules layout.yaml -o graph.json    # with edge verdicts
archgraph scan . --cochange -o graph.json             # also read the last 300 commits (--cochange=N, N >= 1)
```

A module is a file's directory truncated to `--depth` segments below the source root (`src/` when present, else the root; for Python, the package directory). Each file directly in the source root is a module of its own, named by its file name with extension (`main.ts`, `errors.py`), so a package's lowest layer and its entry points never merge into one module. Imports between two such files are therefore cross-module: rules step 5 (same module) no longer allows them, so under `default: forbidden` they need an `allow` or `layers` rule. Scan documents written by commit 2bf5805 put these files in module `.` and fail `check`, `render` and `simulate` as inconsistent: rescan them. When the root has a `src/` directory, only `src/` is cruised (files outside it appear when `src/` imports them, with module ids prefixed `../`: `../lib`, and `../tool.ts` for a file directly in the root). Only `.ts/.tsx/.js/.jsx/.mjs/.cjs` and `.py` files are files; imports of stylesheets, JSON and other assets are dropped. Type-only imports (`import type`) are edges like any other: they count in metrics, cycles and verdicts.

Options: `--depth N`, `--exclude <glob>` (repeatable), `--include-tests`, `--no-default-excludes`, `--language typescript|python`, `--python <interp>`, `--package <pkg>`, `--collector-input <recorded.json>`.

**check** prints forbidden and unplanned imports. Exit 1 when any import is forbidden, 2 on usage or input errors.

```sh
$ archgraph check graph.json --rules layout.yaml
unplanned src/domains/cart/controller.ts -> src/lib/clock.ts (none)
forbidden src/domains/checkout/controller.ts -> src/domains/checkout/client.ts (browser-no-server) via src/domains/checkout/client.ts -> src/domains/checkout/model.server.ts
forbidden src/shared/format.ts -> src/domains/cart/types.ts (shared-no-domain)
2 forbidden, 1 unplanned
```

**simulate** compares forbidden imports, unplanned imports, cycles and module metrics before and after moving one file (`kind: simulation`, with full `before` and `after` summaries). The destination path decides the new module and layer.

```sh
archgraph simulate graph.json --move src/utils/dates.ts src/calendar/dates.ts
```

**render** writes one self-contained HTML page: a zoomable map of modules as circles colored by health, holding their files grouped by folder, with module imports as curved arrows (red forbidden, amber unplanned, dashed type-only). Zoom in (wheel, pinch, `+`/`-`) and the files, their imports and their names appear. Hover or click a module for its importers and imports, a file or an import for its verdict and rule; the side panel lists findings by kind and draws suggested moves with their before and after numbers. Search, filters for forbidden, unplanned and cycle imports, and hiding busy modules keep large graphs readable. The page makes no network requests.

```sh
archgraph render graph.json -o graph.html
```

**diff** compares two scan documents (`kind: diff`): per-module metric pairs `[old, new]`, added and removed modules, new and resolved cycles, new and resolved forbidden imports.

```sh
archgraph diff before.json after.json
```

`check`, `simulate` and `render` take `--rules layout.yaml` to replace the rules embedded in the scan.

## Schema contract

Every document has `schema: "archgraph/v1"` and `kind: "scan" | "simulation" | "diff"`. The contract is [schema/archgraph.v1.schema.json](schema/archgraph.v1.schema.json) (JSON Schema draft 2020-12); every test output is validated against it.

- Consumers check `schema` and reject unknown major versions. They ignore unknown fields and unknown finding kinds (`kind` is an open string in the schema; known kinds have typed evidence).
- A scan has a top-level `summary`: forbidden, unplanned and cycle counts and each module's cohesion, Ca, Ce and instability. A finding's `simulation` is `{ "after": … }`: the same counts after the move, with `modules` listing only the modules whose metrics changed (`null` for a module the move empties). Compare it with the scan's `summary`.
- A scan document whose files, modules and edges disagree (an edge naming a file not in `files`, a file listed twice, a module its config does not give) is an input error, exit 2.
- Adding optional fields or new finding kinds keeps v1. Renaming, removing or changing the meaning of a field bumps to v2.
- No absolute paths. Arrays are sorted, so two scans of one tree differ only in `generated_at` and `git`.
- Floats are rounded to 4 decimals. Instability is `Ce / (Ca + Ce)`, cohesion is `internal / (internal + outgoing)` imports, both `null` when undefined.

## Rules file

```yaml
version: 1
modules:                      # optional; overrides --depth for matching files
  - { id: "domains/<name>", path: "src/domains/<name>/**" }
layers:                       # lowest first; a file may import its own or a lower layer
  - "src/shared/**"
  - "src/domains/*/model.server.ts"
  - "src/domains/*/client.ts"
  - "src/domains/*/ui/**"
public:                       # a module's public entries
  - { module: "src/domains/<name>/**", entries: ["src/domains/<name>/index.ts"] }
forbid:
  - { id: shared-no-domain, from: "src/shared/**", to: "src/domains/**", reason: "shared imports no domain" }
  - { id: browser-no-server, from: "src/domains/*/client.ts", to: "**/*.server.ts", reachable: true }
allow:
  - { from: "app/**", to: "**", reason: "composition" }
default: forbidden            # optional; imports no rule decides are forbidden, not unplanned
```

Globs: `*` is one path segment, `**` any depth, `{a,b}` alternation. `<name>` captures one segment and binds it within a rule: `modules` path to id, `public` module to entries, and `forbid`/`allow` importer to target, so `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }` matches only imports within one module. `to`, `id` and `entries` may use only their rule's capture, and `layers` take no capture (write `*`). A file matching several layers takes the first. Unknown keys in `layout.yaml` are an error; rules embedded in a scan document may carry keys a newer archgraph added.

Verdict per import, first match wins:

1. A `forbid` rule matches: **forbidden**. With `reachable: true` it also matches when any file matching `to` is reachable from the imported file.
2. An `allow` rule matches: **allowed**.
3. The target is inside a `public` module, the importer is outside that module, and the target is not an entry: **forbidden** (rule `public`), whether or not the entry exists.
4. Both files are in layers: target layer at or below the importer's: **allowed**, else **forbidden** (rule `layers`).
5. Same module: **allowed** (rule `same-module`).
6. Cross-module import of a `public` entry: **allowed** (rule `public`).
7. Otherwise: **unplanned** (rule `none`), or with `default: forbidden` **forbidden** (rule `default`), so `check` exits 1 on them.

`default: forbidden` turns the rules into a whitelist. An archgraph older than this key reads a scan document whose embedded rules carry it and ignores the key (unknown keys in embedded rules are tolerated), so its `check` reports those imports as unplanned and exits 0. Pin the archgraph version wherever a whitelist gates CI.

## Findings

| kind | when | suggestion |
| --- | --- | --- |
| `cycle` | a module-level import cycle, or a file cycle among the imports inside one module | the fewest imports to remove (`break`). Module cycles also list `two_way`: each pair of members importing each other as its lighter direction, with its import counts both ways and up to 20 of its file imports, sorted by imports. The lighter side of a pair is the cheapest cut to act on when the break set is large |
| `misplaced-file` | a file (not a barrel, not a module's only file) imported by at least 2 files of one other module and by no third module, with more of those imports than edges (in + out) within its own module, whose move adds no forbidden import or cycle, gives the target no new dependency and raises the two modules' combined cohesion. What the file itself imports never pulls it: a router importing `app/deps` stays where it is | a move into that module, keeping the file's folder within its module when the target has it (`chains/domain/hd.py` into signer is `signer/domain/hd.py`), else its root. The file's folder is read relative to the deepest directory its own module's files share, and the target's folder counts only when a file of the target module sits in it. With simulation; none when the name is taken there or the destination would fall in another module |
| `sink-split` | a module imported by 3 or more modules whose files serve separate groups of importers | one module per group; single-consumer groups as moves, unless two files share a name |
| `internals-reach` | an import past a module's public entry (rules), or past a TypeScript barrel (no rules) when at least half the cross-module imports into that module go through its barrels (`barrel_share`): a barrel most importers bypass is not the module's API | import the entry instead |
| `unstable-dependency` | module A imports B (through barrels, as Ca and Ce count) and B is at least 0.25 more unstable than A | depend on an abstraction owned by A, or move the code |
| `seam` | file clusters that fit the imports better than the folders do (modularity gain of at least 0.05), holding at least half of the non-barrel files; files with a degree above the 90th percentile are left out as hubs | the clusters as candidate boundaries |
| `hidden-coupling` | with `--cochange`, files in different modules changed together in 3 or more commits without importing each other (commits touching more than 30 files are skipped, renames followed) | evidence only |
| `small-repo` | fewer than 20 files or 3 modules | findings carry little signal |

## Limits

- `scan` runs tooling the scanned repository controls (see the warning at the top): scan only trusted repositories, or inside a sandbox.
- With a `src/` directory only `src/` is cruised; files outside it appear only when `src/` imports them.
- Type-only imports count in metrics, cycles and verdicts (Factory's FDA layout keeps them).
- Imports are not semantics: DI containers, event buses, string-based lookups, reflection and dynamic imports without static strings are invisible. `--cochange` partly covers this.
- Small repositories have little signal; the `small-repo` finding says so.
- Findings are moves with evidence. There is no single score to optimize; module colors in `render` are a reading aid.
- Module boundaries come from folders (`--depth`) or rules `modules:`; wrong boundaries give wrong metrics.
- Barrel attribution over-approximates: a consumer of an index is counted against every re-exported file.

## Next

Go (`go list -json`) and Rust (`cargo metadata`) collectors.

## Agent skill

[skills/archgraph/SKILL.md](skills/archgraph/SKILL.md) teaches a coding agent the scan, check, simulate, render workflow.

## License

MIT OR Apache-2.0, at your option.
