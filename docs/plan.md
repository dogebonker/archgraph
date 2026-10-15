# archgraph v1 plan

Revision 2: folds in the independent critique (findings F1 to F20, noted inline as `[Fn]`).

## Outcome

One Rust binary, `archgraph`, plus an Agent Skill at `skills/archgraph/SKILL.md`, that turns a repository's import graph into evidence for architecture decisions:

- `archgraph scan <root> [--rules layout.yaml] [-o graph.json]`: collect file-level import edges with an existing tool, group files into modules, compute per-module coupling and cohesion, cycles, findings, and (with rules) edge verdicts. Writes one JSON document, `kind: scan`, schema `archgraph/v1`. This document is the only contract Software Factory consumes.
- `archgraph check graph.json --rules layout.yaml [-o checked.json]`: recompute the document with those rules and print forbidden and unplanned edges. Exit 1 when any edge is forbidden, 2 on usage or input errors.
- `archgraph render graph.json [--rules layout.yaml] -o graph.html`: one self-contained HTML file, no external requests.
- `archgraph simulate graph.json --move <file> <dest-path> [--rules layout.yaml]`: forbidden edges, cycles and cohesion before and after a proposed move. `kind: simulation`.
- `archgraph diff old.json new.json`: per-module metric deltas, new or removed cycles and forbidden edges (trend over time). `kind: diff`.

Only `scan` runs a collector. Every other subcommand reads `files`, `edges` and `config` from a scan document and recomputes modules, metrics, cycles, findings and verdicts from them with the same function `scan` uses, so module ids, metrics and verdicts always agree with the rules in effect [F2]. A document that lists a file twice, gives a file a module its embedded config does not, or has an edge naming a file not in `files` is an input error (exit 2), never a crash.

## Non-goals (v1)

- Go and Rust collectors (`go list -json`, `cargo metadata` / cargo-modules). The collector seam is one function per language returning `RawGraph`; adding one is a new file. Next after v1.
- Parsing source code. Every edge comes from dependency-cruiser or grimp.
- External packages (npm, PyPI, stdlib). Only edges between files inside the scanned root.
- A single health score in the JSON. Render derives colors; the contract carries measured values and findings with evidence.
- Auto-applying moves or writing anything into the scanned repository (including tool caches [F6]).
- Editing the Software Factory repository.

## Crate layout

One crate, `archgraph`, library plus binary (the library lets integration tests and the binary share code; it is not a published API).

```
Cargo.toml
src/main.rs                 clap CLI, one function per subcommand
src/lib.rs                  analyze(files, edges, config, rules) -> Document: the one pipeline every subcommand calls
src/model.rs                schema v1 types (serde), the contract
src/collect/mod.rs          RawGraph, default excludes, language detection
src/collect/depcruise.rs    run depcruise or read its JSON; map to RawGraph
src/collect/grimp.rs        run python with grimp_dump.py or read its JSON; map to RawGraph
src/collect/grimp_dump.py   embedded with include_str!
src/modules.rs              module assignment (depth, rules `modules:`), barrel detection
src/graph.rs                module metrics, Tarjan SCC (module and file level), cycle break set, modularity, label propagation
src/findings.rs             the recommendation analyses
src/rules.rs                layout.yaml parsing, glob compilation, edge verdicts, reachability forbids
src/simulate.rs             apply a move, rerun analyze, compare summaries
src/render.rs + assets/graph.html
src/cochange.rs             optional git log co-change analysis
src/diff.rs
schema/archgraph.v1.schema.json   JSON Schema draft 2020-12, hand-written, tested against every fixture output
skills/archgraph/SKILL.md
tests/fixtures/<name>/      fixture repositories with recorded collector output
tests/*.rs                  integration tests running the binary
```

Dependencies: clap 4 (derive), serde, serde_json, serde_yaml_ng 0.10 (serde_yaml 0.9 is deprecated upstream), globset, anyhow, sha2 (rules file hash). Dev: jsonschema with `default-features = false` [F20]. Tests run the binary with `std::process::Command` and `env!("CARGO_BIN_EXE_archgraph")`.

## Definitions

- **File**: a source file inside the root, path relative to the root with `/` separators. Source means `.ts/.tsx/.js/.jsx/.mjs/.cjs` or `.py`; imports of anything else (stylesheets, JSON, images) are dropped.
- **Excluded by default** (removed before anything else): generated code `**/generated/**`, `**/__generated__/**`, `**/*.generated.*`, `**/*.gen.*`, `**/*_pb2.py`, `**/*_pb2_grpc.py`, `**/*.d.ts`; vendored and build output `**/node_modules/**`, `**/dist/**`, `**/build/**`; tests `**/*.test.*`, `**/*.spec.*`, `**/__tests__/**`, `**/test_*.py`, `**/*_test.py`, `**/conftest.py`, `**/tests/**` [F8]. `--include-tests` keeps tests; `--exclude <glob>` adds; `--no-default-excludes` drops all defaults.
- **Module**: the unit metrics are computed for. Rules `modules:` patterns win; other files fall back to the file's directory truncated to `--depth` segments below the source root (default 2; `src/` when present, else the scan root; for Python, the package directory). A file directly in the source root is module `.`. A file outside the source root (depcruise follows an import from `src/` to a root `lib/u.ts`) counts from the scan root with a `../` prefix (`../lib`, `..` for a file directly in the root), so it never merges with `src/lib`. Depth modules can nest by name (`shared` holds `src/shared/x.ts`, `shared/ui` holds `src/shared/ui/y.ts`) and a Python `pkg/modules` folder merges every module under it; both are documented with `modules:` as the fix [F15].
- **Edge**: file `from` imports file `to`. One edge per pair. `kind` precedence when depcruise reports the pair more than once: `static` > `reexport` > `dynamic` > `type`; an edge is `type` only when every report of the pair is type-only [F1]. Type-only edges count in metrics, cycles and verdicts like any other (Factory's FDA layout keeps them).
- **Barrel** [F7]: TypeScript/JS: a file named `index.{ts,tsx,js,jsx,mjs,cjs}` whose outgoing edges are all `reexport`. Python: an `__init__.py` whose every import is of its own submodules is a barrel; any other `__init__.py` is ordinary code. Barrels stay in the graph and in verdicts. For metrics, cohesion, sink, misplaced-file and seam analyses, an edge `consumer -> barrel` is attributed to each of the barrel's re-export targets, and the barrel's own outgoing edges are dropped; evidence that relies on this says `"via_barrel": true` because it over-approximates (the consumer may use only one of the targets).
- **Ca** (afferent): number of distinct other modules with at least one edge into this module. **Ce** (efferent): number of distinct other modules this module has an edge to. Package-level counting, as JDepend does.
- **Instability** I = Ce / (Ca + Ce); `null` when Ca + Ce = 0.
- **Cohesion** = internal / (internal + outgoing), counting edges (after barrel attribution) whose importer is in the module: internal when the target is in the same module, outgoing otherwise. `null` when the module has no such edges.
- Floats are rounded to 4 decimals in output [F10].

## Schema v1 (`archgraph/v1`)

Every document has `schema: "archgraph/v1"` and `kind: "scan" | "simulation" | "diff"` [F10]. Consumers check `schema` and reject unknown majors; they ignore unknown fields and unknown finding kinds. Adding optional fields or new finding kinds keeps v1; renaming, removing or changing meaning bumps to v2.

```json
{
  "schema": "archgraph/v1",
  "kind": "scan",
  "tool": { "name": "archgraph", "version": "0.1.0" },
  "source": {
    "language": "typescript",
    "collector": { "name": "dependency-cruiser", "version": "18.4.0", "cruised": 42, "unresolved": 0 },
    "git": { "commit": "abc123", "dirty": false },
    "generated_at": "2026-10-15T12:00:00Z"
  },
  "config": { "depth": 2, "source_root": "src", "excludes": ["**/generated/**"], "include_tests": false,
              "rules": { "path": "layout.yaml", "sha256": "…", "doc": { "version": 1, "...": "the parsed rules" } } },
  "files":   [ { "path": "src/domains/cart/ui/Cart.tsx", "module": "domains/cart", "barrel": false } ],
  "edges":   [ { "from": "src/a.ts", "to": "src/b.ts", "kind": "static", "verdict": null, "rule": null } ],
  "modules": [ { "id": "domains/cart", "files": 4, "ca": 1, "ce": 2, "instability": 0.6667,
                 "internal_edges": 6, "outgoing_edges": 3, "cohesion": 0.6667 } ],
  "module_edges": [ { "from": "domains/cart", "to": "shared", "imports": 3, "forbidden": 0 } ],
  "cycles": [ { "level": "module", "members": ["a", "b", "c"], "exact": true,
                "break": [ { "from": "src/c/x.ts", "to": "src/a/y.ts" } ] } ],
  "summary": { "forbidden": 0, "unplanned": 0, "cycles": 1,
               "modules": { "domains/cart": { "cohesion": 0.6667, "ca": 1, "ce": 2, "instability": 0.6667 } } },
  "findings": [ { "kind": "misplaced-file", "subject": "src/a/helper.ts",
                  "evidence": { "own_module_edges": 1, "target_module": "b", "target_module_edges": 4 },
                  "suggestion": { "move": { "file": "src/a/helper.ts", "to": "src/b/helper.ts" } },
                  "simulation": { "after": "Changes" } } ],
  "cochange": null
}
```

- No absolute paths: the root is not recorded, so scans of the same tree in two worktrees are byte-identical except `generated_at` and `git` [F10].
- Sorted arrays: files by path, edges by (from, to), modules by id, module_edges by (from, to), cycles by members, findings by (kind, subject).
- Finding `kind` is an open string: a consumer skips kinds it does not know. The known kinds type their `evidence` in the schema.
- `config.rules.doc` embeds the parsed rules so `check`, `render` and `simulate` can reuse them without the file; `--rules` on those commands replaces them. Unknown keys in it are ignored (a newer archgraph may add rule keys); only loading `layout.yaml` rejects them.
- `verdict` is `null` without rules, else `allowed | forbidden | unplanned`; `rule` is the deciding rule's `id` when it has one, else `forbid[i]`, `allow[i]`, `public`, `layers`, `same-module`, or `none` [F10].
- `cycles[].level` is `module` or `file`; file-level SCCs are computed over the imports within each module, so a cycle inside one module is reported even when its files also take part in a cross-module cycle (which shows as a module cycle) [F13].
- `Summary = { forbidden: int, unplanned: int, cycles: int, modules: { <id>: { cohesion, ca, ce, instability } } }`, the top-level `summary` of a scan. A finding's `simulation` is `{ after: Changes }`: the Summary after the move with `modules` cut to the modules whose metrics changed (a module the move empties is `null`); the counts are always present, and the state before is the scan's own `summary`. `simulate` keeps the full `before`/`after` pair [F10].
- `kind: simulation`: `{ schema, kind, move: { file, to, module }, before: Summary, after: Summary, new_forbidden: [edge], removed_forbidden: [edge] }`.
- `kind: diff`: `{ schema, kind, old: { commit, generated_at }, new: {...}, modules: { <id>: { cohesion: [old, new], ca: [...], ce: [...], instability: [...] } }, added_modules, removed_modules, new_cycles, resolved_cycles, new_forbidden, resolved_forbidden }`. Pairs are fine here: a diff is a pair by definition.
- Each finding kind's `evidence` object is defined in `$defs` of the schema file.

## Collectors

**TypeScript/JavaScript (dependency-cruiser 18.4.0 and typescript 6.0.3, the Factory pins).** `scan` runs, in the root:
`npx --no-install depcruise --output-type json --no-config --do-not-follow node_modules --ts-pre-compilation-deps [--ts-config tsconfig.json] <source_root>` [F1].
`--ts-config` is passed when `tsconfig.json` exists (resolves `@/` style aliases); `--ts-pre-compilation-deps` keeps `import type` and type-only-used imports. When `src/` exists only `src/` is cruised (plus files its imports reach). `npx` resolves the scanned repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`, so a scan runs tooling the repository controls: scan only trusted repositories, or inside a sandbox. Fails with an install hint when depcruise is missing, and fails when `summary.totalCruised` is 0 (the TS 7 failure mode in Factory's tools.yaml). Records `cruised` and `unresolved` (count of `couldNotResolve` dependencies) in `source.collector`; prints a warning when `unresolved > 0`. `--collector-input depcruise.json` reads a file instead (for users with their own `.dependency-cruiser.cjs`). Mapping: `modules[].source` → files; `dependencies[]` whose `resolved` is a source file inside the root and not `coreModule`/`couldNotResolve` → edges; `dependencyTypes` containing `export` (including `type-only` with `export`, from `export type … from`) → `reexport`, so an index that re-exports types is still a barrel; otherwise `type-only` → `type`, `dynamic-import` → `dynamic`, else `static`; then the precedence rule above.

**Python (grimp 3.17, the version import-linter 2.15 requires).** `scan --python <interp>` (default `python3`) runs `<interp> -I -B -c <grimp_dump.py> <source_root> <package>` with `cache_dir=None` passed to `grimp.build_graph`, so nothing is written into the repository [F6]. `-I` keeps `PYTHONPATH`, the working directory and user site-packages off `sys.path`; the helper imports `json` and `grimp` first and only then appends the source root, so a repository file such as `json.py` cannot shadow them and run. It prints importer and imported; grimp reads source statically. Rust maps dotted names to `a/b.py` or `a/b/__init__.py`; a namespace package (a directory without `__init__.py`) is no file and is left out. Missing grimp → error naming `pip install grimp==3.17` or `uv run --with grimp==3.17`. `--collector-input grimp.json` reads recorded output. The package defaults to the single top-level package under the source root: a directory with an `__init__.py` or, for a namespace package, with `.py` files, skipping hidden and excluded directories (`tests` under the default excludes); ambiguity is an error asking for `--package`.

**Language detection** [F19]: count `.ts/.tsx/.js/.jsx/.mjs/.cjs` against `.py` files under the source root (after default excludes); the larger wins. `--language` overrides.

## Rules file (`layout.yaml`)

Shapes taken from Factory's `kit/layouts/*.md` Layers and sibling rules, so a Layout Decision translates line by line, except that a capture in a layer becomes `*`. Globs use globset with `literal_separator(true)`: `*` is one path segment, `**` any depth, `{a,b}` alternation [F3]. `<name>` captures one segment (Factory writes `<domain>`, `<pkg>`) [F3] and binds within one rule: in `modules` (`path` to `id`), in `public` (`module` to `entries`), and in `forbid` and `allow` (the value `from` matched in the importer must recur in the target, so `from: "src/modules/<name>/**", to: "src/modules/<name>/**"` means within one module). `to`, `id` and `entries` may use only the capture of their rule's pattern; a capture in `layers` is an error (write `*`). Unknown keys are an error when loading `layout.yaml`, so a typo cannot silently allow everything.

```yaml
version: 1
modules:                          # optional; overrides --depth for matching files
  - { id: "domains/<name>", path: "src/domains/<name>/**" }
layers:                           # lower first, as in lenses.yaml; a file may import its own or a lower layer
  - "src/shared/**"               # a file matching several layers takes the first [F18]
  - "src/domains/*/{types,constants}.ts"
  - "src/domains/*/repo.server.ts"
  - "src/domains/*/model.server.ts"
  - "src/domains/*/client.ts"
  - "src/domains/*/controller.ts"
  - "src/domains/*/ui/**"
  - "app/**"                      # layers take `*`, never a capture
public:                           # module pattern -> its public entries [F4]
  - { module: "src/domains/<name>/**", entries: ["src/domains/<name>/index.ts", "src/domains/<name>/server.ts"] }
forbid:
  - { id: shared-no-domain, from: "src/shared/**", to: "src/domains/**", reason: "shared imports no domain" }
  - { id: browser-no-server, from: "src/domains/*/{client,controller,index}.ts", to: "**/*.server.ts",
      reachable: true, reason: "browser graph never reaches server code" }   # [F5]
allow:
  - { from: "app/**", to: "**", reason: "composition" }
```

`<name>` in `public` binds the module pattern and its entries: an edge into `src/domains/b/client.ts` from outside `src/domains/b/**` must target `src/domains/b/index.ts` or `src/domains/b/server.ts`. In `forbid` and `allow`, `<name>` is bound by the importer: `{ from: "src/modules/<name>/application/**", to: "src/modules/<name>/domain/**" }` matches `src/modules/a/application/x.ts → src/modules/a/domain/y.ts`, never `→ src/modules/b/domain/y.ts`.

Verdict per edge, first match wins:
1. A `forbid` rule matches the edge (importer matches `from`, target matches `to` with `from`'s capture bound) → forbidden. A `reachable: true` forbid matches when the importer matches `from` and any file matching `to` is reachable from the edge's target through import edges (BFS over the file graph, including reexports), or the target itself matches `to`; the evidence path is reported by `check` [F5].
2. An `allow` rule matches → allowed.
3. The target matches a `public` module pattern, the importer is outside that module instance, and the target is not one of its entries → forbidden (rule `public`). This applies whether or not the entry file exists, so a module without an `index.ts` is still private [F4].
4. Both files are in a layer: target layer index ≤ importer layer index → allowed, else forbidden (rule `layers`).
5. Same module → allowed (rule `same-module`).
6. Cross-module edge into a `public` entry → allowed (rule `public`).
7. Otherwise → unplanned (rule `none`).

Traces against Factory layouts:
- FDA Next, `src/domains/cart/ui/Cart.tsx → src/domains/checkout/index.ts`: steps 1-2 no match; 3: target is the entry, skip; 4: index is in no layer; 5: different modules; 6 → allowed.
- FDA Next, `src/domains/cart/client.ts → src/domains/cart/model.server.ts`: step 1 `browser-no-server` → forbidden (layers alone would allow it, which is why the reachable forbid exists).
- FDA, `src/domains/cart/index.ts` re-exporting `model.server.ts`, imported by `ui/`: step 1 on the index edge (`from` includes `index`) → forbidden.
- Modular monolith, `src/modules/a/application/x.ts → src/modules/b/domain/y.ts` with `public: [{ module: "src/modules/<name>/**", entries: ["src/modules/<name>/index.ts"] }]`: step 3 → forbidden even when `b/index.ts` does not exist.
- Hexagonal TS, `layers: ["src/domain/**", "src/application/**", "src/adapters/**", "src/main.ts"]` plus `forbid: [{ from: "src/adapters/in/**", to: "src/adapters/out/**" }, { from: "src/adapters/out/**", to: "src/adapters/in/**" }]`: `src/domain/user.ts → src/adapters/out/pg.ts` → step 4 forbidden; `src/adapters/in/http.ts → src/adapters/out/pg.ts` → step 1 forbidden; `src/adapters/in/http.ts → src/application/create.ts` → step 4 allowed.

## Analyses (findings)

Every finding has `kind`, `subject`, `evidence` (typed per kind in the schema) and, when it proposes a move, `suggestion.move` (file and destination path [F9]) plus a `simulation` (`{ after: Changes }`) computed by the same code as `simulate`. A move is suggested only when its destination is free: no file of that name in the target directory and no other suggested move to the same path. Thresholds are constants in `findings.rs`, each with a comment; they are tuning knobs, not truths.

| kind | trigger | suggestion |
| --- | --- | --- |
| `sink-split` | module with Ca ≥ 3 whose bipartite graph of its files and their importing modules (files imported only from inside the sink join their importers' component) has ≥ 2 connected components [F11] | split into one module per component; a component with one consumer module is suggested as a move into it, unless two of its files share a basename |
| `misplaced-file` | a non-barrel file, not a package `__init__.py` nor its module's only file, with all of: (a) at least one importer (a file nobody imports is an entry point a framework locates, such as `page.tsx` or `route.ts`); (b) importers from other modules in at most one module (a file several modules import is a shared provider); (c) ≥ 2 edges (in + out) to the target module and more than to its own; (d) the move adds no forbidden import or cycle, gives the target module no new dependency (its Ce does not grow) and raises the cohesion of the source and target modules taken together (internal / (internal + outgoing) over both). When the basename is taken in the target directory, (d) is checked on a free name there and the finding has no suggestion | move to `<target module dir>/<basename>`, with simulation |
| `cycle` | each module-level SCC of size ≥ 2, and each file-level SCC of the imports within one module [F13] | the break set: fewest file imports whose removal makes the SCC acyclic. Module level: weight of a module edge = its import count; exact search over the SCC's module edges when there are ≤ 16, greedy Eades-Lin-Smyth above that, `exact: false` |
| `internals-reach` | an edge with verdict forbidden by rule `public`; without rules, TS only: a cross-module edge into a non-barrel file of a module that has a TS barrel. Not emitted for Python without rules [F7] | import the public entry instead |
| `unstable-dependency` | module A depends on B with I(B) − I(A) ≥ 0.25 (stable dependencies principle), over the same barrel-attributed module pairs Ca and Ce count; `imports` is the number of attributed file pairs | depend on an abstraction owned by A, or move the depended-on code |
| `seam` | label propagation over the undirected file graph with hub files (degree above the 90th percentile) removed, at most 50 iterations, deterministic order; reported only when the clustering's modularity Q exceeds the folder partition's Q by ≥ 0.05 and the clusters (of 2 or more files) hold at least half of the non-barrel files. Evidence carries both Q values and each cluster's files [F12] | the clusters that disagree with folders, as candidate boundaries |
| `hidden-coupling` | with `--cochange[=N]` (N ≥ 1, default 300 commits), file pairs in different modules changed together in ≥ 3 commits with no direct edge between them. Commits touching more than 30 files are skipped; renames are followed with `git log --name-status -M` [F17]. Every git command runs with `-c core.fsmonitor=false`, so a repository's config cannot make archgraph run a program | evidence only |
| `small-repo` | fewer than 20 files or 3 modules | says findings carry little signal |

## Render

`assets/graph.html` with the scan document embedded as `<script type="application/json">`, with every `<` in the JSON written as `<` so a path containing `</script>` cannot break out [F16]. Inline JS draws modules as circles (area by file count) with a small deterministic force layout and module edges as arrows (width by import count). Health color: green when cohesion ≥ 0.5 and Ca + Ce ≤ the median, red when cohesion < 0.3 and Ca + Ce > the median, amber otherwise, grey for modules with fewer than 2 files. A module edge with any forbidden file edge is red. Clicking a module highlights its importers and lists importing modules and their file edges in a side panel. A `Content-Security-Policy` meta tag (`default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'`) blocks external requests; a test asserts the output contains no `http://`, `https://` or `//` resource URL and that a `</script>` path stays escaped.

## Tests and fixtures (written before the code they test)

Each fixture is a small repository under `tests/fixtures/` with sources, an optional `layout.yaml`, and the recorded collector output (`depcruise.json` or `grimp.json`), regenerated by `tests/fixtures/record.sh` with the pinned tools. The default suite reads recorded output, so `cargo test` needs neither Node nor Python.

| fixture | language | proves |
| --- | --- | --- |
| `ts-basic` | TS | Ca, Ce, instability, cohesion by hand; sorted, byte-stable output; schema validation; contains an `@/` alias import and an `import type` that must appear as edges [F14] |
| `ts-cycles` | TS | a 3-module and a 2-module cycle with the known minimum break set; a file cycle inside one module |
| `ts-sink` | TS | `utils` used by two consumer groups → `sink-split` with both components |
| `ts-misplaced` | TS | a file that talks mostly to another module → `misplaced-file`; simulation shows cohesion up |
| `ts-internals` | TS | feature A imports feature B's internal file while B has a barrel → `internals-reach` |
| `ts-sdp` | TS | stable module importing a volatile one → `unstable-dependency` |
| `ts-seams` | TS | folders that cut across two real clusters → `seam` with both Q values |
| `ts-fda` | TS (FDA Next layout) | allowed cross-domain public import; client → model.server forbidden; server re-export through index forbidden; cross-domain internal forbidden; shared → domain forbidden; a file in no layer unplanned; `check` exits 1 [F5, F14] |
| `ts-modmono` | TS | modular-monolith index-only, including a module with no `index.ts` [F14] |
| `ts-hexagonal` | TS | inward layers and adapter siblings in both directions [F14] |
| `ts-excludes` | TS | generated and test files gone; barrel attribution in cohesion; an `export type … from` re-export keeps the index a barrel |
| `py-clean` | Python | hexagonal layers; adapters ↔ entrypoints sibling forbid [F14] |
| `py-cycles` | Python | grimp mapping, barrel vs code `__init__.py`, a cycle |
| rules errors | — | unknown key rejected; `*` does not cross `/` [F3, F14] |
| simulate CLI | TS | moving a file changes forbidden, cycles and cohesion as computed by hand; destination path decides the layer [F9, F14]. Applying moves (unit test) rejects a file moved twice or two files moved to one path |
| cochange | git | the test builds a temporary repo with commits; `hidden-coupling` appears; a 31-file commit is ignored |
| diff | TS | scans of `ts-cycles` before and after a fix: cycle resolved |
| render | TS | no external URLs; `</script>` in a path stays escaped [F14, F16] |

Live collector tests run the real depcruise and grimp on the fixtures and assert the alias and type-only edges exist and the mapping equals the recorded output, and that a `json.py` planted in a Python source root never runs. They are `#[ignore]` locally, run in CI with `cargo test -- --ignored` after installing the pinned tools, and are run in this session with the exact command reported [F14].

## Limits (README and SKILL.md)

- `scan` runs tooling the scanned repository controls: `npx` resolves its own `node_modules/.bin/depcruise` and reads its `.npmrc`. Scan only trusted repositories, or inside a sandbox.
- With a `src/` directory only `src/` is cruised; files outside it appear only when `src/` imports them.
- Type-only imports count in metrics, cycles and verdicts.
- Imports are not semantics: DI containers, event buses, string-based lookups, reflection and dynamic imports without static strings are invisible. `--cochange` partly covers this.
- Small repositories have little signal; the `small-repo` finding says so.
- Findings are moves with evidence. There is no single score to optimize; module colors in `render` are a reading aid.
- Module boundaries come from folders (`--depth`) or rules `modules:`; wrong boundaries give wrong metrics.
- Barrel attribution over-approximates: a consumer of an index is counted against every re-exported file.

## Phases

1. Skeleton, `model.rs`, schema file, depcruise mapping, modules and barrels, metrics, cycles and break set, `analyze`, `scan`. Fixtures `ts-basic`, `ts-cycles`, `ts-excludes`.
2. grimp collector and helper; `py-cycles`.
3. Rules, verdicts, `check`; `ts-fda`, `ts-modmono`, `ts-hexagonal`, `py-clean`, rules errors.
4. Findings and `simulate`; `ts-sink`, `ts-misplaced`, `ts-internals`, `ts-sdp`, `ts-seams`, simulate CLI.
5. `render`, `diff`, `--cochange`.
6. `SKILL.md`, README, LICENSE files, CI workflow, live collector tests.

License: MIT OR Apache-2.0 (Rust convention).
