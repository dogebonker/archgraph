# archgraph

archgraph turns a repository's import graph into evidence for architecture decisions. It collects file-level imports with an existing tool (dependency-cruiser for TypeScript/JavaScript, grimp for Python), groups files into modules, and reports per-module coupling and cohesion, cycles with their smallest break set, rule verdicts for every import, and findings that propose file moves backed by a simulation of each move.

It never parses source code itself, and writes into the scanned repository only when `report` runs without `--out-dir` (into `<root>/.archgraph/`).

**Scan only repositories you trust, or scan inside a sandbox.** For TypeScript/JavaScript, `scan` runs `npx --no-install depcruise` in the scanned repository, and npx resolves that repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`: the repository controls what runs. `--depcruise <path>` runs that executable instead, so the repository no longer picks the binary (depcruise still reads its `tsconfig.json`). The Python collector runs your interpreter with `-I` and adds the source root to `sys.path` last, so repository files cannot shadow the standard library or grimp; git runs with `core.fsmonitor=false`.

## Install

A prebuilt binary (Linux x64 and arm64, static; macOS x64 and arm64), installed to `~/.local/bin`:

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/dogebonker/archgraph/main/install.sh | sh
archgraph --version
```

[install.sh](install.sh) downloads the release archive and the release's `SHA256SUMS` over HTTPS only, and installs only when the archive's SHA-256 matches (it needs `sha256sum` or `shasum`). `ARCHGRAPH_VERSION=0.1.0` picks a release (default the latest), `ARCHGRAPH_INSTALL_DIR` the directory. On Windows, download `archgraph-<tag>-x86_64-pc-windows-msvc.zip` from [releases](https://github.com/dogebonker/archgraph/releases), or use npm.

A checksum file next to the archive proves the download is intact, not who built it. Every release archive also carries a build provenance attestation from the release workflow; check one with the GitHub CLI:

```sh
gh attestation verify archgraph-v0.1.0-x86_64-unknown-linux-musl.tar.gz --repo dogebonker/archgraph
```

For the maintainers: keep immutable releases turned on in the repository settings, so a published release's archives and tag cannot be replaced afterwards.

With Node 18 or later, [archgraph-cli](npm/README.md) downloads, verifies and caches the same binary on first run:

```sh
npx archgraph-cli report . --out-dir /tmp/arch
```

From source, with a Rust toolchain:

```sh
cargo install --git https://github.com/dogebonker/archgraph --locked
```

The collector runs inside the scanned repository:

- TypeScript/JavaScript: `npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3` in that repository. `scan` runs `npx --no-install depcruise`. Or install both once elsewhere (`npm install --prefix /opt/archgraph-tools dependency-cruiser@18.4.0 typescript@6.0.3`) and pass `--depcruise /opt/archgraph-tools/node_modules/.bin/depcruise`, which runs it with the same arguments.
- Python: a Python with `grimp==3.17` (`pip install grimp==3.17`, or `uv run --with grimp==3.17`), passed with `--python`.

## Commands

`scan` and `report` are the only commands that run a collector. The others read a scan document and recompute modules, metrics, cycles, verdicts and findings from its files, edges and config with the same code, so their output always agrees with the rules in effect.

**scan** writes the scan document (`kind: scan`).

```sh
archgraph scan . -o graph.json                        # language detected, modules = 2 folders below src/
archgraph scan . --rules layout.yaml -o graph.json    # with edge verdicts
archgraph scan . --cochange -o graph.json             # also read the last 300 commits (--cochange=N, N >= 1)
```

A module is a file's directory truncated to `--depth` segments below the source root (`src/` when present, else the root; for Python, the package directory). Each file directly in the source root is a module of its own, named by its file name with extension (`main.ts`, `errors.py`), so a package's lowest layer and its entry points never merge into one module. Imports between two such files are therefore cross-module: rules step 5 (same module) no longer allows them, so under `default: forbidden` they need an `allow` or `layers` rule. Scan documents written by commit 2bf5805 put these files in module `.` and fail `check`, `render` and `simulate` as inconsistent: rescan them. When the root has a `src/` directory, only `src/` is cruised (files outside it appear when `src/` imports them, with module ids prefixed `../`: `../lib`, and `../tool.ts` for a file directly in the root). Only `.ts/.tsx/.js/.jsx/.mjs/.cjs` and `.py` files are files; imports of stylesheets, JSON and other assets are dropped. Type-only imports (`import type`) are edges like any other: they count in metrics, cycles and verdicts.

Options: `--depth N`, `--exclude <glob>` (repeatable), `--include-tests`, `--no-default-excludes`, `--language typescript|python`, `--depcruise <path>` (a dependency-cruiser executable to run instead of `npx --no-install depcruise`; exit 2 when the path does not exist), `--python <interp>`, `--package <pkg>`, `--rules <layout.yaml>`, `--cochange[=N]`, `--collector-input <recorded.json>`.

**report** is the one command for people: it scans with the same options as `scan`, applies rules from `--rules`, else from `<root>/layout.yaml` when it exists (it prints which file it used), writes `archgraph.json` (the scan document) and `archgraph.html` (the `render` page) into `--out-dir` (default `<root>/.archgraph/`; add it to `.gitignore`), and prints a short summary built from the scan document: files, modules and imports; with rules, forbidden and unplanned counts by rule; module cycles with the largest one's size and the lightest two-way cut (from the cycles' `two_way`); findings by kind with counts and up to three subjects each; then the two paths, the page last. Headings are bold only when stdout is a terminal and `NO_COLOR` is unset. Exit codes are those of `check`: 1 when any import is forbidden, 0 otherwise, 2 on errors.

```sh
$ archgraph report . --out-dir /tmp/arch
archgraph report: .
rules: ./layout.yaml

Graph
  12 files, 4 modules, 13 imports

Rules
  5 forbidden: browser-no-server 3, public 1, shared-no-domain 1
  1 unplanned: none 1

Cycles
  no module cycles

Findings
  internals-reach (1)
    src/domains/cart/ui/Cart.tsx -> src/domains/checkout/client.ts
  small-repo (1)
    .

Wrote
  /tmp/arch/archgraph.json
  /tmp/arch/archgraph.html
```

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

**render** writes one self-contained HTML page. It opens with an assessment read from the scan: what holds up, what hurts (ranked by how much of the code it touches) and at most seven next steps, each with its numbers and a link to its evidence; each module panel opens with its role (foundation, hub, entry point and so on) and what is good, risky and next. Cohesion there is judged without imports into foundation modules, using `modules[].outgoing_to`. Below that is a zoomable map of modules as circles colored by health, holding their files grouped by folder, with module imports as curved arrows (red forbidden, amber unplanned, dashed type-only). Zoom in (wheel, pinch, `+`/`-`) and the files, their imports and their names appear. Hover or click a module for its importers and imports, a file or an import for its verdict and rule; the side panel lists findings by kind and draws suggested moves with their before and after numbers. Search, filters for forbidden, unplanned and cycle imports, and hiding busy modules keep large graphs readable. The page makes no network requests.

```sh
archgraph render graph.json -o graph.html
```

**diff** compares two scan documents (`kind: diff`): per-module metric pairs `[old, new]`, added and removed modules, new and resolved cycles, new and resolved forbidden imports, new unplanned imports.

```sh
archgraph diff before.json after.json
archgraph diff before.json after.json --fail-on new-forbidden,new-cycles --format markdown >> "$GITHUB_STEP_SUMMARY"
```

- `--fail-on <list>`: comma list of `new-forbidden`, `new-cycles`, `new-unplanned`; any other value is a usage error (exit 2). Exit 1 when any listed condition is in the diff, else 0. The document is printed either way.
  - A **new forbidden** (or **new unplanned**) import is a file pair `from -> to` that is forbidden (unplanned) in new and is missing, or has another verdict, in old.
  - A **new cycle** is an entry of `new_cycles`: a cycle in new whose level and member set appear in no cycle of old. Cycles are strongly connected components, so a cycle that grows (`a <-> b` becomes `a -> b -> c -> a`) is new, and its old member set is listed in `resolved_cycles`; a cycle that keeps its members while its imports change is not new. Both module cycles and file cycles inside one module count.
- `--format json|markdown` (default `json`): `markdown` prints a compact summary for a pull request or CI job summary instead of the document: a pass or fail headline naming the failed `--fail-on` conditions, new forbidden and unplanned imports (`file → file (rule)`), new and resolved cycles, a table of modules whose cohesion or instability moved by 0.05 or more (largest move first, at most 20 rows, then "and N more"), and the numbers of added and removed modules. Paths and module ids are backslash-escaped (backslash, `|`, backtick, `<`, `>`, `*`, `_`, `[`, `]`) and line breaks become spaces, since they come from the scanned repository.

`check`, `simulate` and `render` take `--rules layout.yaml` to replace the rules embedded in the scan.

## CI

The action at the repository root runs `report` on every push, and on pull requests compares HEAD with the pull request's base commit:

```yaml
name: architecture
on: [push, pull_request]
permissions:
  contents: read
jobs:
  archgraph:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          fetch-depth: 0
          persist-credentials: false
      # TypeScript/JavaScript only: dependency-cruiser 18.4.0 needs Node 22
      - uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0
        with:
          node-version: 22
      - uses: dogebonker/archgraph@v0.1.0
```

Never run it on `pull_request_target`. That event gets the base repository's write token and secrets, and scanning the pull request's code there hands a privileged job to files the pull request controls: its `tsconfig.json`, `layout.yaml` and file names (see the warning at the top). `pull_request` runs with a read-only token and no secrets for forks.

- **Push:** `report` with `layout.yaml` from `path` (or `rules`), its summary in the job summary. The step fails when any import is forbidden.
- **Pull request** (or with `base-ref`): `report` on HEAD, then a scan of the base commit, checked out with `git worktree` into `$RUNNER_TEMP/base`, with HEAD's rules file and the same options, then `diff --fail-on <fail-on> --format markdown` into the job summary. The step fails only on what the pull request adds: by default new forbidden imports and new cycles (`fail-on: new-forbidden,new-cycles,new-unplanned` for all three). `fetch-depth: 0` keeps the base commit in the checkout; with the default shallow checkout the action fetches that one commit from `origin`, which needs the repository to be public or the credentials kept.
- **Rules changes.** Both commits are judged by HEAD's rules, so a pull request that deletes a `forbid` rule and adds the import it forbade passes the diff. When the rules file differs from the base commit's (changed, added or removed), the job summary opens with a warning and a table of forbidden imports in base and HEAD under each commit's rules, and the output `rules-changed` is `true`. That is a signal, not a failure: `fail-on-rules-change: true` fails the step on it, for teams that review rule changes separately.
- It installs the archgraph release whose version is in the action's own `Cargo.toml`, so `@v0.1.0`, a commit SHA or a branch each run the archgraph they were cut from (`version` overrides). The collector goes into `$RUNNER_TEMP`, locked: dependency-cruiser 18.4.0 and typescript 6.0.3 with `npm ci` from [ci/tools/package-lock.json](ci/tools/package-lock.json) when `path` has `package.json` or `tsconfig.json`; grimp 3.17 in a venv with `pip install --only-binary :all: --require-hashes` from [ci/tools/requirements.txt](ci/tools/requirements.txt) when it has `pyproject.toml`, `setup.py` or `setup.cfg` (else set `language`), which needs CPython 3.10 to 3.14 as `python3` (`actions/setup-python`). Linux and macOS runners.
- Paths and messages from the scanned repository reach the log with workflow commands turned off (`::stop-commands::`), and the push summary's code fence is longer than any backtick run in it.
- `archgraph.html` and `archgraph.json` are uploaded as the artifact `archgraph-<job id>`, also when the step fails. In a matrix job, set `artifact-name` per entry (`artifact-name: archgraph-${{ matrix.os }}`), since artifact names must be unique in a run.

Inputs: `path` (default `.`), `rules`, `depth`, `language`, `package`, `exclude` (globs, one per line or comma-separated), `fail-on`, `fail-on-rules-change` (default `false`), `version`, `binary` (an archgraph executable to use instead of a release), `base-ref`, `artifact-name`. Outputs: `json` and `html` (paths), `forbidden` (count at HEAD), `new-forbidden`, `new-cycles` (counts) and `rules-changed`, the last three for pull requests only. All logic is in [ci/action.sh](ci/action.sh), which reads its inputs from `INPUT_*` environment variables, so it runs outside Actions too.

## Schema contract

Every document has `schema: "archgraph/v1"` and `kind: "scan" | "simulation" | "diff"`. The contract is [schema/archgraph.v1.schema.json](schema/archgraph.v1.schema.json) (JSON Schema draft 2020-12); every test output is validated against it.

- Consumers check `schema` and reject unknown major versions. They ignore unknown fields and unknown finding kinds (`kind` is an open string in the schema; known kinds have typed evidence).
- A scan has a top-level `summary`: forbidden, unplanned and cycle counts and each module's cohesion, Ca, Ce and instability. A finding's `simulation` is `{ "after": … }`: the same counts after the move, with `modules` listing only the modules whose metrics changed (`null` for a module the move empties). Compare it with the scan's `summary`.
- A scan document whose files, modules and edges disagree (an edge naming a file not in `files`, a file listed twice, a module its config does not give) is an input error, exit 2.
- Adding optional fields or new finding kinds keeps v1 (a diff's `new_unplanned` is such a field: always written now, absent from diffs of older versions). Renaming, removing or changing the meaning of a field bumps to v2.
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

Globs: `*` is one path segment, `**` any depth, `{a,b}` alternation. `<name>` captures one segment and binds it within a rule: `modules` path to id, `public` module to entries, and `forbid`/`allow` importer to target, so `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }` matches only imports within one module. `to`, `id` and `entries` may use only their rule's capture, and `layers` take no capture (write `*`). A file matching several layers takes the first. Unknown keys in `layout.yaml` are an error, and so is a control character (a line break, a tab) in any of its strings, since ids, reasons and globs are printed into logs and job summaries; rules embedded in a scan document may carry keys a newer archgraph added.

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

## Agent skills

Four skills in the open Agent Skills format, usable in Claude Code, Codex, Cursor and other harnesses (copy or symlink the folders into the harness's skills directory):

| Skill | What the agent does with it |
| --- | --- |
| [archgraph](skills/archgraph/SKILL.md) | Runs archgraph and reads its output: scan, check, simulate, render, diff, and what each finding means. |
| [archgraph-guard](skills/archgraph-guard/SKILL.md) | Before reporting a coding task done, compares the change with its base commit ([scripts/guard.sh](skills/archgraph-guard/scripts/guard.sh)) and resolves every new forbidden import, unplanned import or cycle. Optional Claude Code Stop hook. |
| [archgraph-fix](skills/archgraph-fix/SKILL.md) | Fixes one finding end to end: simulate, apply (move and rewrite imports, or cut), type check and test, re-scan, keep only if green. |
| [archgraph-rules](skills/archgraph-rules/SKILL.md) | Writes a `layout.yaml` from an approved architecture ([patterns](skills/archgraph-rules/patterns.md)), calibrates every rule against planted violations, and wires the CI gate. |

## License

MIT OR Apache-2.0, at your option.
