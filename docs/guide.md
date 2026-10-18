# archgraph guide

The full reference: install options, every command, the CI action, the rules file, the scan document contract, every finding kind and the limits. The [README](../README.md) is the short tour.

archgraph turns a repository's import graph into evidence for architecture decisions. It collects file-level imports with an existing tool (dependency-cruiser for TypeScript/JavaScript, grimp for Python, the Go toolchain's own `go list` and `go/types` for Go, `rust-analyzer` for Rust, the JDK compiler for Java), groups files into modules, and reports per-module coupling and cohesion, cycles with their smallest break set, rule verdicts for every import, and findings that propose file moves backed by a simulation of each move.

It has no parser of its own: imports come from dependency-cruiser and grimp, and for Python it reads importing files with the standard `ast` module, without running them, to tell deferred imports (inside functions or `TYPE_CHECKING` blocks) from load-time ones. For Go, imports and identifiers are resolved with Go's standard library (`go/types`) by an embedded helper run with your Go toolchain, which compiles nothing from the repository and runs no repository code. It writes into the scanned repository only when `report` or `history` runs without `--out-dir` (into `<root>/.archgraph/`).

**Scan only repositories you trust, or scan inside a sandbox.** For TypeScript/JavaScript, `scan` runs `npx --no-install depcruise` in the scanned repository, and npx resolves that repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`: the repository controls what runs. `--depcruise <path>` runs that executable instead, so the repository no longer picks the binary (depcruise still reads its `tsconfig.json`). The Python collector runs your interpreter with `-I` and adds the source root to `sys.path` last, so repository files cannot shadow the standard library or grimp; git runs with `core.fsmonitor=false`. The Go collector writes its helper to a temporary directory outside the repository and runs `go run` there, which compiles only the helper; the helper runs `go list -e -find -json ./...` in each module of the repository, which reads `go.mod` and the source files' package clauses and build constraints and builds, generates and runs nothing. Both run with `GOTOOLCHAIN=local` (a `toolchain` line in the repository's `go.mod` would otherwise make the go command download and run another toolchain; with it the line is ignored, and a `go` line newer than your Go fails the scan instead), `GOPROXY=off` (no network), `GOWORK=off` (a `go.work` in the repository is not read), `CGO_ENABLED=0` (no C toolchain) and `GOFLAGS=-mod=readonly` (replacing any `GOFLAGS` from your environment or `go env`, so no `-toolexec` or `-overlay` applies, and `go.mod` and `go.sum` are never written).

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
- Go: Go 1.21 or later on `PATH` (tested with 1.27.1, from [go.dev/dl](https://go.dev/dl)), or passed with `--go <path>`. Nothing is installed into the repository, and dependencies outside the repository are never loaded. The module cache is used only when the go command needs a module's dependency graph; without it, that module is read with `go/build` (see Go below).

- Rust: rustup with `rust-analyzer` and `rust-src` (`rustup component add rust-analyzer rust-src`), a workspace with `Cargo.lock`, and dependencies already fetched. Uses the active installed toolchain, never downloads one. `--rust-analyzer <path>` overrides the analyzer executable.
- Java: JDK 11 or later on `PATH`, or `--java <path>`. The helper uses [JavacTask parse/analyze](https://docs.oracle.com/en/java/javase/24/docs/api/jdk.compiler/com/sun/source/util/JavacTask.html), without generating bytecode or running Maven, Gradle or annotation processors.

## Commands

`scan`, `report`, `history` and `impact` run collectors. `infer-layout` drafts rules, `context` selects reading context, and `tour` prints a reading order from saved graphs without running collectors. `services` analyzes a separate declarative service-call inventory without scanning source code. Other analysis commands read a scan document and recompute modules, metrics, cycles, verdicts and findings from its files, edges and config with the same code, so their output agrees with the rules in effect.

**scan** writes the scan document (`kind: scan`).

```sh
archgraph scan . -o graph.json                        # language detected, modules = 2 folders below src/
archgraph scan . --rules layout.yaml -o graph.json    # with edge verdicts
archgraph scan . --cochange -o graph.json             # also read the last 300 commits (--cochange=N, N >= 1)
```

A module is a file's directory truncated to `--depth` segments below the source root (`src/` when present, else the root; for Python, the package directory). Each file directly in the source root is a module of its own, named by its file name with extension (`main.ts`, `errors.py`), so a package's lowest layer and its entry points never merge into one module (in Go the root's files are one package, so they form the module `.`). Imports between two such files are therefore cross-module: the same-module rule no longer allows them, so under `default: forbidden` they need an `allow` or `layers` rule. Scan documents written by commit 2bf5805 put these files in module `.` and fail `check`, `render` and `simulate` as inconsistent: rescan them. When the root has a `src/` directory, only `src/` is cruised (files outside it appear when `src/` imports them, with module ids prefixed `../`: `../lib`, and `../tool.ts` for a file directly in the root). Only `.ts/.tsx/.js/.jsx/.mjs/.cjs` and `.py` files are files; imports of stylesheets, JSON and other assets are dropped. Edges distinguish `runtime` (load-time imports), `lazy` (`import()` or imports inside functions), `type` (type-only imports) and `reexport` (structural re-exports). Type-only re-exports use `kind: type` with `is_reexport: true`. Metrics weight these kinds separately; rule verdicts consider every kind unless filtered. Cycles use runtime edges by default, with other cyclic components accounted for separately (see [Edge quality and migration](#edge-quality-and-migration)). For Python, an import that appears only under `if TYPE_CHECKING:` is `type`, and one that appears only inside function or method bodies is `lazy` (grimp's import lines, with the file parsed by the standard library's `ast`, never run). For TypeScript, `package_entries` lists the files a `package.json` in the scanned tree publishes through `exports`, `main`, `module` and `types`, mapped from build output (`dist/`, `build/`, `lib/`, `.js`, `.mjs`, `.cjs`, `.d.ts`) back to their sources; like barrels they are public entries. A composition root is a file nobody imports that imports 3 or more other modules (`app.ts`, `cli.ts`), a registrar a file with 5 or more imports into one module, or with imports into 5 or more sibling modules under one parent folder other than the root (code choosing one of many drivers): their imports are wiring, left out of sink-split consumers, unstable-dependency counts and two-way cuts.

Options: `--depth N` (default 2; for Go and Java, each package is a module unless it is given), `--exclude <glob>` (repeatable), `--include-tests`, `--include-generated`, `--no-default-excludes`, `--language typescript|python|go|rust|java`, `--depcruise <path>` (a dependency-cruiser executable to run instead of `npx --no-install depcruise`; exit 2 when the path does not exist), `--python <interp>`, `--package <pkg>`, `--python-path <directory>` (repeatable), `--go <path>` (the Go toolchain, default `go`), `--rules <layout.yaml>`, `--symbols` (collect public surface evidence), `--cochange[=N]`, `--collector-input <recorded.json>`. A shared exclusion policy drops tests, generated/build output, dependency/vendor directories and fixture/story/example content by default; `archgraph show-excludes` prints its actual patterns and override precedence. See [Exclusions and included tests](#exclusions-and-included-tests) for category flags and ignore-file support. dependency-cruiser runs with `tsPreCompilationDeps: "specify"`, from a config archgraph writes to the temporary directory (the repository's own config is not read): an import of types or interfaces only, written without `import type`, is kept and marked, and becomes `type`. When dependency-cruiser crashes in that mode, the scan runs it with and without pre-compilation dependencies and marks the difference; when that fails too, these imports stay `runtime` and `source.collector.type_marking` is `unavailable`, which the page and `report` mention.

**Go.** Detection counts `.go` files (not tests, outside `vendor/` and `testdata/`) only when the scan root has a `go.mod` or `go.work`; the language with the most files wins, and a tie keeps TypeScript, then Python, then Go. The source root is the scan root (`src/` is a folder like any other). **Modules are packages:** without `--depth`, each package (directory) is a module and the root's own files are the module `.` (`config.package_modules` is `true`); an explicit `--depth N` merges packages by folder as for other languages. Go rejects import cycles between packages, so a valid repository has no module cycle at the default, and since a package compiles as a unit, files of one directory form no file cycle with each other either: cycles appear only where `--depth` or rules `modules:` merge packages. Every `go.mod` below the root with a `module` line is a module of the scan (one without, or an empty one, is skipped with a warning; `--exclude` matching `<dir>/go.mod` skips it), so modules joined by `replace` or merely sitting side by side resolve to each other's files. A file is a file of its package when `go list` puts it in `GoFiles`, `CgoFiles`, `TestGoFiles` or `XTestGoFiles`, or in `IgnoredGoFiles` without needing the `ignore` build tag: every platform's files at once (`foo_linux.go` and `foo_windows.go`), whatever GOOS archgraph runs on, and cgo files although cgo is off. When `go list` fails in a module (it needs that module's dependency graph for some `replace` directives, which `GOPROXY=off` cannot fetch when the module cache lacks it), the scan warns and reads that module's packages with `go/build` instead. Each package is type-checked with `go/types` (the standard library through `go/importer`, the repository's own packages from source, other modules not at all), and an edge goes from a file to every file declaring something it uses: a function, type, variable or constant, a field, or a method, so `s.db.Get()` leads to the file declaring the field `db` and to the one declaring `Get`. A name declared in several platform files leads to all of them. A blank import (`import _ "x"`) leads to the package's `doc.go`, else `<dir>.go`, else its first file, picked among the files that are neither tests nor generated when there are any. An import none of whose uses resolved (they sit inside an expression whose type comes from a module that is not loaded) gives no edge; it counts in `source.collector.unresolved`, with imports that look local (under a scanned module path) but name no package in the repository. Every Go edge is `runtime`, including those between files of one package, which keep cohesion and `misplaced-file` meaningful. `source.collector.main_packages` lists the directories of `package main` packages and `bound_files` the files that cannot leave their package and still compile: a file holding a method of a type declared in another file, using an unexported name another file declares, or declaring a package-level name another file declares too (platform variants under exclusive build constraints, `notify_linux.go` and `notify_windows.go`), together with that file; a pair counts only when both files are scanned, so a method in an excluded test binds nothing. `misplaced-file`, `sink-split` and registrar moves never propose moving them, and `misplaced-file` never proposes moving a public file (outside `internal/`, not `package main`), which other modules may import, nor a file that has edges within its own package without using the target module itself: such a file is a piece of the library its package offers, whoever uses it now. `seam` is not reported for Go: file clusters would split packages, which Go cannot do, and package clusters would be compared with folders that are the packages themselves. `go_modules` lists the directories holding a `go.mod` (`.` for the root): no move is proposed into another Go module, which would change the file's module path and its module's published API, so a `misplaced-file` whose only destination lies in another module is not reported. `internals-reach` comes only from `public` rules, since the compiler already rejects imports into another tree's `internal/`. The default excludes add `**/*_test.go` (tests), test support (kept with `--include-tests`, and named in a warning): packages named `*testing` or `*testutil`, and each non-test file that uses package `testing` for more than `testing.Testing()` when no production file of another package uses it (a stub beside production code that others use stays) and, if it has variants for other platforms, every variant is such a file too; the other files of its package stay, `**/*.pb.go`, `**/*.pb.gw.go`, `**/zz_generated*.go` and every file with a `// Code generated ... DO NOT EDIT.` header (generated code); `vendor/`, `testdata/` and folders starting with `.` or `_` are never scanned. Reflection, `go:linkname`, `go:embed`, assembly and C are invisible.

**Rust.** Detection counts `.rs` files when the scan root has `Cargo.toml`, ignoring `target/`. Scans resolve cross-file workspace symbol references and trait implementations through `rust-analyzer scip`; local symbols stay out of the file graph, and recorded external imports use separate package nodes. Paths are relative to the scan root and ordinary folder depth/rules determine modules. `--rust-features all|default` chooses enabled Cargo features (default `all`). Build scripts and proc macros are disabled by default; `--rust-build-scripts` enables them and executes repository code. The collector uses cached dependencies offline and refuses repository toolchain/program overrides; missing dependencies fail with a fetch instruction. Generated references can be missing with build scripts disabled. SCIP can emit references inside inactive `#[cfg(test)]` code even when test definitions are omitted; inline test references can remain in architecture edges. `--include-tests` controls file exclusions only. Some `pub extern crate` facade links are absent from SCIP and therefore missing from the graph.

**Java.** Detection counts `.java` files, ignoring tests and build output. Without `--depth`, each source directory is a module (`config.package_modules: true`); an explicit depth groups folders from the scan root. References resolve to declaring files, including same-package, qualified, static and inherited references. Edges are `runtime`. The compiler receives only selected sources, no external classpath or annotation processors. Missing external libraries, unsupported syntax or module layouts can leave a partial graph; `source.collector.unresolved` retains compiler error diagnostics and produces a warning; import-statement accounting is separate. Default excludes add `target/`, `out/`, `test/`, `*Test.java` and `*Tests.java`; `--include-tests` keeps tests. Hidden directories and node_modules are never traversed. Kotlin is not supported.

**report** is the one command for people: it scans with the same options as `scan`, applies rules from `--rules`, else from `<root>/layout.yaml` when it exists (it prints which file it used), writes `archgraph.json` (the scan document) and `archgraph.html` (the `render` page) into `--out-dir` (default `<root>/.archgraph/`; add it to `.gitignore`; it refuses to write when that folder or either file is a symbolic link, which a scanned repository could plant to aim the write elsewhere), and prints a short summary built from the scan document: files, modules and imports; with rules, forbidden and unplanned counts by rule; module cycles with the largest one's size and the lightest two-way cut (from the cycles' `two_way`); findings by kind with counts and up to three subjects each; then the two paths, the page last. Headings are bold only when stdout is a terminal and `NO_COLOR` is unset. Exit codes are those of `check`: 1 when any import is forbidden, 0 otherwise, 2 on errors.

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
    src/domains/checkout/**
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

**render** writes one self-contained HTML page. It opens with an assessment read from the scan: what holds up, what hurts (ranked by how much of the code it touches) and at most seven next steps, each with its numbers and a link to its evidence; each module panel opens with its role (foundation, hub, entry point and so on) and what is good, risky and next. Cohesion there is judged without imports into foundation modules, using `modules[].outgoing_to`. Below that is a zoomable map of modules as circles whose ring shows what archgraph checked (red: on either end of a forbidden import, or holding an import to cut or a registrar move for a module cycle; amber: the other members of a module cycle, or named by a finding; grey: nothing found), holding their files grouped by folder, with module imports as curved arrows (red forbidden, amber unplanned, dashed type-only). Zoom in (wheel, pinch, `+`/`-`) and the files, their imports and their names appear. Hover or click a module for its importers and imports, a file or an import for its verdict and rule; the side panel lists findings by kind and draws suggested moves with their before and after numbers. Search, filters for forbidden, unplanned and cycle imports, and hiding busy modules keep large graphs readable. The page makes no network requests.

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

## Declared cross-service edges

This first service-level milestone uses declarations. It does not discover OpenAPI clients, protobuf stubs or queue strings. Teams and external tools can feed the same versioned YAML/JSON manifest; no plugin executable is loaded or run.

```sh
archgraph services                              # read ./services.yaml, print Markdown
archgraph services examples/services.yaml --format json
archgraph services services.yaml --output services-report.json
```

```yaml
version: 1
services: [frontend-bff, billing, notifications, db-service]
edges:
  - from: frontend-bff
    to: billing
    kind: http
    confidence: high
    operation: POST /invoices
    source: {file: apps/frontend/invoices.ts, line: 18}
  - from: billing
    to: frontend-bff
    kind: grpc
    confidence: high
    operation: Frontend.GetCustomer
    source: {file: services/billing/customer.go, line: 42}
  - from: billing
    to: notifications
    kind: queue
    confidence: low
    operation: invoice.created
    source: {file: services/billing/events.py, line: 27}
  - from: frontend-bff
    to: db-service
    kind: http
    source: {file: apps/frontend/customer.ts, line: 9}
forbid:
  - id: bff-no-direct-storage
    from: frontend-*
    to: db-service
    kinds: [http, grpc]             # omitted: every transport
    reason: Use the service API.
```

Every call requires a known caller/callee, transport (`http`, `grpc`, `queue`) and source file with a positive line number. Confidence is `high`, `medium` or `low`; omitted confidence defaults to **low**. `operation` is an optional opaque route, RPC method or topic label. Queue edges point **publisher → consumer**, not consumer → publisher. Source locations can identify code, configuration or a declaration; they are required, user-supplied evidence and are **not read or verified**. This also permits inventories spanning several repositories. No endpoint is contacted.

The report includes all calls with confidence and file:line evidence, strongly connected service groups, distinct-service fan-in/out, instability `fan_out / (fan_in + fan_out)`, and forbid matches. Isolated instability is `null`. Parallel HTTP/gRPC/queue calls remain separate evidence but count once toward neighbor metrics. Self-calls remain visible while being excluded from cross-service cycles and coupling. A hub has at least two distinct other callers; hubs rank by fan-in, then ID. Cycle confidence is the lowest recorded confidence among its internal, non-self calls. Low-confidence edges participate rather than silently disappear.

Forbid rules use existing glob semantics: `*` stays within one namespace segment, `**` crosses `/`; captures such as `<name>` are not supported. The first matching rule of an applicable transport wins. An edge marked `allowed` means **no declared forbid matched**, not approval of architectural intent. Rule IDs must be unique. Unknown fields, duplicate services/exact call records, unknown endpoints, invalid protocols/confidence/globs, control characters and malformed evidence are errors. Service IDs and evidence paths use normalized relative names; evidence paths may contain spaces. No absolute, drive-letter, backslash, empty or dot path segments are accepted.

JSON reports use `schema: archgraph/v1.1`, `kind: services`. Their transport-valued edges have their own schema definition; source-import kinds are unchanged. [Manifest schema](../schema/services.v1.schema.json) validates the input structure, while [the v1.1 schema](../schema/archgraph.v1.1.schema.json) validates reports. Cross-references and semantic constraints are additionally checked by Rust. Array indices in cycle and forbidden-call evidence are zero-based; Markdown numbers calls from one. Report source metadata says `collector: declarative`, `complete: false`.

This is a standalone report type. Import commands such as `check`, `render`, `simulate`, `tour` and `context` do not consume service reports. Service-map integration and automatic collectors are separate milestones; there is no legacy-v1 export. Valid reports exit 0 even when they contain cycles or forbidden calls; invalid declarations exit 2. JSON may be saved with `--output` while Markdown is printed; the output path must differ from the input manifest. All declared transports participate; file `--granularity` does not apply.

**Interpretation:** a call cycle is communication-coupling evidence. It does not prove a distributed deadlock or a deployment-order failure, especially when queues are asynchronous. The inventory can be incomplete or wrong. Review the displayed confidence and evidence before acting; this mode makes no completeness claim.

## Layout inference

`infer-layout` drafts rules from the architecture observed in a saved v1 or v1.1 graph. **The result is a draft for review, not a statement of architectural intent.** Folder names do not prove intent, and missing imports or excluded files weaken evidence that a dependency is absent.

```sh
archgraph infer-layout graph.json                       # print annotated YAML
archgraph infer-layout graph.json --apply               # create ./layout.yaml
archgraph infer-layout graph.json --apply layout.draft.yaml
archgraph check graph.json --rules layout.draft.yaml     # after reviewing/editing
```

Printing is the default and writes no files. `--apply` creates a new file and refuses to overwrite an existing file or symbolic link. Inference does not run collectors, inspect source, or change the graph.

The command condenses module SCCs and assigns each component its longest distance from an entrypoint in the condensation DAG. Evidence comments show the layers top to bottom: layer 0 has no incoming dependencies; deeper layers contain dependencies. Leaves can occur at different depths. The executable `layers` list remains **foundation-first**, as required by the existing rules format. Patterns at the same depth share one layer, and patterns come from actual file membership rather than module names. Cyclic module groups appear in `# unresolved: cycle between ...` comments and receive no layer or forbid proposals.

Inference uses **all import kinds**, because existing layer rules apply to all kinds. Consequently, a type/lazy/re-export cycle can remain unresolved here even when the default runtime cycle report is acyclic. Dependencies use their written boundary (`original_to` for resolved barrel imports), matching `check`. Tests and externals are outside the internal layering graph; written targets absent from the scanned files are counted as omitted evidence. External package policies use the separate import ledger described below.

Sparse-boundary proposals require at least two files on each side and **100 potential file pairs** by default. Density is distinct observed importing-file/target-file pairs divided by source files × target files, across all kinds. A density at or below **0.01** qualifies. Comments give the evidence and list existing violating dependencies with source lines when available: adopting such a rule means reviewing and fixing those imports. Proposals already implied by layer ordering are omitted. At most **20** forbid proposals are printed. External-free layer proposals come first; the remaining internal proposals are ordered by density, potential pairs and module IDs. Both share `--max-rules`, and comments report omitted proposals.

```sh
archgraph infer-layout graph.json --min-potential-edges 500 --max-density 0.005
archgraph infer-layout graph.json --max-rules 0           # layers only
```

When import accounting is present and at least one known external package exists elsewhere in the scan, inference also proposes `to: "pkg:**"` for an acyclic layer with **zero external imports and zero unresolved imports**. It requires `layer file count × distinct package identities observed repo-wide` to meet `--min-potential-edges` (default 100). Comments show the factors; these are file/package opportunities, not measured missing edges. No package rule is inferred from an unavailable ledger or unresolved absence evidence.

These thresholds limit over-fitting; zero observed imports still do not establish an intended boundary. Review the draft before making it a CI policy. No scan-schema or rules-format migration is required.

## Live what-if in the map

```sh
archgraph render graph.json -o graph.html
```

Open the HTML, enable **What-if**, zoom until files appear, and drag a file into a different module disc. The target module's common physical directory becomes the proposed destination folder; confirm it in the panel, especially for custom modules spanning several folders. The source basename is retained. A labeled file/folder form provides the same preview with keyboard or touch; it also accepts new folders. Every proposal starts from the original graph, rather than chaining moves.

The panel compares total cycles (module and within-module file SCCs), largest module cycle, distinct two-way module pairs, forbidden/unplanned imports, and changed module instability. Map links and health rings show preview cycle/rule evidence. Original findings are hidden until preview mode closes: heuristics such as seams, placement, ownership, cut sets, public surfaces and package readiness are **not** recomputed in JS. A new module appears in results; module disc positions stay fixed. Clear restores original paths, positions, counts and health. Preview moves are not persisted to browser layout storage and never edit the repository. Ordinary dragging outside What-if still arranges the map.

A preview rewrites the path and import endpoints, including written barrel destinations. Module assignment, runtime/all-kind SCCs, test exclusions, weighted barrel-aware metrics, and supported glob/capture rules follow the Rust pipeline. Rules use their normal precedence, including kind filters, reachable forbids, public entries, strict entrypoints and layers. A residual SCC contained in a previous SCC is not called new cycle membership; a new or expanded component is flagged. This membership comparison is a preview explanation, not a CI verdict.

**Rust remains authoritative.** Grab-bag refinement, legacy boundary assumptions, unknown schema/rule fields and unsupported glob syntax visibly mark a limited/raw preview and withhold the no-new-cycle assurance. Dynamic/unresolved imports remain invisible. The preview does not rewrite import strings, check build compatibility, recompute symbol surfaces or execute code. Verify the displayed, shell-quoted `archgraph simulate` command before using the Git snippet; set its graph JSON path and run from the scanned root. Moving files can still require import and build-configuration edits. Clipboard-denied and local-file environments fall back to selecting the command for manual copying.

The HTML remains self-contained and offline, including maps embedded in history timelines. No runtime package or schema migration is required. A fixture test rebuilds canonical Rust results and compares them with [the committed parity cases](../tests/fixtures/whatif-parity.json); Node tests run those cases against the exact JS embedded in reports. Regenerate intentionally with `UPDATE_WHATIF_FIXTURES=1 cargo test --test whatif`, then run `node --test tests/whatif-parity.test.cjs tests/whatif-ui.test.cjs`. Grab-bag fixtures verify the limitation warning rather than claim raw SCC parity with refined Rust cycles.

### Record the demo

The [capture script](../scripts/record-whatif-demo.cjs) uses a development-only Playwright/Chromium installation and `img2webp` (WebP tools); these are not archgraph dependencies. It drives a real browser mouse drag, clicks Copy, clears the preview, and asserts the browser has no errors and original state returns. It does not paint or fabricate the result.

```sh
python3 - <<'PY' > /tmp/whatif-demo.json
import json
cases = json.load(open('tests/fixtures/whatif-parity.json'))
print(json.dumps(next(c['document'] for c in cases if c['name'] == 'cut-module-cycle')))
PY
archgraph render /tmp/whatif-demo.json -o /tmp/whatif-demo.html
node scripts/record-whatif-demo.cjs /tmp/whatif-demo.html demo.webp a/three.ts b
```

Set `PLAYWRIGHT_MODULE`, `CHROMIUM_EXECUTABLE` or `IMG2WEBP` to existing tool paths when needed. The checked-in [animation](assets/whatif-demo.webp) records the `cut-module-cycle` Rust fixture: moving `a/three.ts` into `b` removes its two-module cycle. The script also writes a PNG poster. The asset demonstrates interaction, not a measured result on an OSS repository.

## Reading order

```sh
archgraph tour                                      # .archgraph/archgraph.json
archgraph tour graph.json > TOUR.md                  # dependencies before their importers
archgraph tour graph.json --reverse                 # entrypoints before dependencies
archgraph tour graph.json --agent                   # include saved export names and locations
archgraph tour graph.json --all-kinds                # include type, lazy and re-export imports
```

`tour` prints Markdown from a saved v1 or v1.1 scan without collecting imports, reading source bodies, or writing files. No schema migration or new dependencies are required. Runtime edges define the reading graph by default; a saved `config.all_kinds` or `--all-kinds` includes all recorded kinds. Dependencies use resolved semantic destinations, so an import through a resolved barrel points at its defining module. Included test files and external packages do not become stops or affect counts.

Each strongly connected component becomes one stop. Leaf components come first; a component's layer is its longest dependency distance to a leaf. Within a layer, stops sort by descending fan-in, then module name. `--reverse` reverses layers, retaining that fan-in order. This starts at dependency roots; it does not identify executable `main` functions or infer domain/application roles from folder names. Disconnected modules remain visible.

Every stop explains how many distinct outside modules import it and how many it imports. Repeated file imports count once. Cyclic stops say to read their modules together and start at the member with the fewest distinct outbound modules, including cycle peers; ties use module name. The stop's aggregate counts exclude its own members.

`--agent` attaches saved public surface export names and declaring files. Without a surface report it falls back to the saved symbol index; without either, it reports unavailable evidence and suggests rescanning with `--symbols`. Partial coverage is labeled. These summaries contain names, not full type signatures or bodies. `--granularity file` also works: export evidence follows its declaring file. The tour reflects only the saved graph; unresolved imports, exclusions and dynamic dependencies can hide relationships.

## Context pruning

```sh
archgraph scan . --symbols -o .archgraph/archgraph.json
archgraph context --task "Fix billing tax rounding" --files src/billing/tax.ts --budget 40k
archgraph context --graph graph.json --root . --task "Fix billing tax rounding" --files src/billing/tax.ts --symbols TaxRate --budget 20k --depth 3 --format markdown
```

`context` reads v1 or v1.1 scans and prints a standalone v1.1 JSON reading plan (`kind: context`); `--format markdown` prints a pasteable plan. It does not modify files or run collectors. Use the source root corresponding to the saved graph. Current files supply sizes and layouts; graph dependencies and export names reflect the saved scan, so rescan after structural changes. JSON contains ordered `entries` with `tier`, `path`, `reason`, `tokens`, `mode`, optional inline `content`, `depth` and `score`, plus omissions and limitations. Body entries name files for the agent to open; they do not inline their bodies.

Explicit files seed the search; task words match paths, and `--symbols` matches saved exported names. At most 20 automatic seeds are added. The selector follows imports and importers (every recorded kind) to `--depth` (default 3), using inverse depth, same-module, and available saved hotspot/co-change evidence. Explicit files rank first. Dependency directories are excluded; tests, generated files and support content are downranked unless explicitly named. It never opens an `index.ts` body or a recorded pure barrel: recorded direct consumers can redirect a barrel seed to the definitions they actually use, without expanding every re-export.

Root and seed-ancestor `layout.yaml` files and the saved graph's embedded rules come first. Nested layouts are constraint evidence, not proof that the saved graph enforced them. Different on-disk and saved rules are both labeled. Constraints that exceed the budget cause an error with the required estimate. Then `must-read` covers seeds and distance 1, `should-read` covers same-module distance 2, and `skim` uses export names for other distance-2 and deeper files. Oversized bodies fall back to saved export names. These are names, **not full type signatures**; without symbol/surface evidence such summaries are omitted explicitly.

Budgets accept positive integers or decimal suffixes `k` (1000) and `m` (1000000). Estimated tokens use UTF-8 bytes / 4, rounded up, including task/framing, mandatory constraints, selected bodies, and inline export summaries. Greedy packing guarantees `used_tokens <= budget` for this estimate, not an exact model tokenizer limit or the serialized JSON envelope. Large required files may fit only as exports or be omitted. Missing dynamic dependencies and stale graph evidence still require judgment.

The Python 3 hook examples require a POSIX host. The project skill is [archgraph-context](../skills/archgraph-context/SKILL.md). Editor integration is opt-in: merge [Claude settings](../integrations/claude-context.settings.json) into project `.claude/settings.json`, or [Cursor hooks](../integrations/cursor-context.hooks.json) into `.cursor/hooks.json` and copy [the rule](../integrations/cursor-context.mdc) to `.cursor/rules/archgraph-context.mdc`. Keep `integrations/context-hook.py` in the project. Install the skill in the editor's project skill directory when desired. Nothing is installed automatically.

Claude `UserPromptSubmit` receives a compact, constraints-first context handoff and artifact paths; it must read the plan. Cursor `beforeSubmitPrompt` prepares artifacts but cannot inject context. Its asynchronous `sessionStart` guidance is best effort; the always-apply rule reruns the CLI when an artifact's workspace, task, generation or seeds cannot be verified. Hook subprocesses time out after eight seconds and fail softly; failure invalidates previous context for that conversation. `ARCHGRAPH_BINARY`, `ARCHGRAPH_CONTEXT_GRAPH`, `ARCHGRAPH_CONTEXT_BUDGET`, and optional `ARCHGRAPH_CONTEXT_FILES` (JSON array for Claude seeds) configure the adapter. Missing graphs do not block normal editing. See [the preregistered PR benchmark](../benchmarks/CONTEXT.md) for recall methodology and results.

## Package splits

`package-splits` ranks physical folder subtrees that could become packages, including folders spanning several graph modules. **Import-level ready** describes the observed dependency boundary; it does not mean extraction, compilation, packaging, or deployment has been verified.

```sh
archgraph package-splits graph.json --root .
archgraph package-splits graph.json --root . --format json
archgraph package-splits graph.json --root . --output packages.json
archgraph render packages.json -o packages.html
```

Each candidate needs at least two production files, a non-barrel implementation, an internal dependency, and at least one importing production module outside the folder. The score is **distinct importing modules × production files**. Weighted cohesion must be at least **0.8** by default: canonical internal dependencies divided by internal plus unextracted outgoing dependencies. Known dependencies on separate existing packages are externalized for this factor. Written barrel paths and import-ledger destinations still constrain the physical boundary; resolving a symbol into the folder cannot hide a dependency on an outside facade.

- **Free:** no recorded dependency to cut; outgoing dependencies may target proven separate packages. A subfolder's dependency on its own enclosing package is not a free external escape.
- **Cheap:** at most **3** distinct source/target file dependencies to cut by default. Every cut includes its kind and source line when available. These are dependency counts, not a prediction of how many edits are needed.
- **Needs verification:** missing import ledgers, unresolved/namespace imports, low-confidence edges, unscanned local bodies, ambiguous external aliases, unverified workspace metadata, or a package-boundary cycle prevent a readiness claim. A candidate can have a free observed boundary and still need verification.

All import kinds constrain the boundary, including lazy/type imports. Included tests inside the folder contribute boundary dependencies, while production files determine size, cohesion and fan-in. The cycle simulation removes the candidate from the remainder, compares SCC membership with the original graph, and also checks the collapsed package boundary after the listed cuts. Counts can increase when one SCC splits; that alone is not a new cycle. Simulation uses runtime imports by default, or all kinds with `--all-kinds`, and excludes test endpoints. Its SCC counts are for the physical dependency graph, before grab-bag reclassification.

```sh
archgraph package-splits graph.json --root . --max-cuts 0  # free boundaries only
archgraph package-splits graph.json --root . --min-cohesion 0.9 --limit 10
```

`--root` is the **directory originally scanned**, which may be a subfolder of the Git repository. Static inspection supports npm/Yarn workspace declarations, pnpm patterns and exclusions, and explicit Nx project metadata. A [configured Nx project](https://nx.dev/docs/reference/project-configuration) is tracked separately from a manifest-backed package; workspace layout conventions alone are not a package inventory. Existing package/project roots are not suggested again, and ancestors spanning those roots are skipped. A smaller folder inside an existing package can still be a candidate. Nx plugins and package-manager commands are never executed. Unsupported, skipped or incomplete metadata is reported.

New scans record `source.scan_root`, a Git-relative directory, to distinguish sibling roots at the same commit. A mismatched root/revision is rejected. Older graphs, non-Git scans, dirty checkouts, or omitted `--root` without saved metadata cannot establish verified workspace boundaries; results remain available with conservative labels. Keep generated graphs in an ignored output directory when using a clean checkout. Saved workspace evidence is reused when replaying an enriched graph, and candidate scores/cuts are recomputed from its current files and edges.

The map adds dashed island halos without replacing architecture health rings. Panels show free/cheap/unverified status, score factors, all cuts, simulation results and partial-module coverage. Nested suggestions can overlap; review them as alternatives rather than simultaneous extractions. Build configuration, package exports, consumer paths, excluded/shared test utilities, generated code, and environment assumptions remain outside this proof.

No scan-version migration is required: `package_splits` and `source.scan_root` are optional additions to v1.1. Standalone report JSON uses the new `package-splits` kind and cannot be exported with `--legacy-v1`; legacy scan input remains supported.

## Exclusions and included tests

```sh
archgraph show-excludes
archgraph scan . --include-tests --include-generated -o graph.json
```

The defaults cover test names/directories (`*_test.go`, `*.test.*`, `*.spec.*`, `test_*.py`, `*_test.py`, `test/`, `tests/`, `__tests__/`, `conftest.py`), generated code (`*.generated.*`, `*_pb.*`, `*.pb.go` and language-specific generated files), build/dependency directories (`dist/`, `build/`, `node_modules/`, `vendor/`, `.venv/`), and fixtures, stories and examples. Use `show-excludes` for the complete list. Git ignore rules, including nested `.gitignore` files, and `.archgraphignore` also filter files; `.archgraphignore` uses gitignore syntax, including directory patterns, comments and negation.

Selection precedence is explicit: CLI `--exclude` and layout `exclude` win; layout `include` can override built-in defaults and ignore-file exclusions. `--include-tests` disables the default test category and `--include-generated` disables the generated category; neither cancels explicit excludes or ignore rules. Fixtures, stories and examples are a separate support category; restore them with layout `include` or `--no-default-excludes`, not `--include-generated`. `--no-default-excludes` disables all built-in defaults. Layout paths are relative to the scanned root:

```yaml
version: 1
exclude: ["src/legacy/**"]
include: ["tests/architecture/**", "examples/reference.py"]
```

Included test files carry `class: tests` and belong to separate test modules. Their imports remain available as evidence, but test nodes never participate in runtime or deferred cycle membership. `test_coverage` reports production module counts, how many have direct importing tests and `without_importing_tests`; this is import coverage, not execution or assertion coverage. Python inclusion discovers test files and support files under test directories outside the selected package, using the same AST pass without executing test code.

## Import resolution accounting

```sh
archgraph report .
archgraph unresolved .archgraph/archgraph.json
archgraph unresolved graph.json --json
archgraph scan . --language python --package app --python-path lib --python-path shared -o graph.json
```

Fresh collector output carries an `imports` ledger independent of symbol-reference edges. An explicit module specifier is counted once per occurrence: `from x import a, b` is one clause, while `import x, y` names two specifiers. Repeated imports on different lines remain separate; a single Go import can produce many reference edges. `report` prints the total imports, resolved count and percentage, and unresolved count. Each unresolved entry retains the importing file, line, raw specifier, kind and `reason`: `alias` (unresolved configured alias), `missing` (no proven destination), `dynamic` (computed specifier), or `native` (unsupported native source resolution). A known local namespace package can resolve without a file edge and carries its root-relative directory as `namespace`.

Resolution and scan selection are separate. An import whose destination is known but excluded still counts as resolved; its target remains in the ledger even though it contributes no internal graph edge. Unknown bare package names are not automatically treated as externals. Saved graphs and old collector recordings without an import ledger have unavailable accounting rather than an invented 100% resolution rate; rescan with the live collector to obtain a list.

TypeScript/JavaScript resolution considers dependency-cruiser evidence, nearest package `tsconfig` paths and `baseUrl`, package `exports`/`imports`, and declared workspace packages. Python resolves relative imports, `src/` layouts, namespace packages and additional `--python-path` roots. Relative search roots are interpreted from the scan root; included local modules retain root-relative file paths. Python proves installed/standard-library or explicitly supplied external packages by inspecting module specs, without importing project package initializers. A Python native extension inside the scanned tree stays an explicit `native` unresolved item when its source cannot be analyzed. Dynamic namespace mutation and unresolved configuration cases remain visible backlog, not guessed edges.

Configured aliases use the nearest package configuration, even when a root-configured collector found a different destination. Package export conditions follow declared order for `node`, `import` or `require`, and `default`; custom development/source conditions are not enabled implicitly. Mixed type/value clauses retain per-binding evidence. A binding is refined to `type` only when its actual exported declaration and erased use are proved; unknown uses retain runtime weight.

Java counts explicit compiler-AST imports; compiler diagnostics prevent missing wildcard packages from becoming external proofs. Rust counts source `use`/`extern crate` statements with SCIP evidence. Macro-expanded imports and grouped Rust imports spanning several external packages remain bounded limitations; unsupported bindings are explicit `native` items. Older native recordings without occurrence metadata retain unavailable coverage.

Known external packages are graph nodes in `external_nodes`, with `external: true`, a package name and a stable id. Their links live in `external_edges`; ordinary `files` and `edges` remain the internal architecture graph. Externals never enter internal cycle, coupling or cohesion calculations. Consumers can display or inspect the separate external graph without treating a dependency package as a source module.

## Third-party exposure per module

Package rules use the collector's known external identities, separate from internal folder edges. Use `pkg:<glob>` or a named `category:<name>` as the `to` selector of `forbid` or `allow`; `from` remains a source-file glob. Categories contain package globs, and an unknown or empty category is an input error.

```yaml
version: 1
pkg-category:
  orm: [typeorm, prisma, "@prisma/client", sqlalchemy, "gorm.io/gorm{,/**}"]
  http: [axios, httpx, requests, "net/http"]
external_exposure:
  max_percent: 80                 # default; advisory only
forbid:
  - { id: domain-no-orm, from: "src/domain/**", to: "category:orm" }
  - { id: domain-no-typeorm, from: "src/domain/**", to: "pkg:typeorm", kinds: [runtime] }
  - { id: domain-no-transitive-orm, from: "src/domain/**", to: "category:orm", reachable: true }
```

```sh
archgraph scan . --rules layout.yaml -o graph.json
archgraph check graph.json
archgraph report . --rules layout.yaml
archgraph render graph.json -o graph.html
```

Match identities actually present in `external_nodes[].package`: package roots and subpaths vary by collector and ecosystem. For Go, use full import paths such as `gorm.io/gorm`; `gorm.io/gorm{,/**}` covers that root and its subpackages. `gorm` is not an inferred alias. Standard-library packages count when the collector identifies them as external; external does not mean downloaded or third-party licensed code.

Explicit forbids take precedence over allows, then the configured default applies. Folder selectors such as `to: "**"` match internal files only; use `pkg:**` for all external packages. Externals skip same-module allowances, layers and entrypoint checks. `kinds` filters apply as usual, including every hop of a `reachable: true` path from an internal file to a package. `check`, diff and simulation include package verdicts; known external nodes remain outside internal SCCs, coupling and cohesion. A saved graph without an import ledger cannot enforce an explicit package policy: rescan rather than treating missing evidence as a pass.

`report` shows each module's distinct external packages, external versus total import **occurrences**, their weights and both percentages. The optional scan `external_exposure` block retains these factors and a per-package breakdown. All recorded occurrences count, including unresolved imports in the denominator; unresolved imports can hide additional external usage, so the external share is a lower bound. Weight totals use the same `edge_weights` as other metrics (runtime 1, lazy 0.3, type 0.1, reexport 0 by default). A zero denominator yields an unavailable percentage, not zero. Old graphs without accounting show unavailable exposure.

An advisory appears only when the raw external occurrence percentage is **strictly greater than** `external_exposure.max_percent` (default **80**, range 0–100). It does not fail `check`. A high share may describe an intentional adapter or wrapper; review intent before calling it a leaking boundary.

The map places recorded external packages in an outer ring. Select or search a package to inspect its importing files, source lines, kinds, weights, categories and rule verdicts; module panels show exposure factors. External links appear on selection to limit clutter, and the assessment provides keyboard-accessible package links. Live what-if previews hide the ring while a result is active and warn that external verdicts and exposure are not recomputed in JavaScript. Run the copied Rust `simulate` command for authoritative results. Missing collector edges still limit every view; this is not a complete dependency or vulnerability inventory.

## Grab-bags and granularity

A folder can create a module cycle even when its helper files have no cyclic dependency. Grab-bag detection uses measured structure rather than folder names: a candidate needs at least two files, at least one non-barrel production file, defined weighted cohesion **below 0.2**, weighted fan-in **at least 5**, and raw internal edges per file **below 0.5**. Undefined cohesion is not treated as zero. Configure the thresholds in `layout.yaml`:

```yaml
version: 1
grabbag:
  cohesion_max: 0.2
  fan_in_min: 5.0
  internal_edges_per_file_max: 0.5
```

For a cycle containing a candidate, archgraph splits that folder into per-file units temporarily and verifies which cycles survive. A `cycle-via-grabbag` finding is emitted only when this removes or shrinks the original module cycle. Its evidence lists original members, candidate factors, involved files/imports and any remaining cycles, with a suggestion to split the folder. Surviving cycles and their cuts remain ordinary cycle findings, using edges verified within the split components. If overlapping surviving components map back into the full original module cycle, archgraph conservatively retains the original cycle instead of claiming the folder split fixed it.

Test modules are never candidates, and test edges never enter these cycle probes. With `--include-tests`, visible weighted fan-in can still include test consumers, consistently with ordinary metric calculations; candidate selection alone is not proof of a cycle artifact. The verified split is the deciding evidence.

Choose graph grouping explicitly with the global option:

```sh
archgraph --granularity file scan . -o files.json
archgraph --granularity folder render files.json -o folders.html
```

`folder` is the default grouping by depth or layout `modules`; `file` makes each source file its own graph module and overrides those grouping choices. Folder-based layout policy boundaries still govern rule verdicts, so switching the view does not weaken a boundary. The choice is saved in the scan config and replayed by historical probes and caches. File mode already exposes individual units and does not run the temporary grab-bag split again.

## Ownership

`scan` and `report` automatically read the first existing `.github/CODEOWNERS`, `CODEOWNERS`, or `docs/CODEOWNERS` at the Git repository root. Outside Git, the scan root is used. Scanning a subdirectory keeps repository-relative matching, so `/packages/server/ @org/server` still applies when scanning `packages/server`.

```text
* @org/platform
/src/payments/ @org/payments @org/security
/src/catalog/ @org/catalog
/src/catalog/payment.ts @org/payments
/src/generated/
```

Last matching rule wins; an ownerless rule clears ownership. Matching is case-sensitive and supports `*`, `**`, `?`, directory patterns, comments, and escaped spaces. Unsupported negation, ranges, braces and escaped `#` patterns are skipped with line warnings. Files over 3 MiB, invalid UTF-8, non-regular files and links outside the repository are refused. Local parsing checks syntax, not GitHub account existence or repository permissions. See [GitHub's CODEOWNERS contract](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners).

The report shows declared responsibility for each file and module. Shared files count once for each owner; totals can therefore exceed the number of files. A primary module owner owns a strict majority of **all** its files, including unowned files in the denominator. Ties and sparse coverage do not invent a primary owner.

Ownership qualifies the existing `misplaced-file` finding. It supports a destination when the file shares a primary target owner and does not share a primary current owner; it conflicts when the primary target owners or the actual destination-path owners are known and disjoint from the file's owners. Conflicting findings keep their dependency evidence but withhold an automatic move. Shared and unknown ownership preserve the structural suggestion. The existing dependency, rule, cycle and simulated-cohesion checks still apply. Ownership alone does not produce a move or prove where code belongs. Registrar and sink-split moves also withhold destinations that would transfer a known file to disjoint declared owners.

`config.codeowners` embeds the matching rules, scan-root prefix and diagnostics. File `owners` and module `ownership` are recomputed by `check`, `render` and `simulate`; moving a path can change its declared owners. History uses each commit's own CODEOWNERS. Older scans without ownership retain their original behavior. Replacing embedded rules with `--rules` discards cached hotspot/provenance context because it was judged under the previous boundaries and policy.

## Architecture bisect

```sh
archgraph bisect graph.json --root path/to/repo --finding 0 --format markdown
archgraph bisect graph.json --root path/to/repo --edge src/a.ts src/b.ts --from v1.0
archgraph bisect graph.json --root path/to/repo --finding 0 --github
```

`--finding` is the zero-based index in the saved graph; the first cycle finding is selected by default. `--edge` selects a forbidden import. Optional collector options such as `--depcruise`, `--python` and `--java` select the tooling. Probes preserve the graph's module boundaries, exclusion mode, selected Python package/Rust features and embedded rules, so a forbidden-edge introduction is judged under that fixed policy, not a claim that the rule existed historically.

The engine tries rename-following pickaxe candidates for supported import syntax, including imports added to existing files. It verifies graph states before attributing a change. Dynamic or aliased imports can fall back to graph bisect. Verified findings introduced within seven days are marked as recent. A verified result includes author name/email, author/commit dates, age, the introducing commit, and a closing import when that import itself appeared at the transition. Saved Rust graphs that enabled build scripts still require an explicit `--rust-build-scripts` for new probes. PR numbers are inferred from commit messages; `--github` optionally queries GitHub through `gh`. Author names are never converted into invented GitHub handles.

Searches use first-parent history, default to two years and at most 10,000 commits, and have a `--max-scans` budget of 64. `--from` supplies an inclusive lower revision. Presence at the lower boundary, missing collectors/dependencies, ambiguous rename identities and exhausted budgets remain explicit unknown/boundary results. Binary fallback verifies a local transition; nonmonotonic history means it does not prove the earliest-ever birth or uninterrupted presence. A later reintroduction can be a separate transition. Squash/merge authorship describes the Git commit, not necessarily the original contributor.

`report` adds bounded provenance to findings automatically when live Git history is available. All findings can be investigated by their kind/subject predicate; dates appear only after a verified transition. Reports share 64 historical collector calls and reuse probe results across findings. `--no-provenance` skips this cost. Recorded collector input cannot describe historical commits. The output's guard statement is conditional on the checked cycle/rule policy; it does not claim that archgraph CI was installed or ran on the historical PR.

## Hotspots

```sh
archgraph scan . --hotspots --hotspot-authors -o graph.json
archgraph report . --hotspots='12 months ago'
archgraph hotspots graph.json --root . --since '12 months ago' --authors --top 20
archgraph hotspots graph.json --root . --format json -o enriched.json
archgraph render enriched.json -o hotspots.html
```

The Git pass records per-file commits, added/deleted lines, distinct mailmapped author emails and fix-message commits, following detected renames. Only files in the scan participate in normalization; excluded generated/vendored files cannot dominate the score. The default window ends at the graph's commit, not today's date, so historical scans remain reproducible. Relative windows accept `<N> days/weeks/months/years ago`; twelve months is 365 days. ISO dates are also accepted.

```text
churn = ln(1 + file commits) / ln(1 + maximum scanned-file commits)
coupling = module (weighted_Ca + weighted_Ce) / maximum scanned-module (weighted_Ca + weighted_Ce)
cycle = 1 for a module in a cycle, 0.5 for an adjacent module, else 0
authors = file distinct authors / maximum scanned-file distinct authors
hotspot = churn * (coupling + cycle) * (1 + authors)
```

The authors factor is zero unless enabled; undefined normalizations are zero. The product is not an overall architecture quality score and can reach 4. Output shows factors and concrete counts, cycle size, cut candidates and attached findings. Existing findings are reordered by the highest relevant file hotspot; their structural/rule evidence remains intact. The map's explicit **Churn** view changes disc size/fill while preserving architecture rings. File/module panels show the counts behind the color.

Shallow clones report a history gap and zero churn rather than treating the boundary commit as real activity. Commits touching over 30 repository files are skipped as mechanical sweeps (`hotspots --max-files`, or `scan/report --hotspot-max-files`). Rename detection is similarity-based and can miss heavily rewritten files. Formatting/lockfile activity below the threshold can still inflate churn. Authors measure coordination, not productivity or code ownership.

`validation` counts unique commits whose messages contain fix/bug/regression terms. It measures the share touching the top 10% of scored files. `lift` is a naive comparison with file share; `expected_random_share` and `random_lift` also control each fix commit's number of touched scanned files through random sampling without replacement. These are in-sample associations with commit-message proxies, not verified defects or predictive/causal evidence. The [showcase validation runner](../benchmarks/validate_hotspots.py) records repository coverage, failures and preparation time alongside the statistics. The [completed 20-repository study](../benchmarks/HOTSPOTS.md) found 72.2% concentration versus 19.6% adjusted random expectation; its in-sample limitations are recorded with the result.

## Public surface

Collect symbol evidence when scanning, then inspect the saved graph:

```sh
archgraph scan . --symbols -o graph.json
archgraph surface graph.json
archgraph surface graph.json --max-symbols 50 --format json -o enriched.json
archgraph render enriched.json -o surface.html
```

`surface` prints a per-module report to standard output; `--format json` selects JSON. `-o` saves the enriched **scan document** for `render` and `check`, rather than redirecting the report. `archgraph report . --symbols` collects the same evidence and includes it in the report and map.

For each module, the report lists its entrypoints, exposed symbols and their definitions, symbols used from outside the module, and the importing files. Explicit `entrypoints` or existing `public` rules define the intended facade. Otherwise the module's top-level `index` or `__init__.py` supplies its facade; a module without one exposes its direct file exports or Go package exports.

- **Could be internal:** an exposed symbol has no external consumer **inside this scan**. These suggestions appear only when symbol coverage is complete across the scan. This does not prove that a published library's API is unused by downstream projects, and archgraph never removes exports automatically.
- **Deep import:** an outside file bypasses a module's facade. The report lists the importer, target and available symbol names, and suggests importing through the entrypoint, re-exporting the required symbol or declaring a boundary rule.
- **Oversized surface:** external consumers touch more than `--max-symbols` distinct symbols (default 50). This suggests a facade around the operations those consumers need; it does not count every unused export toward the threshold.

`check` prints these advisories when the graph has surface evidence. They do not change its exit status. Declare a strict entrypoint to make every outside bypass forbidden, including imports of files the entrypoint does not re-export:

```yaml
version: 1
entrypoints:
  "src/billing/**": "src/billing/index.ts"
```

```sh
archgraph check graph.json --rules layout.yaml
```

The value names one exact entry file. Captures also work: `"src/domains/<name>/**": "src/domains/<name>/index.ts"` binds the same domain on both sides. A bypass is forbidden even when the entry file is missing. Explicit `forbid` rules take precedence; `entrypoints` rejects bypasses before `allow` rules. Existing `public` rules retain their earlier precedence, allowing explicit `allow` exceptions. Entry imports remain subject to layer rules. See [rule ordering](#rules-file).

Symbol collection uses the installed TypeScript compiler to resolve named/default imports, re-exports and `tsconfig` path aliases; Python's standard `ast` to read `from` imports, static `__all__` and star re-exports; and Go's `go/types` to resolve exported functions, types, methods and fields with qualified names. Go method analysis conservatively retains exports that implicit interface satisfaction might need. The symbol helpers read source without executing project code or fetching dependencies; the underlying file collector still has its [usual trust requirements](#archgraph-guide).

Dynamic Python export lists, unresolved stars, namespace ambiguity and missing dependencies can make coverage incomplete. Rust and Java currently report an explicit symbol-collection gap. When `complete` is false, counts are provisional and **all could-be-internal suggestions are withheld**. Missing data can understate the surface, while namespace consumers and implicit Go interfaces conservatively credit extra symbols as used. File-level deep-import evidence remains available without symbols: `surface` accepts older scan documents and reports the missing coverage.

The map uses a bounded logarithmic ring width (1.25–6 pixels) for exposed symbol count while preserving architecture health colors. Selecting a module shows its surface metrics, importers and limitations.

## History

```sh
archgraph history . --append                         # monthly first commits, up to 24 points
archgraph history . --sample commits --every 50 --limit 30
archgraph history . --sample tags --limit 20
archgraph history . --since 2024-01-01 --hotspots --annotate
archgraph history . --diff v3.0 v4.0                 # release-note Markdown
```

The default selects the first first-parent commit of each month over two years ending at the selected `--rev` (default `HEAD`). `--sample commits` restores commit-by-commit sampling; `--every N` chooses intervals in that mode. Tags select release commits. `--limit` caps new selected samples. `--append` retains earlier cached points beyond that cap and scans only missing points. A root inside a monorepo scans that same subdirectory at every point. One language is selected from the current root (or `--language`) for the whole timeline; unrelated language tooling is not executed for cache metadata.

`.archgraph/history.json` contains full scans, adjacent diffs, release tags and summaries: files, modules, edges, module-cycle count, largest module cycle, percentage of modules in cycles, cycles per 100 modules, mean defined instability, forbidden edges (null until rules exist), and optional in-sample hotspot fix-message concentration. Counts use each snapshot's own boundaries; folder/module renames are not reconciled across time. The HTML shows both absolute cycle counts and normalized percentages; selecting a point opens its architecture map. Dashed markers identify tagged releases. `--annotate` verifies provenance between sampled points where cycles increase, then caches that context so append does not repeat those probes.

Full scan documents and sample indices are cached under `.archgraph/history/`. Cache identity includes scan root/options, current binary and dependency lock, explicit rules, selected external collector entry points and tool versions. Explicit `--rules` stays fixed; otherwise rules and ownership come from each commit. `--diff OLD NEW` scans those endpoints and reports new/resolved cycles and forbidden imports in Markdown. Its rule comparison uses each endpoint's own rules unless explicitly fixed.

Snapshots are collected in a disposable clone. The working tree, index, refs and registered worktrees remain unchanged. Failed snapshots are skipped, logged and listed in the report; an entirely failed selection exits 2. Shallow repositories expose only available commits. Ignored dependency installations and submodules are absent from snapshots; archgraph does not install/build historical projects automatically. TypeScript reuses an installed collector from the original checkout or `--depcruise`; Rust snapshots need cached locked dependencies. Missing tooling at old revisions is recorded as a failed point, not a fabricated empty graph.

[The monthly workflow](../.github/workflows/history.yml) restores cached snapshots, runs append on a schedule and uploads the report as an artifact. It is a repository workflow definition; local verification does not execute GitHub Actions.

## Impact

`archgraph impact BASE..HEAD` reports the blast radius of a committed diff: changed files, affected modules and files with shortest reverse-import depth and a decaying weight, selected tests, suggested reviewers, and cycle/hotspot risk. It leaves your checkout untouched and scans committed endpoints in a disposable local clone. It never installs dependencies or runs selected tests.

```sh
archgraph impact origin/main..HEAD
archgraph impact v1.0..v1.1 --root path/to/repo --max-depth 5 --format json
archgraph impact origin/main..HEAD --tests '**/checks/*.ts' --tests-output tests.txt -o impact.json
archgraph impact origin/main..HEAD --max-depth 5 --fail-on-depth 3
archgraph impact origin/main..HEAD --language go --go-packages-output packages.txt
```

Named/default TypeScript imports through pure re-export barrels are resolved to source declarations using the installed TypeScript compiler. Namespace imports, side-effect imports, unresolved symbols and changes to barrels retain conservative edges. The helper parses and typechecks source without executing it; if the compiler is unavailable, output records the fallback. Reverse imports consume one step; remaining pure re-export hops consume none, so an `index.ts` does not hide consumers behind an artificial depth limit. Depth defaults to 3; weight is `1 / (1 + depth)`, including changed files at depth 0. Module depth is the minimum across affected files. `--depth` retains its scan meaning (module folder depth); use `--max-depth` for traversal. Normal analysis succeeds even with risk; `--fail-on-depth N` exits 1 when a module's shortest depth exceeds N. Set `--max-depth` greater than N. Invalid inputs and collector failures exit 2.

Tests include `*_test.go`, `*.spec.*`, `*.test.*`, Python `test_*.py`, files in `test/`, `tests/` or `__tests__/`, and additional repeatable `--tests` globs. Live scans include tests automatically; TypeScript `--include-tests` collects the whole scanned root, including test trees outside `src/`. Python selection covers the chosen Grimp package; external test trees absent from that graph have unknown coverage. For saved scans, use `scan --include-tests`; a graph without test edges cannot establish coverage. `tests.txt` contains one existing head test path per line (newline-containing paths require JSON). An empty selection is not evidence that tests are unnecessary.

Go uses [`go list -deps -test -json`](https://pkg.go.dev/cmd/go#hdr-List_packages_or_modules) per module, including dependencies used only by external tests. This selection is independent of the graph depth cap. `--go-packages-output` exports package directories; run Go tests by package, from the appropriate directory containing `go.mod` in repositories with multiple modules. Selection reflects the current platform with cgo disabled, and requires cached dependencies. Failure retains graph-based tests and records the coverage gap. No project test code is executed.

Reviewers prefer CODEOWNERS for each affected module; modules without declared owners fall back to the five most active contributors over the previous 365 days, anchored to the head commit date. Git commits count once per author/module even when several files changed; names and emails follow Git mailmaps. Declared ownership is not inferred authorship. Hotspot risk uses positive-score files in the top 10% of the scanned root's hotspot ranking; cycle risk uses module cycles. Missing or shallow history is reported as a data gap.

Reuse existing clean endpoint scans to avoid collectors:

```sh
archgraph impact BASE..HEAD --graph head.json --base-graph base.json
```

The recorded commit must match the endpoint, the graph must be clean, and `--root` must match the scanned directory within the repository. Base traversal preserves deleted files and removed imports; returned test paths are filtered to files still present at head. Live scans attempt both endpoints; unavailable base collectors produce a limitation rather than dropping the head result. A saved head graph alone cannot recover missing base dependencies. Counts follow each snapshot's module boundaries, so renamed folders can appear separately.

Results mean **at least these**, not an exhaustive list. Dynamic imports missing from collectors, dependency injection, reflection, build conditions and runtime wiring can add impact. Barrel edges without enough symbol information retain conservative consumers. Vendored/generated paths and explicit scan exclusions stay excluded. This is dependency evidence, not a guarantee that running selected tests alone is sufficient.

For GitHub Actions, use the PR's actual head and base commits (checkout's default merge commit is different). Fetch both endpoints and install archgraph and its collector as usual. Have a read-only analysis job produce clean `head.json` and `base.json` scans with tests included, then pass those artifacts to a posting job:

```yaml
permissions:
  contents: read
  pull-requests: write
steps:
  # Checkout with fetch-depth: 0; download head.json/base.json analysis artifacts.
  - name: Analyze impact and post a PR comment
    env:
      GH_TOKEN: ${{ github.token }}
      IMPACT_BASE: ${{ github.event.pull_request.base.sha }}
      IMPACT_HEAD: ${{ github.event.pull_request.head.sha }}
    run: archgraph impact "$IMPACT_BASE..$IMPACT_HEAD" --graph head.json --base-graph base.json --github-actions --tests-output tests.txt -o impact.json
```

`--github-actions` writes `GITHUB_STEP_SUMMARY` and creates a new PR comment via [`gh pr comment --body-file`](https://cli.github.com/manual/gh_pr_comment); it takes the PR number from `GITHUB_EVENT_PATH`, or `--pr N`. Posting failures (including read-only fork tokens) become limitations and leave analysis successful. JSON and exported files remain available for artifact upload or later steps. Lists in comments are capped at 100 items; machine-readable output is complete. Each invocation creates a new comment. Keep write tokens out of untrusted collector/build execution; use separate analysis and posting jobs for fork PRs.

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
- It installs the archgraph release whose version is in the action's own `Cargo.toml`, so `@v0.1.0`, a commit SHA or a branch each run the archgraph they were cut from (`version` overrides). The collector goes into `$RUNNER_TEMP`, locked: dependency-cruiser 18.4.0 and typescript 6.0.3 with `npm ci` from [ci/tools/package-lock.json](ci/tools/package-lock.json) when `path` has `package.json` or `tsconfig.json`; grimp 3.17 in a venv with `pip install --only-binary :all: --require-hashes` from [ci/tools/requirements.txt](ci/tools/requirements.txt) when it has `pyproject.toml`, `setup.py` or `setup.cfg` (else set `language`), which needs CPython 3.10 to 3.14 as `python3` (`actions/setup-python`); Go 1.27.1 from go.dev, checked against its SHA-256 in [ci/action.sh](ci/action.sh), when it has `go.mod` or `go.work` (no `actions/setup-go` needed; `ARCHGRAPH_GO` names one to use instead). Linux and macOS runners.
- Paths and messages from the scanned repository reach the log with workflow commands turned off (`::stop-commands::`), and the push summary's code fence is longer than any backtick run in it.
- `archgraph.html` and `archgraph.json` are uploaded as the artifact `archgraph-<job id>`, also when the step fails. In a matrix job, set `artifact-name` per entry (`artifact-name: archgraph-${{ matrix.os }}`), since artifact names must be unique in a run.

Inputs: `path` (default `.`), `rules`, `depth` (default 2; for Go and Java, each package is a module), `language`, `package`, `exclude` (globs, one per line or comma-separated), `fail-on`, `fail-on-rules-change` (default `false`), `version`, `binary` (an archgraph executable to use instead of a release), `base-ref`, `artifact-name`. Outputs: `json` and `html` (paths), `forbidden` (count at HEAD), `new-forbidden`, `new-cycles` (counts) and `rules-changed`, the last three for pull requests only. All logic is in [ci/action.sh](ci/action.sh), which reads its inputs from `INPUT_*` environment variables, so it runs outside Actions too.

## Schema contract

New documents have `schema: "archgraph/v1.1"` and `kind: "scan" | "simulation" | "diff" | "history" | "bisect" | "hotspots" | "impact" | "surface"`. The contract is [schema/archgraph.v1.1.schema.json](../schema/archgraph.v1.1.schema.json) (JSON Schema draft 2020-12); the [v1 contract](../schema/archgraph.v1.schema.json) remains available for legacy exports. Tests validate generated documents against their schema.

- Consumers check `schema` and reject unknown major versions. They ignore unknown fields and unknown finding kinds (`kind` is an open string in the schema; known kinds have typed evidence).
- `source.language` and `source.collector.name` are closed enums that may grow within archgraph/v1 as languages are added (`go` and `go-types` were). An older archgraph, or a consumer validating against an older copy of the schema, rejects a document with a language or collector it does not know: read Go scans with an archgraph that has the Go collector.
- A scan has a top-level `summary`: forbidden, unplanned and cycle counts and each module's cohesion, Ca, Ce and instability. A finding's `simulation` is `{ "after": … }`: the same counts after the move, with `modules` listing only the modules whose metrics changed (`null` for a module the move empties). Compare it with the scan's `summary`.
- A scan document whose files, modules and edges disagree (an edge naming a file not in `files`, a file listed twice, a module its config does not give) is an input error, exit 2.
- Scans may carry optional `external_exposure` summaries and separate `external_nodes`/`external_edges` package data. These are additive v1.1 fields; the frozen v1 schema remains unchanged, and compatibility exports retain additive package data. Missing occurrence accounting means unavailable exposure.
- Scans may carry optional `symbols` (exports, imports, resolution targets, completeness and limitations) and `surface` (per-module analysis and threshold) blocks. Standalone surface reports use `kind: "surface"`. Older scans remain readable; absent symbol data never establishes that an export is unused.
- v1.1 adds weighted metrics and deferred-cycle accounting and uses canonical edge-kind names. Upgraded readers accept v1 and v1.1; frozen v1 readers need the compatibility export described below. Optional fields and unknown finding kinds should be ignored by consumers that do not use them.
- Historical v1 change before the first release, 0.1.0 (the kind names in this paragraph are the old names): `cycles`, and with them `summary.cycles`, each cycle's `break` and `two_way` and a diff's `new_cycles` and `resolved_cycles`, count runtime imports only (`static`, `reexport`). Edge `kind` gained `type-reexport` for `export type … from`, formerly `reexport`. Builds before this change also counted `type`, `dynamic` and type-only re-exports and list more and larger cycles; rescan both sides before diffing across it. Python edges gained `type` and `dynamic` (until then every grimp edge was `static`), and scans have `package_entries` and two-way `registry`/`entry` flags. In the same change an `internals-reach` finding became one per bypassed module (subject: the module id) with `imports`, `importers`, `importing_files` and up to 20 `edges`, replacing one finding per import with `from` and `to`; with rules, one per bound `public` module (subject: its pattern).
- Graph-node paths stay relative to the scan root. Raw import specifiers and explicitly configured external Python search roots may contain absolute paths. Arrays have deterministic ordering; scans with the same tree, collectors, options and dependency environment differ only in `generated_at` and `git`.
- Instability and cohesion ratios are rounded to 4 decimals and are `null` when their denominator is zero. In v1.1 they use the weighted totals described below; raw integer counts remain available.

### Edge quality and migration

`check`, `simulate`, `render` and `diff` read both `archgraph/v1` and `archgraph/v1.1` and recompute analysis with the current engine. Legacy edge names `static` and `dynamic` are accepted as `runtime` and `lazy`; `type-reexport` becomes `type` with `is_reexport: true`. New exports use the four canonical kinds. TypeScript/JavaScript classification combines dependency-cruiser metadata with a lightweight import tokenizer: function-local `require` and dynamic `import()` are lazy, while dependency-cruiser's type-only usage information takes precedence. No additional language parser is used.

Default cycles contain only `runtime` edges. Pure structural re-exports do not close a runtime cycle. `type_only_cycles` retains other cyclic components, with their `kinds` and a `contains_runtime_cycle` flag when a larger component contains a smaller runtime cycle. Despite the field name, this accounting includes lazy, re-export and mixed cycles, not just type-only cycles. Summary/report counts identify this broader category. `--all-kinds` runs cycle analysis over every kind for comparison. It is broader than the previous engine at commit `6627a5e`, whose cycle analysis already excluded types and dynamic imports but included value re-exports.

Each edge carries a `weight`. Defaults are runtime **1.0**, lazy **0.3**, type **0.1**, reexport **0.0**; customize them with `edge_weights` in `layout.yaml`. Values must be finite and non-negative. Metric attribution through barrels inherits the consumer's weight. Multiple edges between the same file pair contribute their strongest weight. Weighted Ca/Ce sum the strongest weight for each neighbouring module, rather than adding every file import; weighted internal/outgoing totals sum weighted file pairs. Instability is `weighted_ce / (weighted_ca + weighted_ce)`; cohesion is `weighted_internal_edges / (weighted_internal_edges + weighted_outgoing_edges)`. The integer `ca`, `ce`, `internal_edges`, `outgoing_edges` and `outgoing_to` counts are retained alongside `weighted_*` fields.

For frozen consumers requiring v1 field names and discriminator, add the global JSON export flag:

```sh
archgraph --legacy-v1 scan . -o graph-v1.json
archgraph --all-kinds scan . -o graph-all-kinds.json
```

`--legacy-v1` exports the compatibility shape, including old edge-kind names; it does not restore the old analysis or its unweighted metric ratios. A file-granularity scan exported with this flag is regrouped into the legacy folder view so old readers can validate it; the exported module and cycle numbers describe that folder view. Keep the ordinary v1.1 export for file-view numbers. Rescan both sides with the same options before comparing architecture changes across engines. A mixed old/new comparison can reflect methodology changes rather than repository changes. Frozen readers may ignore newer policy fields such as rule `kinds`; use an upgraded, pinned CLI when enforcing those policies in CI.

### Resolving barrels

For TypeScript/JavaScript, the collector tokenizes import and export specifiers and follows named/default re-export routes to their defining files. Python uses its existing standard-library AST pass, including relative imports, aliases, static `__all__`, and statically visible star exports. No scanned Python code is imported or executed by this pass. Go packages need no barrel resolution. Legacy collector recordings without source-route metadata retain their existing edges.

A file containing only re-exports is marked `barrel: true`; Python package initializers qualify only when they contain imports, static `__all__` and optional docstrings. Initializers with executable statements keep their load dependencies, so a real initialization cycle remains visible. Pure structural forwarding edges stay out of default runtime cycle membership. Resolved consumer edges carry `confidence: high` and point at the defining file. Their `original_to` retains the written import destination: `public`, `entrypoints`, direct forbid/allow matching and reachability boundaries check that written path, so resolving a facade import does not turn it into a forbidden deep import. Conversely, resolving a written bypass does not make the bypass permitted.

Re-export traversal is capped at ten hops. Ambiguous namespace/star imports, missing export evidence and exhausted chains retain the written barrel target as `kind: reexport`, `confidence: low`, with `resolution_reason` and a warning rather than inventing a definition. Cycles in forwarding routes produce a `reexport-cycle` finding with file-level edges. These diagnostics are separate from runtime cycles. The cap, low-confidence evidence and dynamic Python namespace changes remain limits of static resolution.

## Rules file

```yaml
version: 1
edge_weights:                 # optional; omitted values keep these defaults
  runtime: 1.0
  lazy: 0.3
  type: 0.1
  reexport: 0.0
modules:                      # optional; overrides --depth for matching files
  - { id: "domains/<name>", path: "src/domains/<name>/**" }
layers:                       # lowest first; a file may import its own or a lower layer
  - "src/shared/**"
  - "src/domains/*/model.server.ts"
  - "src/domains/*/client.ts"
  - "src/domains/*/ui/**"
entrypoints:                  # strict: outside imports must use this exact file
  "src/billing/**": "src/billing/index.ts"
public:                       # a module's public entries, with explicit allow exceptions
  - { module: "src/domains/<name>/**", entries: ["src/domains/<name>/index.ts"] }
forbid:
  - { id: shared-no-domain, from: "src/shared/**", to: "src/domains/**", reason: "shared imports no domain" }
  - { id: browser-no-server, from: "src/domains/*/client.ts", to: "**/*.server.ts", reachable: true }
allow:
  - { from: "app/**", to: "**", reason: "composition" }
default: forbidden            # optional; imports no rule decides are forbidden, not unplanned
```

Globs: `*` is one path segment, `**` any depth, `{a,b}` alternation. `<name>` captures one segment and binds it within a rule: `modules` path to id, `public` module to entries, `entrypoints` module to entry file, and `forbid`/`allow` importer to target, so `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }` matches only imports within one module. `to`, `id`, `entries` and entrypoint values may use only their rule's capture, and `layers` take no capture (write `*`). A file matching several layers takes the first. Unknown keys in `layout.yaml` are an error, and so is a control character (a line break, a tab) in any of its strings, since ids, reasons and globs are printed into logs and job summaries; rules embedded in a scan document may carry keys a newer archgraph added.

`forbid`, `allow` and `public` rules optionally accept `kinds: [runtime]` (or any selection of the four kinds); omitted `kinds` applies to every kind. A reachable forbid filters every edge along its path by the same selection, including the initial import. Strict `entrypoints` mappings always apply to every kind. Weights affect metrics, not rule matching: a zero-weight import can still be forbidden.

Verdict per import, first match wins:

1. A `forbid` rule matches: **forbidden**. With `reachable: true` it also matches when any file matching `to` is reachable from the imported file.
2. The target is inside an `entrypoints` module, the importer is outside that module, and the target is not its exact entry file: **forbidden** (rule `entrypoints`), whether or not that entry exists.
3. An `allow` rule matches: **allowed**.
4. The target is inside a `public` module, the importer is outside that module, and the target is not an entry: **forbidden** (rule `public`), whether or not the entry exists.
5. Both files are in layers: target layer at or below the importer's: **allowed**, else **forbidden** (rule `layers`).
6. Same module: **allowed** (rule `same-module`).
7. Cross-module import of an `entrypoints` or `public` entry: **allowed** (rule `entrypoints` or `public`).
8. Otherwise: **unplanned** (rule `none`), or with `default: forbidden` **forbidden** (rule `default`), so `check` exits 1 on them.

`default: forbidden` turns the rules into a whitelist. An archgraph older than this key reads a scan document whose embedded rules carry it and ignores the key (unknown keys in embedded rules are tolerated), so its `check` reports those imports as unplanned and exits 0. Pin the archgraph version wherever a whitelist gates CI.

## Findings

| kind | when | suggestion |
| --- | --- | --- |
| `cycle` | a module-level import cycle, or a file cycle among the imports inside one module, over `runtime` imports by default: type-only, lazy and structural re-export edges are accounted for separately in `type_only_cycles` (all kinds remain in weighted metrics and applicable rule verdicts) | the fewest imports to remove (`break`); with `--all-kinds`, a public entry's re-export is removed only when nothing else breaks the cycle. Public entries for this are package entries and the source root's index: `index.ts`, or the Python package's `__init__.py`, whose imports of its own submodules are re-exports. An internal barrel's re-export of another module's file is the first thing to cut ("drop the re-export"). When one registrar file, imported by at least one file (a file nobody imports is an entry point build config names) and not an index (`index.ts`, `__init__.py`: its module's entry), alone forms the other side of 3 or more two-way pairs, the cycle lists it in `registrar_moves` (`file`, `module`, `pairs`, `with`) with a simulated destination `to`: a new folder named after the file in kebab case (`driver/DriverFactory.ts` to `driver/driver-factory/DriverFactory.ts`), in the file's folder when that is a module of its own at the scan's depth, else beside its module's folder, or the folder of its one importer when it has exactly one. The destination removing the most two-way pairs (`pairs_removed`, net over all module cycles) without a new cycle wins, with the cycle's size before and after (`cycle_before`, `cycle_after`); a registrar with no such destination is not listed. The move is the first step only when it removes more than one pair while a single-import cut exists (more than none otherwise) and the cycle shrinks (`cycle_after` below `cycle_before`: removing pairs while the cycle keeps or grows its size only reroutes them); else it follows the cut, with its numbers. When a cycle has no pair to cut, the break import shown first is one into a package entry or the root index if the break has one. Module cycles also list `two_way`: each pair of members importing each other in the direction to cut, with its import counts both ways and up to 20 of its file imports: the lighter direction, unless it holds only wiring or a public re-export and the other does not. A pair whose sides both have at most 2 imports, within a factor of 2, is flagged `even` (a mutual dependency such as a foreign key both ways, listed in the names' order with no lighter side); a pair with no ordinary side is flagged `registry` (only wiring) or `entry` (public re-exports both ways). A pair whose other side is only a registrar listed in `registrar_moves` is flagged `resolved_by_move`: the move resolves it. Flagged pairs sort last and none is offered as a cut. The first ordinary pair is the cheapest cut to act on when the break set is large |
| `cycle-via-grabbag` | a folder-level cycle disappears or shrinks under a verified per-file split of low-cohesion, high-fan-in candidates; ordinary surviving cycles remain reported | split the folder using the involved file/import evidence; inspect retained residual cycles and cuts |
| `misplaced-file` | a file (not a barrel, not a module's only file) imported by at least 2 files of one other module and by no third module, with more than twice as many of those imports as edges (in + out) within its own module, whose move adds no forbidden import or cycle, gives the target no new dependency and raises the target module's cohesion by at least 0.02 and the two modules' combined cohesion. What the file itself imports never pulls it: a router importing `app/deps` stays where it is. Not reported when both modules' last folder names a kind of code (`repositories` to `services`, `menus` to `components`, singular or plural, any case; the list also has `exception`, `error` and `event`): a codebase organised by kind would change the file's kind, not its boundary. Nor between a module and its own subfolder (`server/lib` to `server`), which only changes folder depth, nor for a leaf utility: a file that imports nothing, in a module whose last folder is `util(s)`, `helper(s)`, `lib`, `shared`, `common` or `storage` | a move into that module, keeping the file's folder within its module when the target has it (`chains/domain/hd.py` into signer is `signer/domain/hd.py`), else its root. The file's folder is read relative to the deepest directory its own module's files share, and the target's folder counts only when a file of the target module sits in it. With simulation; none when the name is taken there or the destination would fall in another module |
| `sink-split` | a module imported by 3 or more modules whose files serve separate groups of importers, at least two of the groups holding 2 or more files (a lone file one module uses is a `misplaced-file` case), and not a folder named by kind (`utils`, `services`, `errors`, the `misplaced-file` list): such a folder serves separate importers by design. Groups follow runtime imports only and leave out wiring (composition roots, registrars), the sink's parent folder as a consumer, and a file that imports 3 or more modules but only the sink's entry or index (it registers the sink). Not reported when the groups import each other at runtime (the split would be a cycle) | one module per group; single-consumer groups as moves, unless two files share a name, the sink and the consumer are both folders named by kind (as for `misplaced-file`), a file is public API (a package entry or what it re-exports; in Python, any module with no `_`-prefixed segment; in Go, any file outside `internal/` folders and `package main`) or the moves raise some receiving module's cohesion by less than 0.02 |
| `internals-reach` | with rules, the imports a `public` or `entrypoints` rule forbids, one finding per bound boundary module (subject: its pattern with the capture bound, `src/domains/cart/**`; `module`: the module of its first import's target), exactly what `check` reports. Without rules, TypeScript only, one finding per module (subject: its id) over imports of a file that the module's entries (barrels and package entries) re-export, directly or through other re-exports, when at least half the cross-module imports into that module go through its entries (`barrel_share`): an entry most importers bypass is not the module's API, and an import of a file no entry offers uses something the module does not export; not from a module below the target's module (`packages/common/decorators/core` into `packages/common`), where importing the entry would be a cycle. Evidence counts the bypassing `imports`, their distinct `importers` (modules) and `importing_files`, and lists up to 20 of them (`edges`) | import the entry instead |
| `unstable-dependency` | module A imports B (through barrels, as Ca and Ce count), B is at least 0.25 more unstable than A, and files of A import files of B at least twice at runtime (`imports`: `runtime` file imports, not through barrels, not wiring). Not reported between a module and its own subfolder (`db` and `db/backends`), nor into a private module of the importer's own package (`pkg/_internal` from `pkg/api`): a public facade over its implementation | depend on an abstraction owned by A, or move the code |
| `seam` | file clusters that fit the imports better than the folders do (modularity gain of at least 0.05), holding at least half of the non-barrel files; files with a degree above the 90th percentile are left out as hubs. Not reported when more than half the non-barrel files sit in single-file modules (a flat package where every root file is its own module), or when more than half the folder modules (root-file modules aside) are named by kind (`controllers`, `services`, `repositories`, the `misplaced-file` list): a layout by kind, which clusters across kinds only restate. Negative folder modularity is no obstacle: folders that cut across the clusters are the clearest seam | the clusters as candidate boundaries |
| `hidden-coupling` | with `--cochange`, files in different modules changed together in 3 or more commits without importing each other (commits touching more than 30 files are skipped, renames followed) | evidence only |
| `small-repo` | fewer than 20 files or 3 modules | findings carry little signal |

## Limits

- `scan` runs tooling the scanned repository controls (see the warning at the top): scan only trusted repositories, or inside a sandbox.
- Source selection and exclusions limit the graph. Included Python tests can be discovered outside the selected package; excluded files do not establish missing test coverage.
- Type-only, lazy and structural re-export imports retain weighted metrics and rule verdicts. They do not close default runtime cycles; separate deferred-cycle accounting and `--all-kinds` make their cycles visible.
- Imports are not semantics: DI containers, event buses, string-based lookups, reflection and dynamic imports without static strings are invisible. `--cochange` partly covers this.
- Go: references come from type-checking each package without its dependencies outside the repository, so a use reached only through such a dependency's types (a method of a value it returns, an argument of a call to a function it provides) is missing, and an import left with no resolved use counts as unresolved; every platform's files are merged; reflection, `go:linkname`, `go:embed`, assembly and C are invisible. A `go` line in `go.mod` newer than the Go running the scan fails it: archgraph never downloads a toolchain.
- Small repositories have little signal; the `small-repo` finding says so.
- Findings are moves with evidence. There is no single score to optimize; module colors in `render` are a reading aid.
- Folder boundaries come from `--depth` or rules `modules:`; `--granularity file` offers a file view. Wrong boundaries give wrong metrics; grab-bag findings require a verified split rather than a name heuristic.
- Named imports through proven barrels resolve to their defining files. Legacy recordings and imports lacking usable export evidence can retain conservative barrel attribution; inspect low-confidence edges before treating them as runtime dependency evidence.
