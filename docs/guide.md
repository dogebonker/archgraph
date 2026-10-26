# Boundrail guide

The full reference: install options, every command, the CI action, the rules file, the scan document contract, every finding kind and the limits. The [README](../README.md) is the short tour.

boundrail checks declared architecture rules against a repository's import graph. The graph establishes dependencies; the policy establishes which are undesirable. `init` combines reasoned presets with observed code, `sync-rules` shares the policy with agents, and installed hooks compare edits with a pre-edit snapshot. New violations include fix recipes; ADR imports can supply the reason and source. Architecture documents are optional. The analysis commands retain coupling, cohesion, cycle cuts, surfaces and simulated file moves. The [edit-loop evidence](agents/loop.md), [paired agent results](../bench/results/README.md) and [interoperation gaps](interop.md) distinguish measured behavior from remaining limits.

The default reference engine uses those collectors; its Python pass also reads importing files with the standard `ast` module, without running them, to distinguish deferred and type-only imports. The experimental native engine uses tree-sitter queries for every supported language, with Rust resolvers. Both feed the same graph analysis. For Go, imports and identifiers are resolved with Go's standard library (`go/types`) by an embedded helper run with your Go toolchain, which compiles nothing from the repository and runs no repository code. `report` and `history` write into `<root>/.boundrail/` without `--out-dir`; `serve` also maintains its local endpoint and live graph there.

**Scan only repositories you trust, or scan inside a sandbox.** For TypeScript/JavaScript, `scan` runs `npx --no-install depcruise` in the scanned repository, and npx resolves that repository's own `node_modules/.bin/depcruise` and reads its `.npmrc`: the repository controls what runs. `--depcruise <path>` runs that executable instead, so the repository no longer picks the binary (depcruise still reads its `tsconfig.json`). The Python collector runs your interpreter with `-I` and adds the source root to `sys.path` last, so repository files cannot shadow the standard library or grimp; git runs with `core.fsmonitor=false`. The Go collector writes its helper to a temporary directory outside the repository and runs `go run` there, which compiles only the helper; the helper runs `go list -e -find -json ./...` in each module of the repository, which reads `go.mod` and the source files' package clauses and build constraints and builds, generates and runs nothing. Both run with `GOTOOLCHAIN=local` (a `toolchain` line in the repository's `go.mod` would otherwise make the go command download and run another toolchain; with it the line is ignored, and a `go` line newer than your Go fails the scan instead), `GOPROXY=off` (no network), `GOWORK=off` (a `go.work` in the repository is not read), `CGO_ENABLED=0` (no C toolchain) and `GOFLAGS=-mod=readonly` (replacing any `GOFLAGS` from your environment or `go env`, so no `-toolexec` or `-overlay` applies, and `go.mod` and `go.sum` are never written).

## Install

For the current Boundrail CLI, build the checked-out source with stable Rust:

```sh
cargo +stable install --path . --locked --bin boundrail
```

The current source includes the Boundrail CLI and package definitions. Registry publication is a separate release step.

The installer supports Boundrail prebuilt releases on Linux and macOS, installing the executable in `~/.local/bin`:

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/dogebonker/boundrail/main/install.sh | sh
boundrail --version
```

[install.sh](../install.sh) reads the release's `SHA256SUMS` and selects the exact Boundrail archive entry. It downloads over HTTPS only and installs only when the archive's SHA-256 matches (it needs `sha256sum` or `shasum`). `BOUNDRAIL_VERSION=0.1.0` picks a release (default the latest), `BOUNDRAIL_INSTALL_DIR` the directory. On Windows, download the matching Windows x64 ZIP from [releases](https://github.com/dogebonker/boundrail/releases). npm is an option after the package is published.

A checksum file next to the archive proves the download is intact, not who built it. Every release archive also carries a build provenance attestation from the release workflow; check the actual filename listed for that release with the GitHub CLI. For example, using the exact filename of a published release:

```sh
gh attestation verify boundrail-v0.1.0-x86_64-unknown-linux-musl.tar.gz --repo dogebonker/boundrail
```

For the maintainers: keep immutable releases turned on in the repository settings, so a published release's archives and tag cannot be replaced afterwards.

After npm publication, Node 18 or later can run [boundrail-cli](../npm/README.md), which downloads, verifies, and caches the same binary on first use:

```sh
npx boundrail-cli report . --out-dir /tmp/arch
```

From source, with a Rust toolchain:

```sh
cargo install --git https://github.com/dogebonker/boundrail --locked
```

The collector runs inside the scanned repository:

- TypeScript/JavaScript: `npm install --save-dev dependency-cruiser@18.4.0 typescript@6.0.3` in that repository. `scan` runs `npx --no-install depcruise`. Or install both once elsewhere (`npm install --prefix /opt/boundrail-tools dependency-cruiser@18.4.0 typescript@6.0.3`) and pass `--depcruise /opt/boundrail-tools/node_modules/.bin/depcruise`, which runs it with the same arguments.
- Python: a Python with `grimp==3.17` (`pip install grimp==3.17`, or `uv run --with grimp==3.17`), passed with `--python`.
- Go: Go 1.21 or later on `PATH` (tested with 1.27.1, from [go.dev/dl](https://go.dev/dl)), or passed with `--go <path>`. Nothing is installed into the repository, and dependencies outside the repository are never loaded. The module cache is used only when the go command needs a module's dependency graph; without it, that module is read with `go/build` (see Go below).

- Rust: rustup with `rust-analyzer` and `rust-src` (`rustup component add rust-analyzer rust-src`), a workspace with `Cargo.lock`, and dependencies already fetched. Uses the active installed toolchain, never downloads one. `--rust-analyzer <path>` overrides the analyzer executable.
- Java: JDK 11 or later on `PATH`, or `--java <path>`. The helper uses [JavacTask parse/analyze](https://docs.oracle.com/en/java/javase/24/docs/api/jdk.compiler/com/sun/source/util/JavacTask.html), without generating bytecode or running Maven, Gradle or annotation processors.
- Elixir: Elixir 1.14+ with Erlang/OTP 25+ on `PATH`, or `--elixir <path>`. The embedded helper parses source with `Code.string_to_quoted`; it never loads `mix.exs`, evaluates source, compiles the project or expands macros. No Mix dependency installation is required.

## Reading evidence before changing code

The map separates declared **policy violations** from **observations**. The Violations filter shows forbidden imports only. Unplanned imports are uncovered by policy and appear as informational coverage; they do not imply that the import is wrong. An explicit `--fail-on new-unplanned` remains an opt-in coverage gate.

A folder dependency loop is reciprocal coupling, not proof of a circular file import. For example, FastAPI's `app/main.py → routes/orders.py → app/deps.py` is a file DAG even though grouping the two app files yields `app ↔ routes`. Glarity deliberately permits this wiring. These folder-only cycles remain in `cycles` and advisory `diagnostics`, without an import-cut recipe. A recipe is offered only when concrete scanned cyclic file imports support it; Go uses physical compiler packages instead of files. Even a real static cycle does not alone establish a runtime failure or a behavior-preserving repair.

The edit gate and `diff --fail-on new-cycles` compare concrete cyclic import pairs against the baseline. Folder regrouping alone cannot block, while adding a real file cycle inside an already reciprocal folder component can. Existing cyclic imports stay advisory. Raw diff cycle lists retain folder observations for analysis; `new_cyclic_imports` gives concrete file witnesses for enforcement, including a new file cycle inside an unchanged folder component. Go comparison identities use physical package pairs, so a changed representative file does not turn an existing package cycle into new debt. Inspect the command exit/verdict for enforcement.

Inferred entrypoints describe observed export conventions. A deep import gets a repair recipe only when `layout.yaml` explicitly declares its public boundary; otherwise its evidence remains in diagnostics. An initializer or barrel is not sufficient proof that every direct submodule import should be rewritten. Existing v1/v1.1/v1.2 readers and cycle kinds remain supported.

## Day-zero presets and init

```sh
boundrail init --preset layered --scope internal
boundrail check                 # fresh native scan of this directory
```

`init [root]` creates a reviewable `layout.yaml` and prints the top three actionable findings or `clean`. It asks one scope question when stdin is interactive; scripts must supply `--scope`. The default preset is `layered`; alternatives are `hexagonal`, `feature-sliced`, `modular-monolith` and `minimal`. Presets express folder-layer direction and explicit reasoned boundaries. Adapt their paths and declare project-specific entrypoints before relying on a framework's complete architecture conventions. Minimal keeps one ownership layer and the inferred/default module boundaries for the grab-bag guard; it adds no inferred import bans. Scope limits apply to all presets.

| Scope | File lines, maximum | Distinct file importers, maximum | Repo files importing a module, maximum | One observed explicit implementation |
| --- | ---: | ---: | ---: | --- |
| demo | 1,500 | 100 | 90% | Disabled |
| internal | 1,000 | 60 | 70% | Advisory |
| production | 600 | 30 | 40% | Advisory |
| regulated | 350 | 15 | 25% | Advisory |

Every scanned production file must match a layer in all scopes. Module reach counts distinct production importing files outside the module, divided by all scanned production files; duplicate imports do not inflate it. File lines include comments and blanks. Deterministic caps and unclassified files fail live `check`; one observed implementation is a medium-confidence review candidate. Its TypeScript evidence uses the existing parser and uniquely resolved exported interfaces plus explicit `class implements` clauses, including resolved import aliases/barrels. Structural, inherited, generic, namespace-qualified and unscanned implementations remain unknown. Other languages retain the graph/size checks and disclose unavailable implementation evidence. The existing grab-bag thresholds require cycle-backed evidence; they are not a blanket ban on large utility folders.

Each threshold under `early-smells` has a `reason`; `layer-reason` explains implicit ordered-layer violations. Explicit import rules carry their own `reason`. Check output and recipes retain these explanations. All presets include a `## How to change this` comment block; the [20 golden policies](../tests/fixtures/presets/) show every preset/scope combination. Existing v1/v1.1/v1.2 scan inputs remain readable. Saved scans with mandatory size checks return `INCOMPLETE` and exit 2 when no live root is available; use `check <directory>` for full evidence. Existing saved scans without these policies keep their prior check behavior.

For an existing repository, init runs the existing layout inference, merges its module/layer evidence into the preset, and marks conflicting observations `# CONFLICT: inferred X, preset says Y`. Preset boundaries take precedence; review sparse inferred forbids before adoption. It refuses existing layouts and symlinks; `--force` explicitly replaces an existing regular layout. First-run findings describe the resulting draft, including existing violations; creating a draft does not assert that its policy is satisfied. Installed hooks adopt a newly created first layout at the next clean pre-edit event; creating or changing policy during a pending transaction remains unavailable until resolved.

## Generated agent rule files

`boundrail sync-rules [root]` renders `layout.yaml` into owned `<!-- boundrail:start -->` / `<!-- boundrail:end -->` blocks in `CLAUDE.md`, `AGENTS.md`, `.cursorrules` and `.github/copilot-instructions.md`. It includes layer direction, explicit import constraints, declared entrypoints/public surfaces, package categories, available reasons and a pointer to MCP/hooks. Default output targets at most 600 **estimated** tokens per block (UTF-8 bytes / 4, including marker/hash framing); omitted rule lines are counted and the layout/MCP remain authoritative. `--full` includes every rendered rule.

Init synchronizes these files after creating its draft. Live `check` warns when blocks are stale, including an explicit `--rules layout.yaml` resolving to the repository policy; this warning does not change the policy verdict. Saved scans and genuinely alternative policies do not pretend to identify the current repository rule files. Use `sync-rules --check` in CI: exit 0 means current, exit 1 means missing/stale, and exit 2 means malformed input or another setup error. Default check accepts the stored full/brief mode; `--check --full` requires full blocks.

Content and layout hashes distinguish manual edits from a changed source policy. A hand-edited owned block refuses the entire write batch and prints a diff; move custom instructions outside the fence, restore the generated block, then sync again. Missing/duplicate/malformed fences are refused. Original bytes outside existing fences, including line endings and non-UTF-8 bytes, remain unchanged. New blocks append to existing content. All targets are checked for symlink paths before writes; file permissions are preserved. Each replacement checks the preflight bytes again to reject an observed concurrent edit. Replacements are atomic per file; an I/O failure can leave other files stale, so rerun `--check` after correcting it.

## Project architecture skill

```sh
boundrail skill draft .
# Review the inactive SKILL.md and its policy references.
boundrail skill adopt .
```

`skill draft [root]` uses native source analysis and writes
`.boundrail/drafts/project-architecture/SKILL.md`. It describes observed modules and services,
available source-backed examples, current policy references, and commands to collect a baseline
and check an edit. The examples describe draft-time evidence. The instructions require a fresh
scan before changing code. Runnable snippets target POSIX shells. Semantic and runtime
requirements remain unverified.

Drafting does not change active policy, source files, or host instructions. Without a root
`layout.yaml`, it creates a separate layout proposal in the draft directory and preserves an
existing proposal. Review the intended constraints and create the root layout, then draft again.
Observed dependencies do not establish intended architecture.

An optional `--manifest` selects a repository-relative service manifest. Otherwise an existing
`services.yaml` supplies service policy. Generated service commands use the new-only gate only
when the manifest contains authored forbid rules. Without those rules, service collection remains
advisory, so a repository without services receives a usable workflow.

`skill adopt [root]` verifies the reviewed draft and current policy fingerprints, then installs
`.agents/skills/project-architecture/SKILL.md`. It copies the reviewed content without generating
new observations. A changed policy requires a fresh draft. Adoption installs the skill workflow,
and does not apply a proposed layout or enable hooks.

Draft regeneration and installed-skill updates preserve frontmatter and notes outside the owned
block. A hand-edited generated block, malformed or duplicate fences, a symlink path component,
or an occupied unowned installation fails instead of overwriting that content. Review the new
draft before adopting an update. The existing policy files remain the rule source of truth.

## ADR import and lifecycle checks

`boundrail adr import docs/adr` proposes a YAML fragment; it leaves the layout unchanged until `--apply`. Use `--json` for the full inventory or `--brief` for its bounded envelope. Clear positive import prohibitions in Decision/Decision Outcome sections map to reasoned rules; existing module IDs resolve to unique declared file globs. Examples, alternatives, ambiguous scopes and call restrictions stay informational. Proposals include the ADR ID/title, repository-relative `source`, and source digest; apply rechecks source/lifecycle and layout bytes before publishing. It preserves existing comments and refuses ambiguous flow-style/anchor edits.

`boundrail adr check [root]` flags stale or missing sources and distinguishes unknown lifecycle. `--require-source` also lists, under `unsourced`, each `forbid` and `allow` rule (by `id`, else `forbid[N]`/`allow[N]`) that cites no `source`, and fails when any does; run it in CI to keep every enforced boundary tied to an ADR. `layers` has no `source` field and is not checked. Check output, generated rule blocks and recipe notes cite source paths. MADR, ADR-kit and Archgate metadata differ; Archgate does not supply a standard status field. Regex checks and arbitrary companion TypeScript are retained in their original tools. A narrow reviewed Archgate ARCH-018 adapter supplies three runtime-only package rules with type/dynamic exemptions. The measured upstream conversion remains below the requested 50% target; see [the pinned compatibility report and gap list](interop.md).

## Native engine

```sh
boundrail scan . --language typescript --engine native --symbols -o graph.json
boundrail report . --language python --package myapp --engine native
boundrail history . --engine native
boundrail scan . --language typescript --engine depcruise --depcruise /opt/tools/depcruise -o reference.json
```

`--engine reference` is the default. `--engine native` supports TypeScript, JavaScript, Python, Go, Rust, Java and Elixir. `--engine depcruise` is the TypeScript/JavaScript reference escape hatch. Collector options such as `--depcruise` and `--python` apply to the reference path. Native collection and native `--symbols` use embedded tree-sitter grammars and query captures, without launching Node, npx, a Python interpreter, rust-analyzer, Cargo, a JDK, an Elixir VM or repository code. Native Go still runs read-only `go list` metadata queries with your installed Go toolchain; it does not run the go/types helper or compile project code. Git is still used for provenance and history.

Parsing is not resolution. TypeScript/JavaScript uses oxc_resolver for tsconfig paths, package maps and workspace resolution, with source-tree evidence for local packages. Python resolves relative imports, package initializers, namespace directories, source layouts and explicit `--python-path` roots in Rust. Missing dependencies and unknown packages remain unresolved rather than being guessed to be external; recognized Python standard-library roots are recorded as external identities. Dynamic import expressions, runtime path mutation and computed exports cannot be fully resolved statically. Check `unresolved`, resolution coverage and symbol limitations before relying on a boundary or unused-export suggestion.

Collection limits are saved in `source.collector.limitations`, including scans without `--symbols`. Missing inherited tsconfig files permit a bounded fallback for relative imports and builtins; aliases still need their configuration. Python discovers reachable production modules through excluded test sources to match package discovery, then applies exclusions before emitting graph nodes and edges. This does not include the tests in cycle analysis.

Native Go resolves package identities through `go list -e -deps -json`, using local-toolchain, offline and read-only flags; field/method inference and platform semantics are conservative. Filename/directory exclusions apply, but generated files identified only by a `Code generated ... DO NOT EDIT.` header are not detected by this engine. All selected native Go files remain package-bound, disabling move suggestions that would need go/types proof. Native Rust reads source modules, `use` declarations and Cargo TOML manifests without Cargo, rust-analyzer, build scripts or macro expansion. Definite `#[cfg(test)]` / `#[cfg(all(test, ...))]` references are excluded from the production graph; other cfg branches remain conservative. Module declarations are retained as structural `reexport` edges with the default zero weight; they do not close runtime-kind cycles. Named imports, lexically known module-qualified references and statically indexed exported macros point at their defining files. Globs narrow to syntactically used names when supported; ambiguous globs remain low-confidence structural edges. Rust cycle labels describe source/type coupling, not runtime initialization failures. Native Java indexes contained source declarations and imports without javac or a project build; unknown classpaths and semantic dispatch remain unresolved. Native Elixir extracts aliases, static calls and references without loading Mix or starting a VM; its bundled standard-module/application catalog comes from Elixir 1.20.1 / OTP 29 metadata. Other runtime versions and macro-generated definitions can differ. Go, Rust, Java and Elixir symbol indices are explicitly incomplete, withholding unused-export suggestions that require complete coverage. Their native collectors have focused fixture validation; the 20-repository comparison below does not validate these four languages.

Native Python recognizes installed import roots from contained `.dist-info/top_level.txt` or wheel `RECORD` metadata, including pure Python modules, namespace packages and native extensions. The referenced importable files must exist inside the installation root. Distribution names alone, missing wheels, malformed metadata and executable editable-install `.pth` files do not prove an external dependency; unresolved imports remain explicit. Python resolver metadata paths are included in daemon freshness checks when an installation changes.

For a conventional single Rust crate, new scans use `src` as the grouping root, matching other source-layout languages; folder depth is measured below that root. Multi-package workspaces and nonstandard entrypoint layouts conservatively retain the checkout root. Reports and CLI/MCP explanations disclose grouping and source-reference semantics. A 36-module SCC is one mutually reachable component, not 36 independent cycles or proof that one hub caused every edge. Compare actual edges or simulated deltas before attributing it. Old v1/v1.1/v1.2 scans remain readable and retain their recorded source-root settings and edge evidence; fresh source collection is required for the AST-dependent cfg and macro fixes. Rescan source and restart an existing daemon to apply collector changes.

The selected engine is recorded as optional `config.collection.engine`; an absent value means `reference`, preserving older scans and historical cache behavior. Collector names are `tree-sitter-typescript`, `tree-sitter-python`, `tree-sitter-go`, `tree-sitter-rust`, `tree-sitter-java` and `tree-sitter-elixir`. All use the existing scan schema and shared exclusions, barrel handling and analysis. History keeps the chosen engine for snapshots, so native history does not install or launch reference collectors. Go snapshots still need the local Go toolchain.

The [comparison runner](../benchmarks/native_engine.py) scans the first 20 entries of the showcase manifest at their exact pins with both paths, retaining raw JSON, logs, hashes, file-scope changes and every edge disagreement. The [generated report](native-engine-comparison.md) reports precision and recall for resolved internal edges, separate kind/target agreement and unresolved counts. A default switch requires **both precision and recall above 98% for every repository**, plus review of kind, scope and resolution differences. Matching endpoints alone cannot justify a switch when runtime classifications or unresolved counts disagree. Agreement with a reference collector is not proof of correctness, and no speed multiplier is assumed.

## MCP server

```sh
# From the repository you want the agent to inspect:
claude mcp add --transport stdio boundrail -- boundrail mcp . --engine native --symbols
# Or launch the protocol server directly:
boundrail mcp /absolute/path/to/repo --engine native --symbols
```

`mcp [root]` serves the ten tools below using the official [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk). The client starts a local stdio process: stdout contains MCP messages and diagnostics go to stderr. It accepts the shared scan options, including `--rules`, `--hotspots`, language selection and exclusions. Hotspot/history facts remain unavailable unless collected; unavailable facts are never treated as zero. The reference engine remains the default; the examples explicitly select the experimental native engine and enable symbol evidence. Recorded `--collector-input` snapshots are rejected because MCP verification requires live source collection. Collector trust requirements and language limits still apply.

Use the [Claude Code config](mcp/claude-code.json) as a project `.mcp.json`, the [Cursor config](mcp/cursor.json) as `.cursor/mcp.json`, or merge [the Codex TOML](mcp/codex.toml) into your Codex configuration. Replace `/absolute/path/to/repo` in the Claude/Codex examples; Cursor expands `${workspaceFolder}` from the folder containing `.cursor/mcp.json`. Install `boundrail` on the client's PATH, or set `command` to its absolute executable path. Merge existing configuration rather than overwriting other servers. Client configuration details: [Claude Code](https://code.claude.com/docs/en/mcp), [Cursor](https://cursor.com/docs/mcp), [Codex](https://developers.openai.com/codex/mcp).

| Tool | Inputs besides optional `brief` | Evidence returned |
| --- | --- | --- |
| `context` | `task`, `budget_tokens`, optional `files`, `symbols` | Tiered reading plan, reasons and token estimates; the budget covers source context, not response size. |
| `check_diff` | Optional `base` | New forbidden imports, `stale_known` entries, new/grown cycles and current findings compared with the working tree. |
| `propose` | `finding_id` | Ordered actions backed by a current finding, expected deltas and verification predicates. |
| `simulate` | `moves: [{from, to}]` | Before/after cycles, removed pairs and new cycles without moving files. |
| `verify` | Optional typed `expect` | Fresh findings and pass/fail for the requested predicates. |
| `explain` | Exactly one of `path`, `module` | Importers, imports and available architecture evidence within the smaller explanation cap. |
| `rules` | `paths` | Relevant configured layout rules and surface constraints. |
| `review` | Optional `base` | What the working tree changed in the target since `base`, as `data.markdown` (also the text content) for an AI reviewer to quote: newly broken slices, other new forbidden imports with their rules' reasons, newly fixed slices, each import at `file:line`, a `Base` line, and a note on the rows `layout.known.yaml` lists (`known: true` in JSON) and on listed entries that no longer occur. The JSON arrays hold every row; `brief` drops them. `boundrail review [root] [--base <ref>] [--format markdown\|json]` prints the same markdown. |
| `target` | none | The scan's `target` (claim verdicts, slice progress, pending move cost), or `null` when the rules state no claim or move. |
| `services` | Optional `manifest`, exact `service` | Live service metrics, call evidence, owned client usages, authored `forbid` rules, `policy_source`, and caveats. `global_summary` and policy cover the whole model. `scope.service` identifies an optional evidence focus. |

The `services` tool uses native service inference directly and does not run the reference import
collector. Its optional manifest must be a file inside the MCP repository root, including after
symlink resolution. Service IDs are exact, not globs. With no manifest it infers the root, like
`boundrail services .`. Calls include either direction for a selected service; client usages
belong to that service. It returns a compact view, not a complete CLI report with graph indices.
`forbid` preserves all authored rules in precedence order, including rules with no observed match.
`policy_source` contains the resolved repository-relative manifest path and the SHA-256 of the
bytes used for this report. Without an explicit manifest, `policy_source` is `null` and `forbid`
is empty. That means no service policy was supplied, rather than approved architecture.
The service tool namespace and each rule's `id` identify the policy. Import rules remain in
`layout.yaml` and the `rules` tool. Use `boundrail services . --manifest services.yaml --format json`
for complete evidence when the MCP response is truncated. Omit `--manifest` if none was supplied.

Tool input and output JSON Schemas are advertised by MCP `tools/list`; use discovery as the contract rather than parsing display text. Every tool accepts `brief: bool`, default `true`. MCP `propose` returns the typed recipe once in `structuredContent.data`, with a short text pointer, so duplicated JSON does not consume its output cap. A truncated recipe is incomplete; use `boundrail propose <finding_id>` for the full contract. Lists use deterministic ordering. Finding IDs derive from semantic identity rather than array positions, scores or timestamps; use an ID from the current findings when calling `propose`. A fixed commit and collection inputs retain the same IDs. A changed finding can have a new identity. Proposed steps are import-graph evidence, not proof that code compiles; run the project's tests after editing. Tools inspect or simulate code, and do not apply code changes.

Results use an envelope `{data, truncated, omitted, counts?, hint?, error?}`. `data` is an object or `null`; `omitted` counts removed values and `counts` records original top-level array counts where available. Both brief projection and hard-cap pruning set `truncated: true` with a hint, so an original count can exceed the displayed array length. Read the marker before interpreting a partial answer. Errors carry an actionable message and MCP error status.

Responses have a hard cap of **2,000 UTF-8 bytes** for the serialized MCP tool result, including repeated text and structured content. `explain` uses **300 bytes**. `review` and `services` use **4,000 bytes**. These conservative byte limits bound token usage without choosing a client tokenizer; they often leave less room than 2,000/300 natural-language tokens. Truncation is explicit and includes a hint. `brief: false` requests more detail within the same hard cap; narrow the task, seed files or paths, or use the CLI for complete evidence. Verification evaluates the full result before truncating its presentation. Do not interpret omitted findings as absent. `explain` returns a compact `data.summary`; the same compact summary is included as text for clients that only render text content. `?` means unavailable or unclassified, rather than zero. It prioritizes import/importer examples and counts, layer, rule ID and available hotspot/history facts. Long identifiers may require a counts-only summary with an explicit truncation marker.

Without an explicit `base`, `check_diff` uses the guard's merge-base selection: `origin/HEAD`, then `main` or `master`, falling back to `HEAD` when none exists. An explicit base resolves to a commit. The head snapshot includes uncommitted edits. Both snapshots use the current layout rules, and policy changes are disclosed; the result does not claim a rule existed at the base revision. New/grown cycles use the existing architecture diff semantics. `check_diff` includes current v1.2 findings and their recipes; call `propose` to save a live verification baseline before editing.

Use explicit predicates when verification must assert a result, for example:

```json
{"expect":{"cycles_in":["app/editor"],"equals":0,"forbidden":0}}
```

`cycles_in` contains exact existing module IDs; `equals` defaults to zero and applies separately to each listed module's cycle count. `forbidden` is the expected total, not just new imports. Recipe predicates also accept `baseline_id`, `scope` and `expected` for measured before/after checks; pass the complete `recipe.verify.structured` returned by live `propose`. Other optional predicates are `findings_absent: [finding_id]`, `absent_edges: [{from, to}]`, `files_present: [path]` and `files_absent: [path]`. File predicates refer to the scanned graph's file inventory, so excluded files are absent from that evidence even if they exist on disk. Combine predicates to check the proposed change and guard against regressions; run `check_diff` again against the same base. A syntactically valid already-absent finding ID passes an absence check, without proving the finding existed before. Calling `verify` without `expect` is an inspection: its success confirms the scan completed, **not** that the architecture is clean. Use the returned `expectations_supplied` and predicate results when interpreting `pass`.

A matching running `serve` daemon can provide scans through its authenticated local socket or loopback endpoint. The stdio process bridges that protocol; the daemon's length-framed `command` socket is not a raw MCP transport and must not be entered as an MCP URL. Collection options and environment must match for cached scans. `--no-daemon` forces standalone collection, and an unreachable stale endpoint falls back to standalone. Refresh/setup errors remain errors with a next step rather than a successful stale answer. Current scans use v1.2 recipes; upgraded readers still accept v1 and v1.1 inputs.

## Reviewing a pull request with an AI reviewer

`boundrail review [root] [--base <ref>]` prints what a branch changed in the target, short enough for an AI reviewer to quote: newly broken claim slices and other new forbidden imports at file:line with the reason they quote, newly fixed slices, the progress line and the base commit used. MCP clients call the `review` tool for the same text. Two ready-made reviewer instructions tell an AI reviewer to quote it, treat its verdict on import direction as final, and judge only what it does not check:

```sh
mkdir -p .agents/checks && cp integrations/amp-review.check.md .agents/checks/boundrail.md          # Amp
mkdir -p .claude/agents && cp integrations/claude-review.agent.md .claude/agents/boundrail-reviewer.md  # Claude Code
```

The Claude Code reviewer lists no `tools`, so it inherits the session's boundrail MCP tools. Copy the file once; neither `init` nor `sync-rules` writes it.

## Fix recipes and verification

Every actionable `findings[]` entry has a stable `finding_id` and a typed `recipe`. Evidence that cannot determine a concrete edit remains in `diagnostics[]`, with the original finding and a reason; diagnostics are advisory, not automatically fixable findings. This includes weak-signal or conceptual observations such as small repositories, hidden coupling and unsupported boundary changes. A diagnostic is not evidence that the architecture is clean.

```sh
# Collect source evidence and choose a findings[].finding_id from graph.json.
boundrail scan . --engine native --symbols --cochange -o graph.json
boundrail propose <finding_id> --root . --engine native --symbols --cochange
boundrail apply <finding_id> --root . --engine native --symbols --cochange --dry-run
# Once the exact plan is within the requested change:
boundrail apply <finding_id> --root . --engine native --symbols --cochange --yes
boundrail verify <finding_id> --root . --engine native --symbols --cochange
```

Use identical collection, module, rule and hotspot options throughout. These commands also accept `--graph graph.json` to reuse its saved configuration; `apply --yes` and `verify` still collect live source. `--collector-input` cannot substitute for that live scan. `--brief` gives the shared bounded projection; omit it to review complete steps, planned source buffers and verification checks. Obtain an ID from the current scan rather than a finding's array position. IDs exclude timestamps, scores and measured deltas, so reordering findings does not change their identities.

A recipe contains:

- Ordered `steps` with `kind`, `file`, optional `line`, `from`, `to`, structured `detail` and `mechanical`. Kinds are `cut_import`, `move_file`, `add_reexport`, `extract_interface` and `split_module`.
- `expected`: `pairs_removed`, `cycle_size_after`, `new_cycles` and a per-module `instability_delta` map. Pairs mean two-way module pairs from reported cycles under the selected analysis settings; cycle size is the largest reported cycle intersecting the recipe's scope (or the largest overall when no scope is given). An instability delta is after minus before. `null` means unavailable, **not zero**. Semantic steps retain unknown predictions rather than pretending graph-only edits implement an interface or split.
- `verify`: a readable `predicate` plus an executable `structured` object. It can require absent findings/edges, present/absent graph files, cycle counts, forbidden counts and the measured expected deltas.
- `confidence` (`high`, `medium`, `low`) and `risk_notes`. Confidence concerns the architectural suggestion, not a guarantee that the code builds.

Cycle recipes prefer a simulated registrar move when it wins, otherwise a light cut side or the existing break set. Forbidden imports produce a cut or a medium-confidence consumer-owned interface location in an allowed layer. Deep imports can add explicit reexports and redirect the import to an entrypoint. Grab-bag splits include available co-change groups as evidence; missing co-change is disclosed and structural groups are not a proven packaging plan. Interface extraction and module splitting require agent work.

`apply --dry-run` returns `can_apply`, `pending`, file moves and exact before/after source buffers with fingerprints. It does not edit source or staging. A plan with unresolved agent work exits **2** and names what to resolve. `--yes` does not override those refusals or apply a partial recipe. Source fingerprints, destinations, import resolution, bindings and path safety are checked before changes. A saved recipe's optimistic `mechanical` flag is not permission to skip this preflight.

Mechanical support is deliberately conservative. TypeScript/JavaScript and Python moves rewrite accounted-for static imports; import rewrites replace only the AST-captured specifier bytes, preserving surrounding comments, whitespace, quotes, Unicode and line endings. Named reexports are supported for TS/JS and Python; Python `__all__` changes need agent work. TS/JS side-effect imports and unused explicitly bound imports in TS/JS, Python and Go can be cut when the syntactic checks permit it. Go cross-package moves, Rust/Java module or package declaration changes, live binding changes, wildcards, escaped/computed paths, ambiguous matches and uncertain imports remain instructions. Elixir file moves can preserve module-name imports, but macros and runtime behavior still need project validation. Parsing a file is not proof that moving it preserves its language's package semantics.

Successful application runs `git mv` through a private Git index, edits only planned source files, rescans and checks actual deltas against predictions before publishing staging changes. Existing staged entries are preserved; moves are staged, while specifier edits remain worktree edits. Inspect **both** `git diff --cached` and `git diff` before staging the final change. This command does not commit. A failed scan or verification rolls back the recipe's edits; a concurrent user edit is preserved and reported as a rollback conflict rather than overwritten. A verification mismatch returns expected/actual checks and a `verify: FAILED` hint; it is not reported as successful application.

Live `propose` stores a before-scan under `.boundrail/proposed/`; successful `apply` stores a receipt under `.boundrail/applied/`. Keep this local state out of commits. CLI `verify <finding_id>` requires the successful apply receipt. For manual agent steps, call `propose` **before editing**, then pass the returned `verify.structured` JSON as `boundrail verify <finding_id> --expect '<JSON>'` or MCP `verify({expect: <structured>})`. Its `baseline_id` locates the original measurement; do not invent a replacement baseline after the fix. Changed configuration or stale receipt fingerprints require a fresh, explicit verification contract. Verification exits **0** when predicates pass and **1** when they fail; setup/input failures exit **2**. Always run the language build/type checker and tests as well: graph predicates cannot establish runtime correctness or initialization behavior.

Cycle recipes exclude test and structural namespace edges. Unsupported Rust/Java/Elixir import cuts are non-mechanical from the initial proposal, rather than waiting for apply preflight to reject them. Numerical predictions remain unavailable for such semantic work; the source-aware apply planner is still the authority for mechanically executable edits. Recipe generation computes forecasts before attaching generated recipes to the graph, avoiding repeated copies of previously generated recipes without changing IDs, ranking or predicted deltas.

Findings now sort by `hotspot × confidence weight × max(pairs_removed, 0)`, with confidence weights **1.0 / 0.6 / 0.3** for high/medium/low. Without hotspot evidence the factor is **1.0**; unknown pair reductions contribute **0**. Stable kind/subject/ID ties make ordering deterministic. Add global `--rank legacy` to restore hotspot-first ordering with canonical ties. Ranking does not change graph measurements or finding IDs. See the generated [pinned showcase recipe table](../showcase/recipes.md) for top-recipe proposals and actual dry-run outcomes; a successful dry run is import-level readiness, not a completed refactor.

## Warm graphs for agents

```sh
# Run in a dedicated terminal, from the checkout agents will edit.
boundrail serve . --engine native --symbols
# In another terminal, in the same checkout and environment:
boundrail status . --brief
boundrail check .boundrail/daemon-graph.json --brief
boundrail context --graph .boundrail/daemon-graph.json --root . --task "change billing" --budget 8k --brief
boundrail explain src/billing/invoice.ts --graph .boundrail/daemon-graph.json --brief
```

`serve [root]` warms the graph, watches the checkout and answers local commands serially. It accepts the same collection options as `scan`; use matching engine, rules, exclusions and symbol options when requesting a cached scan. The reference engine remains the default. Start with `--engine native` for incremental TypeScript/JavaScript, Python and Go collection; the other native languages and reference collectors use a logged full-collection fallback. No dependency-cruiser partial overlay is assumed to preserve reference semantics.

The default Unix socket is `<root>/.boundrail/daemon.sock`. `--socket <path>` changes it; `--port [number]` selects authenticated loopback TCP (`127.0.0.1`), with an omitted number or zero choosing an available port. Use TCP on platforms without Unix sockets. The CLI discovers `.boundrail/daemon.json` from the requested root or working directory and its ancestors. The descriptor contains the connection token; keep it local and do not commit it. Unix descriptors and sockets use owner-only permissions. Transport is JSON-RPC 2.0 with method `command`, CLI arguments and caller context, framed by a four-byte big-endian JSON length; frames are capped at 16 MiB. Use the CLI to construct requests and retain output/exit-code handling.

`scan`, `report`, `check`, `simulate`, `context`, `impact`, `surface`, `explain` and `status` automatically use a matching running daemon. `--no-daemon` forces standalone execution. If the daemon cannot be reached, normal standalone execution continues; stale sockets therefore do not make the CLI unusable. Restarting `serve` recovers stale endpoints. If a response is lost after a request was sent, the command may already have written artifacts: inspect them before retrying. An environment mismatch asks you to restart the daemon in the caller's environment or use `--no-daemon`.

The daemon maintains `.boundrail/daemon-graph.json` as its current graph. It uses a disposable `.boundrail/daemon.index` for Git status caching; the original staging index is not rewritten. Pass the live graph path explicitly to analyze live edits. Ordinary saved graphs retain their snapshot semantics. `report` still writes its JSON and HTML artifacts, and its default Git provenance probes can add work beyond the warm graph; use `--no-provenance` when measuring graph latency. Automatic forwarding does not guarantee a cache hit when scan options differ. The same input, options, environment and terminal mode use the same command implementation and output bytes through either route.

Changed source files are reparsed individually on the incremental path. Import resolution and barrel propagation are composed against the complete cached source index; these steps can still scale with the repository. Go package membership changes require metadata refresh. A dirty dependency region uses a conservative weakly connected closure when recomputing SCCs, so joining or splitting a cycle cannot hide a component outside a previous SCC. Structural/configuration changes, watcher overflow, unsupported collectors and uncertain dependencies trigger an explicit full-refresh log. Read-only filesystem events and private-cache writes are filtered before queuing, so a full scan cannot overflow its own watcher queue. Refresh failures return an error rather than presenting the previous generation as current.

At most **two graph generations** are retained (current and previous), alongside source/resolver and analysis indexes. This bounds snapshot count, not resident memory in bytes; memory still grows with repository size. `boundrail serve . --engine native --once` warms, answers one request and exits without a watcher; launch it in the background, wait for the descriptor/readiness message, then issue a matching CLI command. This mode validates the serving path in CI; it does not watch edits.

### Per-edit agent checks

`boundrail hook install --agent claude|codex --root .` installs project-local pre-edit context and post-edit/Stop checks. Add repeatable `--scope 'src/**'` paths or globs to restrict accepted workspace changes. Hooks use native collection with symbols, select an existing `layout.yaml`, and accept `--language`, `--package`, `--depth` and `--rules`. Start `boundrail serve . --engine native --symbols` with matching options/environment for warm checks. Idempotent installation prints what it wrote.

Pre-edit context is capped at 400 UTF-8 bytes, conservatively below 400 tokens. New forbidden imports, new/grown cycles, declared public-entry bypasses and out-of-scope changes block with recipes; pre-existing problems pass. A blocked baseline survives repair attempts. Policy changes while pending return `UNAVAILABLE`. Blocks leave edits on disk. Mechanical recipes are saved for `apply`; semantic instructions require agent work and project tests. Known read-only tools skip edit transactions; shell and unknown tools are checked. Concurrent edits are refused.

Claude includes failed-tool checks and `onFailure: block` for supported events (requires Claude Code v2.1.295+); Stop crashes/timeouts remain a host limitation. Codex hooks require explicit trust in `/hooks`; installation does not grant trust. Pre-tool context is consumed on the next model request and cannot replan an already-generated call. A same-session Stop validates an unfinished transaction left by cancellation/permission denial. Cursor and OpenCode require `--feedback-only`: their completion hooks cannot provide the same veto, and allowed pre-tool context injection is not established. See [host contracts and the scripted loop](agents/loop.md).

`BOUNDRAIL_BYPASS='reason'` accepts current violations and appends an override record. It explicitly collects standalone because the override changes the caller environment. Approve a specific finding with `hook check --baseline SNAPSHOT_ID --allow-once FINDING_ID --reason 'reason'`; the same finding cannot be approved twice in that snapshot. `hook reset --baseline SNAPSHOT_ID --reason 'reason'` releases exactly one abandoned transaction with an audit record. Neither edits source. Every block appends rule/file/agent/UTC timestamp records to `.boundrail/fires.log`; `boundrail fires --top` counts them by rule. Overrides go to `.boundrail/overrides.log`. Malformed logs cause errors rather than being overwritten.

Scripted integrations call `hook capture --agent codex --files src/example.ts`, then `hook check --baseline ID --brief` or MCP `check_diff({snapshot: ID})`. Snapshot IDs and Git `base` are mutually exclusive; existing Git comparisons retain their behavior. MCP snapshot checks are read-only and do not consume transactions or add fire records. The complete CLI check prints `PASS`/`BLOCK`, then recipe JSON, and exits 0/1; setup errors exit 2. Native hooks return host JSON, including `UNAVAILABLE` on setup errors. Inspect MCP truncation markers before claiming complete evidence.

Inventory includes gitignored workspace files and deletes, excluding `.git`, `.boundrail`, dependency installations and language-specific build output. Explicit edits into omitted directories are refused. Shell changes inside omitted directories are outside this scope check; the guard is not an OS sandbox. Explicit layout includes retain a broader build inventory. Source manifests bracket collection to detect concurrent changes. Unreachable daemons use standalone collection; authenticated/configuration errors require correction or explicit `--no-daemon`. Snapshot artifacts remain under `.boundrail/hooks/` without automatic retention. Keep `.boundrail/` out of commits.

### Brief output and explanations

`--brief` is a global agent-oriented output option. Structured commands emit `schema: "boundrail/brief/v1"`, with `kind`, `result`, full top-level array `counts`, explicit JSON-pointer `omitted` records and a hint for requesting the complete output. Limits are **20 array entries across the result**, 240 value nodes, nesting depth 8 and 512 characters per string. Per-module metric dictionaries in scan/simulation summaries are omitted explicitly to retain findings and summary counts. The brief wrapper is a projection, not a complete v1/v1.1 graph; remove `--brief` before feeding output to graph-reading commands. Output files remain complete graph/report documents even when stdout is brief. Text-only commands use a bounded text projection. Brief mode overrides Markdown display for context, impact and surface.

`boundrail explain <subject> --graph <graph.json>` accepts an exact repository-relative file path, module ID or finding subject. It returns imports, importers, relevant cycles and findings, with owning module metrics for files. It does not infer runtime dispatch beyond the collected static evidence. `boundrail status [root]` reports whether a daemon is running and its graph counts; a stopped daemon includes the command to start it. `check --brief` reports cycles alongside policy evidence, while retaining existing exit semantics: forbidden imports exit 1, valid policy checks exit 0, input/setup errors exit 2. Merely having a cycle does not change that policy exit code.

`boundrail explain <file> --for-agent --graph <graph.json>` emits compact `boundrail/explain-agent/v1` JSON bounded to 400 UTF-8 bytes, including its framing. This conservative byte cap stays below a 400-token edit-context allowance without adding a tokenizer. It reports aggregate import/importer counts, matching rules and available module/layer facts; `omitted: true` makes the summary's missing detail explicit. A new file is labeled unscanned and can still receive matching layout evidence. `--brief` does not add another wrapper to this bounded format. Detailed `explain` output remains unchanged.

Scan `source.generated_at` is reproducible: integer `SOURCE_DATE_EPOCH` when set, otherwise Git HEAD commit time, otherwise `1970-01-01T00:00:00Z`. It identifies source revision time rather than execution time. Invalid or out-of-range environment values fail with a correction hint. Identical source, Git state, collector/dependency environment and options therefore produce identical scan bytes; changing the checkout's dirty state or dependency evidence remains a real input change.

### Daemon benchmarks

The [recorded results](../benchmarks/daemon-results.json) use a frozen release binary with `--engine native` on macOS 26.2 arm64 (14 reported CPUs). Three samples per showcase case; the table shows median milliseconds. Each checkout uses its exact [showcase pin and exclusions](../showcase/repos.json).

| Repository | Files | Standalone cold | Warm idle | One-file import edit | 50-file import edit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Next.js | 1,252 | 3,011.5 | 494.9 | 240.4 | 305.6 |
| Django | 706 | 1,079.5 | 82.6 | 343.3 | 398.1 |
| TypeORM | 496 | 1,119.2 | 63.3 | 90.5 | 119.0 |

Cold means a new standalone process performing full collection with `--no-daemon --brief scan`; OS filesystem caches remain warm. Idle and edited cases use the same brief scan through the daemon. Edits add new literal imports to existing non-barrel source files. Timing runs from the first file write until the response contains the new edge count; exact added pairs are checked after timing, and restores are verified before the next sample. These are end-to-end command timings, not parser-only results. Idle requests include the freshness barrier: Next.js's idle median exceeded its edit median in this short run, and no causal conclusion is drawn from that anomaly.

A separate reproducible **Git-backed 2,000-file fixture** has 100 modules and 1,999 imports forming a DAG. Its body edit changes an initializer; its import edit adds a forward dependency without a cycle. An initial three-sample run and a follow-up checking host-load sensitivity after validation are both retained in the artifact; the table pools all six observations. P95 uses nearest rank.

| Fixture case | Samples | Median ms | P95 ms |
| --- | ---: | ---: | ---: |
| Body edit → fresh `status` generation | 6 | 55.6 | 59.0 |
| New import → fresh `--brief scan` | 6 | 81.8 | 90.2 |

The measured fixture cases reached the below-100-ms target. This does not establish that every command or repository stays below 100 ms: reference collector fallbacks, full resolution composition, connected graph regions, Git probes and artifact serialization can cost more. Showcase samples accompanied final validation; all numbers are workstation observations with small sample counts, not latency guarantees. Raw timings, phase counters, sampled RSS, both fixture runs, commit pins and the binary/source hashes are in the result artifact. RSS is sampled outside timing and is not a peak-memory measurement.

Reproduce from the repository root (the runners require Python 3 and Git):

```sh
cargo build --release --locked
ci/bench-daemon.sh --boundrail target/release/boundrail --repeats 3 --output /tmp/daemon-results.json
python3 -B ci/bench_daemon_2k.py --boundrail target/release/boundrail --repeats 3 --output /tmp/daemon-2k-results.json
```

The showcase runner clones isolated checkouts; `--cache /path/to/showcase/clones` reuses local Git objects without editing the originals. The fixture runner creates and commits its generated sources using command-local Git settings. Both restore edited files, stop their daemons, and retain their work directories and logs for inspection.

## Commands

`scan`, `report`, `history` and `impact` run collectors. `infer-layout` drafts rules, `context` selects reading context, and `tour` prints a reading order from saved graphs without running collectors. `services` analyzes a separate declarative service-call inventory without scanning source code. Other analysis commands read a scan document and recompute modules, metrics, cycles, verdicts and findings from its files, edges and config with the same code, so their output agrees with the rules in effect.

**scan** writes the scan document (`kind: scan`).

```sh
boundrail scan . -o graph.json                        # language detected, modules = 2 folders below src/
boundrail scan . --rules layout.yaml -o graph.json    # with edge verdicts
boundrail scan . --cochange -o graph.json             # also read the last 300 commits (--cochange=N, N >= 1)
```

A module is a file's directory truncated to `--depth` segments below the source root (`src/` when present, else the root; for Python, the package directory). Each file directly in the source root is a module of its own, named by its file name with extension (`main.ts`, `errors.py`), so a package's lowest layer and its entry points never merge into one module (in Go the root's files are one package, so they form the module `.`). Imports between two such files are therefore cross-module: the same-module rule no longer allows them, so under `default: forbidden` they need an `allow` or `layers` rule. Scan documents written by commit 2bf5805 put these files in module `.` and fail `check`, `render` and `simulate` as inconsistent: rescan them. When the root has a `src/` directory, only `src/` is cruised (files outside it appear when `src/` imports them, with module ids prefixed `../`: `../lib`, and `../tool.ts` for a file directly in the root). TypeScript/JavaScript scans keep `.ts/.tsx/.js/.jsx/.mjs/.cjs` files and Python scans keep `.py` files; other collectors keep their language’s source files, including `.ex`/`.exs` for Elixir. Imports of stylesheets, JSON and other assets are dropped. Edges distinguish `runtime` (load-time imports), `lazy` (`import()` or imports inside functions), `type` (type-only imports) and `reexport` (structural re-exports). Type-only re-exports use `kind: type` with `is_reexport: true`. Metrics weight these kinds separately; rule verdicts consider every kind unless filtered. Cycles use runtime edges by default, with other cyclic components accounted for separately (see [Edge quality and migration](#edge-quality-and-migration)). For Python, an import that appears only under `if TYPE_CHECKING:` is `type`, and one that appears only inside function or method bodies is `lazy` (grimp's import lines, with the file parsed by the standard library's `ast`, never run). For TypeScript, `package_entries` lists the files a `package.json` in the scanned tree publishes through `exports`, `main`, `module` and `types`, mapped from build output (`dist/`, `build/`, `lib/`, `.js`, `.mjs`, `.cjs`, `.d.ts`) back to their sources; like barrels they are public entries. A composition root is a file nobody imports that imports 3 or more other modules (`app.ts`, `cli.ts`), a registrar a file with 5 or more imports into one module, or with imports into 5 or more sibling modules under one parent folder other than the root (code choosing one of many drivers): their imports are wiring, left out of sink-split consumers, unstable-dependency counts and two-way cuts.

**What an edge means.** A reference edge goes from a file to the file declaring something it uses. An import edge goes from a file to the file its import statement names, whether or not the code uses the name.

| Language | Engine | Edge | Unused import gives an edge | Type-only uses |
| --- | --- | --- | --- | --- |
| Go | reference (`go/types`) | reference: each function, type, variable, constant, field and method used, within a package too | cannot occur (the compiler rejects it), and a blank import leads to the package's entry file | not separated, every edge is `runtime` |
| Go | native | reference: `pkg.Name` selectors, same-package names at `low` confidence | cannot occur, and an import with no resolved selector leads to the package's entry file at `low` confidence | not separated |
| Java | reference (`javac`) | reference: each type, field, method and constructor used, including through static imports and method references | no | not separated |
| Java | native | reference: type names, static-imported members and `Type.member` or `Type::method` qualifiers, resolved through the imports | no | not separated |
| Rust | reference (SCIP) | reference: each workspace symbol used, and trait implementations | yes: a `use` is a reference | not separated |
| Rust | native | reference: `use` declarations and module-qualified paths | yes | not separated |
| Elixir | reference, native | reference: static remote calls and structs, plus each `import`, `require` and `use` | an unused `alias` gives none, `import`, `require` and `use` always give one | `@spec` and `@type` references are `type` |
| TypeScript | depcruise | import | yes | `import type` and imports the compiler drops are `type` |
| TypeScript | native | import | yes | `type` only when every imported name is a type used only in type positions |
| Python | grimp, native | import | yes | an import only under `if TYPE_CHECKING:` is `type`, one only inside functions is `lazy` |

For a call-level claim such as "a service never calls a controller", a `forbid` on the files is enough. In Go, Java, Rust and Elixir it already catches calls and field uses. In TypeScript and Python it catches the import, which is the coupling: an unused import it flags is still a true positive, and the fix is to delete it. `kinds: [runtime]` leaves out type-only imports where the engine separates them. Go, Java and Rust cannot tell a call from a type use, so such a claim also forbids naming the controller's types.

Options: `--depth N` (default 2; for Java, each package is a module unless it is given), `--exclude <glob>` (repeatable), `--include-tests`, `--include-generated`, `--no-default-excludes`, `--language typescript|python|go|rust|java|elixir`, `--depcruise <path>` (a dependency-cruiser executable to run instead of `npx --no-install depcruise`; exit 2 when the path does not exist), `--python <interp>`, `--package <pkg>`, `--python-path <directory>` (repeatable), `--go <path>` (the Go toolchain, default `go`), `--elixir <path>` (the Elixir runtime, default `elixir`), `--rules <layout.yaml>`, `--symbols` (collect public surface evidence), `--cochange[=N]`, `--collector-input <recorded.json>`. A shared exclusion policy drops tests, generated/build output, dependency/vendor directories and fixture/story/example content by default; `boundrail show-excludes` prints its actual patterns and override precedence. See [Exclusions and included tests](#exclusions-and-included-tests) for category flags and ignore-file support. dependency-cruiser runs with `tsPreCompilationDeps: "specify"`, from a config boundrail writes to the temporary directory (the repository's own config is not read): an import of types or interfaces only, written without `import type`, is kept and marked, and becomes `type`. When dependency-cruiser crashes in that mode, the scan runs it with and without pre-compilation dependencies and marks the difference; when that fails too, these imports stay `runtime` and `source.collector.type_marking` is `unavailable`, which the page and `report` mention.

**Go.** Detection counts `.go` files (not tests, outside `vendor/` and `testdata/`) only when the scan root has a `go.mod` or `go.work`; the language with the most files wins, and a tie keeps TypeScript, then Python, then Go. The source root is the scan root (`src/` is a folder like any other). **Modules are folders:** as for other languages, `--depth` (default 2) merges the packages below each folder into one module, so `internal/platform/db` and `internal/platform/cache` are both in `internal/platform` and show as sub-discs of it on the map; the root's own files are the module `.`. A large `--depth` (`--depth 99`) makes each package its own module. Scans written before this default have `config.package_modules: true` and keep one module per package. Go rejects import cycles between packages, and since a package compiles as a unit, files of one directory form no file cycle with each other either: a module cycle in Go is a grouping cycle between the packages a module merges, never one Go compiled. Every `go.mod` below the root with a `module` line is a module of the scan (one without, or an empty one, is skipped with a warning; `--exclude` matching `<dir>/go.mod` skips it), so modules joined by `replace` or merely sitting side by side resolve to each other's files. A file is a file of its package when `go list` puts it in `GoFiles`, `CgoFiles`, `TestGoFiles` or `XTestGoFiles`, or in `IgnoredGoFiles` without needing the `ignore` build tag: every platform's files at once (`foo_linux.go` and `foo_windows.go`), whatever GOOS boundrail runs on, and cgo files although cgo is off. When `go list` fails in a module (it needs that module's dependency graph for some `replace` directives, which `GOPROXY=off` cannot fetch when the module cache lacks it), the scan warns and reads that module's packages with `go/build` instead. Each package is type-checked with `go/types` (the standard library through `go/importer`, the repository's own packages from source, other modules not at all), and an edge goes from a file to every file declaring something it uses: a function, type, variable or constant, a field, or a method, so `s.db.Get()` leads to the file declaring the field `db` and to the one declaring `Get`. A name declared in several platform files leads to all of them. A blank import (`import _ "x"`) leads to the package's `doc.go`, else `<dir>.go`, else its first file, picked among the files that are neither tests nor generated when there are any. An import none of whose uses resolved (they sit inside an expression whose type comes from a module that is not loaded) gives no edge; it counts in `source.collector.unresolved`, with imports that look local (under a scanned module path) but name no package in the repository. Every Go edge is `runtime`, including those between files of one package, which keep cohesion and `misplaced-file` meaningful. `source.collector.main_packages` lists the directories of `package main` packages and `bound` the pairs of files that cannot leave their package and still compile: `{from, to, names, together}`, where `from` uses the unexported `names` that `to` declares (a field or method as `Type.name`, named by the type it is selected on, so a promoted field `s.x` of an embedded type is `Server.x` when `s` is a `Server`; a composite-literal key by its literal's type, `DB.values`), and `together` marks a file holding a method of a type declared in the other, or declaring a package-level name the other declares too (platform variants under exclusive build constraints, `notify_linux.go` and `notify_windows.go`). A pair counts only when both files are scanned, so a method in an excluded test binds nothing, and a collector limitation counts the pairs left out that way. The native engine cannot tell these pairs and lists every file in `bound_files` instead. `misplaced-file`, `sink-split` and registrar moves never propose moving them, and `misplaced-file` never proposes moving a public file (outside `internal/`, not `package main`), which other modules may import, nor a file that has edges within its own package without using the target module itself: such a file is a piece of the library its package offers, whoever uses it now. `seam` is not reported for Go: file clusters would split packages, which Go cannot do. `go_modules` lists the directories holding a `go.mod` (`.` for the root): no move is proposed into another Go module, which would change the file's module path and its module's published API, so a `misplaced-file` whose only destination lies in another module is not reported. `internals-reach` comes only from `public` rules, since the compiler already rejects imports into another tree's `internal/`. The default excludes add `**/*_test.go` (tests), test support (kept with `--include-tests`, and named in a warning): packages named `*testing` or `*testutil`, and each non-test file that uses package `testing` for more than `testing.Testing()` when no production file of another package uses it (a stub beside production code that others use stays) and, if it has variants for other platforms, every variant is such a file too; the other files of its package stay, `**/*.pb.go`, `**/*.pb.gw.go`, `**/zz_generated*.go` and every file with a `// Code generated ... DO NOT EDIT.` header (generated code); `vendor/`, `testdata/` and folders starting with `.` or `_` are never scanned. Reflection, `go:linkname`, `go:embed`, assembly and C are invisible.

**Rust.** Detection counts `.rs` files when the scan root has `Cargo.toml`, ignoring `target/`. Scans resolve cross-file workspace symbol references and trait implementations through `rust-analyzer scip`; local symbols stay out of the file graph, and recorded external imports use separate package nodes. Paths are relative to the scan root and ordinary folder depth/rules determine modules. `--rust-features all|default` chooses enabled Cargo features (default `all`). Build scripts and proc macros are disabled by default; `--rust-build-scripts` enables them and executes repository code. The collector uses cached dependencies offline and refuses repository toolchain/program overrides; missing dependencies fail with a fetch instruction. Generated references can be missing with build scripts disabled. SCIP can emit references inside inactive `#[cfg(test)]` code even when test definitions are omitted; inline test references can remain in architecture edges. `--include-tests` controls file exclusions only. Some `pub extern crate` facade links are absent from SCIP and therefore missing from the graph.

**Java.** Detection counts `.java` files, ignoring tests and build output. Without `--depth`, each source directory is a module (`config.package_modules: true`); an explicit depth groups folders from the scan root. References resolve to declaring files, including same-package, qualified, static and inherited references. An import alone gives no edge, so an unused import has none. Edges are `runtime`. The compiler receives only selected sources, no external classpath or annotation processors. Missing external libraries, unsupported syntax or module layouts can leave a partial graph; `source.collector.unresolved` retains compiler error diagnostics and produces a warning; import-statement accounting is separate. Default excludes add `target/`, `out/`, `test/`, `*Test.java` and `*Tests.java`; `--include-tests` keeps tests. Hidden directories and node_modules are never traversed. Kotlin is not supported.

**Elixir.** `boundrail scan . --language elixir --elixir elixir -o graph.json` reads `.ex` and `.exs` files under the scan root, including `apps/*/lib` in umbrella projects. The source root is the scan root (`config.source_root: "."`), so `lib/`, `apps/` and `src/` remain ordinary folders. Modules use the usual folder depth or explicit layout rules; they do not correspond one-to-one to Elixir `defmodule` names. Detection recognizes `mix.exs` and counts eligible source files alongside the other supported languages; use `--language elixir` in a mixed repository when needed.

The parser records module definitions and resolves scoped aliases (`as:`, grouped and nested aliases, and `__MODULE__`) when recording `import`, `require`, `use`, static remote calls and struct references. An unused alias alone does not add an edge. References in `@spec` and `@type` are `type` edges. Other static references, including compile-time `use` and `require`, conservatively use `runtime`: this label is a graph-analysis category, not a claim about Elixir's compilation phases. A module defined in multiple files produces edges to each definition because source order alone does not establish which definition will be compiled. Dynamic and unknown references in module-required positions remain in import accounting as unresolved. Unknown bare alias atoms used as data are omitted; table names and namespace prefixes do not establish module dependencies. Internal edges are deduplicated by file pair and kind; the ledger retains individual dependency-reference occurrences.

External identities are OTP application names proven by installed `.app` module-ownership metadata; they are not inferred Hex package names (the names often agree, but are not guaranteed to). A prefix such as `Phoenix` does not establish a package identity. Uninstalled dependencies therefore remain unresolved, and no dependencies are downloaded. Excludes add `_build/`, `deps/`, `*_test.exs` and `test_helper.exs`; `--include-tests` includes test files, while `--include-generated` controls build/dependency exclusions. Existing graph analysis, rules, simulations, history, impact, context and reading tours operate on these file edges. Public symbol/surface collection is not implemented for Elixir. Quoted code, generated modules, macro expansion and dynamic dispatch can hide dependencies: the graph is a lower bound, not a complete compile/runtime dependency graph.

**report** is the one command for people: it scans with the same options as `scan`, applies rules from `--rules`, else from `<root>/layout.yaml` when it exists (it prints which file it used), writes `boundrail.json` (the scan document) and `boundrail.html` (the `render` page) into `--out-dir` (default `<root>/.boundrail/`; add it to `.gitignore`; it refuses to write when that folder or either file is a symbolic link, which a scanned repository could plant to aim the write elsewhere), and prints a short summary built from the scan document: files, modules and imports; with rules, forbidden and unplanned counts by rule; module cycles with the largest one's size, any leading registrar move (for a `grouping` cycle with a sentence naming the files the moved one shares its module with, and for an `entrypoint` a reminder to update deploy and config references), a line for each `grouping` cycle without a move (for Go, that packages are acyclic and the cycle exists only at the configured folder depth, so change `--depth` or the module grouping; three lines at most, then a count), and the break set of the cycle holding the lightest two-way pair, as imports to cut grouped by importing file (top three, then how many more; "approximate" when `exact` is false); findings by kind with counts and up to three subjects each; then the two paths, the page last. Headings are bold only when stdout is a terminal and `NO_COLOR` is unset. Exit codes are those of `check`: 1 when an import is forbidden and not in the known list, or a listed one no longer occurs, 0 otherwise, 2 on errors.

```sh
$ boundrail report . --out-dir /tmp/arch
boundrail report: .
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
  /tmp/arch/boundrail.json
  /tmp/arch/boundrail.html
```

**check** prints forbidden and unplanned imports. Exit 1 when any import is forbidden and not in the [known list](#known-violations), or a listed one no longer occurs; 2 on usage or input errors.

```sh
$ boundrail check graph.json --rules layout.yaml
unplanned src/domains/cart/controller.ts -> src/lib/clock.ts (none)
forbidden src/domains/checkout/controller.ts -> src/domains/checkout/client.ts (browser-no-server) via src/domains/checkout/client.ts -> src/domains/checkout/model.server.ts
forbidden src/shared/format.ts -> src/domains/cart/types.ts (shared-no-domain)
2 forbidden, 1 unplanned
```

`--format sarif` prints SARIF 2.1.0 for GitHub code scanning instead (`-o` still writes the document; the agent-rule and early-smell checks are skipped; exit 1 as for text, else 0): a result per import that breaks a claim, ruled by the claim id, then one per other forbidden import, ruled by its rule (`forbid[i]` when unnamed, so give rules an `id`), each with its rule's reasons and sources. A result points at the importing file under `source.scan_root` and its line; without a line it covers the whole file and its message says so. Its fingerprint hashes the rule, file and target but not the line, so an alert survives the import moving within its file.

**simulate** compares forbidden imports, unplanned imports, cycles and module metrics before and after moving one or more files (`kind: simulation`, with full `before` and `after` summaries). Repeat `--move FILE DEST` to apply several moves together. The destination path decides the new module and layer.

```sh
boundrail simulate graph.json --move src/utils/dates.ts src/calendar/dates.ts
```

**render** writes one self-contained HTML page. When the scan has a target (claims or moves), it opens on a review page: a verdict in sentences (the slices that follow every claim, the slices moved, the imports that break a rule, any cycles) beside a title block with the scan's path, language, collector, commit and rules, then a table of claims by slice (a tick where the claim holds, a red cell with the number of broken imports, hatching where the rule can't fail) whose red cells open the map on that slice's files and imports, a move cost table (files, import lines, names to export, imports in new package cycles, moved; sortable by any column) whose slices open the map with their destination folders, and the side panel's findings to review and next steps. The header's Review and Map switch changes view and is kept in the URL as `#review` or `#map`, so a link opens the same view; without a target the page opens on the map. The side panel opens with an assessment read from the scan: what holds up, what hurts (ranked by how much of the code it touches) and at most seven next steps, each with its numbers and a link to its evidence; each module panel opens with its role (foundation, hub, entry point and so on) and what is good, risky and next. Cohesion there is judged without imports into foundation modules, using `modules[].outgoing_to`. Below that is a zoomable map of modules as circles whose ring shows what boundrail checked (red: on either end of a forbidden import, or holding an import to cut or a registrar move for a module cycle; amber: the other members of a module cycle, or named by a finding; grey: nothing found), holding their files grouped by folder, with module imports as curved arrows (red forbidden, amber unplanned, dashed type-only). Zoom in (wheel, pinch, `+`/`-`) and the files, their imports and their names appear. Hover or click a module for its importers and imports, a file or an import for its verdict and rule; the side panel lists findings by kind and draws suggested moves with their before and after numbers. Search, filters for forbidden, unplanned and cycle imports, and hiding busy modules keep large graphs readable. The page makes no network requests.

```sh
boundrail render graph.json -o graph.html
```

**diff** compares two scan documents (`kind: diff`): per-module metric pairs `[old, new]`, added and removed modules, new and resolved cycles, new and resolved forbidden imports, new unplanned imports.

```sh
boundrail diff before.json after.json
boundrail diff before.json after.json --fail-on new-forbidden,new-cycles --format markdown >> "$GITHUB_STEP_SUMMARY"
boundrail diff before.json after.json --html changes.html
```

- `--fail-on <list>`: comma list of `new-forbidden`, `new-cycles`, `new-unplanned`, `stale-known` (an entry of the new scan's [known list](#known-violations) that no forbidden import matches, listed under `stale_known` and in markdown as `### Known entries no longer forbidden`), `new-vacuous` (a [claim](#claims) vacuous in the new scan that the old scan states and that is not vacuous there, listed under `target.newly_vacuous` with the claim's notes and in markdown as `### Newly vacuous claims`; a claim the old scan lacks never counts); any other value is a usage error (exit 2). Exit 1 when any listed condition is in the diff, else 0. The document is printed either way.
  - A **new forbidden** (or **new unplanned**) import is a file pair `from -> to` that is forbidden (unplanned) in new and is missing, or has another verdict, in old.
  - A **new cycle** is an entry of `new_cycles`: a cycle in new whose level and member set appear in no cycle of old. Cycles are strongly connected components, so a cycle that grows (`a <-> b` becomes `a -> b -> c -> a`) is new, and its old member set is listed in `resolved_cycles`; a cycle that keeps its members while its imports change is not new. Both module cycles and file cycles inside one module count.
- `--format json|markdown` (default `json`): `markdown` prints a compact summary for a pull request or CI job summary instead of the document: a pass or fail headline naming the failed `--fail-on` conditions, new forbidden and unplanned imports (`file → file (rule)`), new and resolved cycles, a `Target: 15 of 18 slices conform (was 16 of 18), 0 of 23 slices moved, 1 newly broken slice.` line when either scan has a claim or a move ("was" only when the count changed, "moved" only when both scans have moves, the broken part only when non-zero), then a `### Newly broken slices` list of each slice a claim newly breaks with its imports; the JSON document has the counts as `[old, new]` pairs of `{done, of}` and the broken slices under `target`, a table of modules whose cohesion or instability moved by 0.05 or more (largest move first, at most 20 rows, then "and N more"), and the numbers of added and removed modules. Paths and module ids are backslash-escaped (backslash, `|`, backtick, `<`, `>`, `*`, `_`, `[`, `]`) and line breaks become spaces, since they come from the scanned repository.

### Visual architecture diff

`diff BEFORE AFTER --html FILE` writes an offline import comparison alongside the normal JSON or
Markdown result. Failure gates still inspect the full diff. A new forbidden import can therefore
exit 1 while leaving its comparison page available for review. Service comparisons and
`--legacy-v1` do not support HTML in this increment.

The module graph and searchable list show added and removed imports, changed rule verdicts,
module assignments, and source evidence. Select a row, module, or graph connection to inspect
before and after evidence. Removed relationships remain inspectable. A line-only move is labelled
"Evidence only". A module with different files is labelled "Membership changed". Package and host
global dependencies remain distinct from local modules. Changes and All imports views keep global
totals separate from filtered counts.

The header discloses policy changes and recorded collection compatibility. Different known
languages, collectors, scope, or collection settings fail before export. Missing provenance is
shown as unverified even when no changes are observed. Recorded analyzer versions are read before
recomputation. Matching recorded settings do not prove repository identity, identical binaries,
or complete dependency coverage. Commit labels retain the recorded clean or dirty state.

The page is self-contained and fetches neither source nor remote assets. Repository strings stay
literal text. Output paths that alias either input are rejected. Existing import JSON diffs retain
their schema and semantics, including their per-snapshot policy verdicts. The HTML output directory
must already exist. Policy-only rule effects are identified separately in the visual view.

`check`, `simulate` and `render` take `--rules layout.yaml` to replace the rules embedded in the scan.

## Service architecture

`boundrail services <directory>` infers service boundaries, infrastructure clients, HTTP calls and
queue topics from local source. It does not run the application or contact infrastructure.
TypeScript, JavaScript, Go, and Python are supported. Reports stay incomplete because configuration,
indirect calls and dynamic values can hide evidence.

```sh
boundrail services . --format json
boundrail services . --manifest services.yaml --output services-report.json
boundrail services examples/services.yaml --format json
```

With no path, the command reads `./services.yaml` when that file exists, otherwise it infers `.`.
A file path always reads a declarative YAML or JSON manifest, preserving the existing report.


To inspect the model in a browser, export its JSON and render it:

```sh
boundrail services . --format json -o services-report.json
boundrail render services-report.json -o services.html
```

The offline page has a searchable service catalog, directed calls, per-service client usages,
and an evidence inspector. Select a service or call, then its source location to inspect or copy
`file:line`. Keyboard controls work in both the catalog and diagram. Narrow screens scroll the
diagram horizontally to preserve readable labels. No source files or remote assets are fetched.

The reader validates service reports and recalculates metrics and rule verdicts from their calls
and resources. `--rules` applies to import reports and is rejected for service input. Generate the
service JSON with `--manifest` to apply service rules before rendering. A report with no inferred
calls explicitly shows that state. It does not imply that the services have no connections.

Boundaries use `Dockerfile`, runtime `start` or `serve` scripts, application project targets, Go
`package main` with `func main`, and bound startup expressions. Build watchers do not count.
Nested application roots split a root image automatically. A root image without its own startup
or application facts outside child packages is dropped when nested applications are found. Startup commands targeting `build/server` or
`dist/server` map to the source `server` directory when present. A library's `main` field alone
never creates a service. Hidden directories, symlinks, tests, generated files, fixtures, examples
and ignored files are excluded. Each file belongs to its deepest service directory.

The infrastructure catalog recognizes common SQL, MongoDB, Prisma, TypeORM, Sequelize, Kysely,
Knex, Redis, Kafka, BullMQ, AMQP and SQS constructors in TypeScript. Go recognizes database/sql,
pgx, pgxpool, go-redis, and NATS clients. Python recognizes SQLAlchemy, psycopg, Redis, and Celery.
Each resource has a package, source file and line,
confidence, and rule verdict. A client constructed inside a service has high confidence. A client
in shared code has medium confidence for each service directly importing that file at runtime.
Unattributed facts are counted in the report's caveats.

Express, Fastify, Koa router, Hono and Nest routes can match fetch or axios calls. Go supports
net/http routes, calls and requests dispatched through a known client. Python supports FastAPI
and Flask routes, requests and httpx calls, and named Celery task topics. Literal methods and paths
match with high confidence. Parameter paths, static template tails and Go methodless routes have
medium confidence. A literal remote host must identify the target service, so a Stripe URL does
not become a local call merely because its path matches. Matching several services emits no edge.
Kafka, BullMQ and NATS topic matches point from publisher to consumer. Unmatched HTTP calls remain
resources.

`--manifest` applies only to a directory input. If its service IDs name existing directories,
those directories replace the entire inferred boundary set. Other IDs remain declared-only
services. A manifest containing only declared-only IDs preserves inferred boundaries. Declared
edges are appended and all edges use the same cycle and forbid analyzer.

Infrastructure rules use explicit package targets. Ordinary service globs judge only service
calls. Omit `kinds` for database and cache rules. A transport filter can match HTTP or queue
resources. Resource violations have separate `forbidden_resources` indices and counts and do not
change service cycles, hubs or instability.

```yaml
version: 1
services: [api, worker]
edges: []
forbid:
  - {id: worker-no-postgres, from: worker, to: 'pkg:pg', reason: 'Database access belongs to the API.'}
  - {id: worker-no-api, from: worker, to: api, reason: 'Workers consume jobs without calling the API.'}
```

To check the authored rules in a script or CI job:

```sh
boundrail services . --manifest services.yaml --fail-on-forbidden --format json -o services-report.json
```

`--fail-on-forbidden` emits the normal report and exits 1 when the full report contains a forbidden
call or infrastructure usage. Exit 0 means no violation was observed in the collected evidence.
It does not establish complete architectural compliance. This check includes existing violations.
It does not compare a baseline. Invalid input, a failed output write, or a check with no authored
forbid rules exits 2. Without the flag, a valid report exits 0 even when it contains violations.
`--brief` changes presentation, while `-o` saves the full JSON report. Neither changes the check.

To check only newly observed service violations, save reports before and after an edit:

```sh
boundrail services . --manifest services.yaml --format json -o .boundrail/services-before.json
# Make the code change, then collect again.
boundrail services . --manifest services.yaml --format json -o .boundrail/services-after.json
boundrail diff .boundrail/services-before.json .boundrail/services-after.json --fail-on new-forbidden --format markdown
```

Service diffs evaluate both reports under the after-report's ordered rules. JSON uses
`kind: services-diff` and separate new, existing, and no-longer-observed forbidden calls and
infrastructure usages. Each relationship retains its before and after source occurrences.
Line or confidence changes do not introduce a new relationship. `policy_changed` and the two
rule arrays disclose policy changes separately. A policy-only edit does not become a new code
violation. Exit 1 means a new forbidden relationship was observed. Existing violations do not fail
this gate. Exit 2 covers invalid or mixed report kinds, different collector modes, an unsupported
failure condition, or an explicit gate without after-report forbid rules.

The comparison uses supplied evidence. Service reports lack analyzer versions, Git revisions,
collection settings, and boundary-manifest digests, so collection parity remains unavailable.
No-longer-observed evidence does not prove a runtime relationship was removed. Import diffs keep
their existing output and failure conditions. Service diffs support only `new-forbidden` and do
not yet participate in edit hooks or MCP baseline queries.

Inference follows local bindings conservatively. CommonJS destructuring, clients hidden by
framework providers, cross-file receiver wiring, configured URLs or topics, and router plugin
prefixes can be missed. Python uses deployment or declared boundaries. Unsupported deeply nested
TypeScript receiver chains remain unknown. Repeated binding names can hide valid Python facts.
Runtime roles sharing one entrypoint remain one service. gRPC and calls across repositories can be supplied through the declarative manifest below.

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

JSON reports use `schema: boundrail/v1.1`, `kind: services`. Their transport-valued edges have their own schema definition; source-import kinds are unchanged. [Manifest schema](../schema/services.v1.schema.json) validates the input structure, while [the v1.1 schema](../schema/boundrail.v1.1.schema.json) validates reports. Cross-references and semantic constraints are additionally checked by Rust. Array indices in cycle and forbidden-call evidence are zero-based; Markdown numbers calls from one. Report source metadata says `collector: declarative`, `complete: false`.

This is a standalone report type. `render` accepts it, while import-only commands such as `check`, `simulate`, `tour` and `context` do not. There is no legacy-v1 service export. Valid reports exit 0 even when they contain cycles or forbidden calls; invalid declarations exit 2. JSON may be saved with `--output` while Markdown is printed; the output path must differ from the input manifest. All declared transports participate; file `--granularity` does not apply.

**Interpretation:** a call cycle is communication-coupling evidence. It does not prove a distributed deadlock or a deployment-order failure, especially when queues are asynchronous. The inventory can be incomplete or wrong. Review the displayed confidence and evidence before acting; this mode makes no completeness claim.

## Layout inference

`infer-layout` drafts rules from the architecture observed in a saved v1 or v1.1 graph. **The result is a draft for review, not a statement of architectural intent.** Folder names do not prove intent, and missing imports or excluded files weaken evidence that a dependency is absent.

```sh
boundrail infer-layout graph.json                       # print annotated YAML
boundrail infer-layout graph.json --apply               # create ./layout.yaml
boundrail infer-layout graph.json --apply layout.draft.yaml
boundrail check graph.json --rules layout.draft.yaml     # after reviewing/editing
```

Printing is the default and writes no files. `--apply` creates a new file and refuses to overwrite an existing file or symbolic link. Inference does not run collectors, inspect source, or change the graph.

The command condenses module SCCs and assigns each component its longest distance from an entrypoint in the condensation DAG. Evidence comments show the layers top to bottom: layer 0 has no incoming dependencies; deeper layers contain dependencies. Leaves can occur at different depths. The executable `layers` list remains **foundation-first**, as required by the existing rules format. Patterns at the same depth share one layer, and patterns come from actual file membership rather than module names. Cyclic module groups appear in `# unresolved: cycle between ...` comments and receive no layer or forbid proposals.

Inference uses **all import kinds**, because existing layer rules apply to all kinds. Consequently, a type/lazy/re-export cycle can remain unresolved here even when the default runtime cycle report is acyclic. Dependencies use their written boundary (`original_to` for resolved barrel imports), matching `check`. Tests and externals are outside the internal layering graph; written targets absent from the scanned files are counted as omitted evidence. External package policies use the separate import ledger described below.

Sparse-boundary proposals require at least two files on each side and **100 potential file pairs** by default. Density is distinct observed importing-file/target-file pairs divided by source files × target files, across all kinds. A density at or below **0.01** qualifies. Comments give the evidence and list existing violating dependencies with source lines when available: adopting such a rule means reviewing and fixing those imports. Proposals already implied by layer ordering are omitted. At most **20** forbid proposals are printed. Slice claims (below) come first, then external-free layer proposals; the remaining internal proposals are ordered by density, potential pairs and module IDs. All share `--max-rules`, and comments report omitted proposals.

```sh
boundrail infer-layout graph.json --min-potential-edges 500 --max-density 0.005
boundrail infer-layout graph.json --max-rules 0           # layers only
```

When import accounting is present and at least one known external package exists elsewhere in the scan, inference also proposes `to: "pkg:**"` for an acyclic layer with **zero external imports and zero unresolved imports**. It requires `layer file count × distinct package identities observed repo-wide` to meet `--min-potential-edges` (default 100). Comments show the factors; these are file/package opportunities, not measured missing edges. No package rule is inferred from an unavailable ledger or unresolved absence evidence.

Inference also drafts claims from a direction that holds across sibling slices. For a folder `root` with at least `--min-slices` (default **5**) child folders, each child is a slice and the next path segment its role: a folder is kept (`ui/`), a file is cut at its first `.` or `_` (`handler_documents.go` is `handler`), and the role's glob is `<root>/<name>/<role>{/**,.*,_*}`. When A imports B in at least `--min-slices` slices, and `B ↛ A` binds at least `--min-slices` slices and holds in at least `--min-share` (default **0.8**) of them, the draft proposes the claim `infer-claim-N` from B to A with the reason `B does not import A (drafted from the graph; review intent)`. Its comments give the counts, as `# internal/<name>: handler imports service in 18 of 18 slices; service imports handler in 0 of 18 (holds in 18).`, and list each slice it breaks in and the imports that break it. Counts come from the rule as `check` evaluates it and include tests. Claims are ordered by bound slices, then share; each role pair gets its own id, and merging claims under one id is a review step. No claim is drafted for a direction no slice shows, such as a layer skipped: add those by hand.

```sh
boundrail infer-layout graph.json --min-slices 3 --min-share 1
```

These thresholds limit over-fitting; zero observed imports still do not establish an intended boundary. Review the draft before making it a CI policy. No scan-schema or rules-format migration is required.

## Live what-if in the map

```sh
boundrail render graph.json -o graph.html
```

Open the HTML, enable **What-if**, zoom until files appear, and drag a file into a different module disc. The target module's common physical directory becomes the proposed destination folder; confirm it in the panel, especially for custom modules spanning several folders. The source basename is retained. A labeled file/folder form provides the same preview with keyboard or touch; it also accepts new folders.

**Add to list** keeps the previewed move in an ordered move list. Each later preview applies every listed move plus the new one together, from the original graph, as `boundrail simulate` does with repeated `--move`. Moving a listed file again, by its original or its previewed path, replaces that item's destination; moving it back to its original folder removes it. Destinations are checked against the scan, so a path an earlier move vacates cannot be reused, and two moves cannot share a destination. Each item has a labeled **Remove** button and **Clear all** empties the list. Previews never change files or the saved layout.

The panel compares total cycles (module and within-module file SCCs), largest module cycle, distinct two-way module pairs, forbidden/unplanned imports, and changed module instability. Map links and health rings show preview cycle/rule evidence. Original findings are hidden until preview mode closes: heuristics such as seams, placement, ownership, cut sets, public surfaces and package readiness are **not** recomputed in JS. A new module appears in results; module disc positions stay fixed. Clear restores original paths, positions, counts and health. Preview moves are not persisted to browser layout storage and never edit the repository. Ordinary dragging outside What-if still arranges the map.

A preview rewrites the path and import endpoints, including written barrel destinations. Module assignment, runtime/all-kind SCCs, test exclusions, weighted barrel-aware metrics, and supported glob/capture rules follow the Rust pipeline. Rules use their normal precedence, including kind filters, reachable forbids, public entries, strict entrypoints and layers. A layout written for a target previews too: `moves`, rule `source`, `layer-reason` and `early-smells` are accepted, a `<name>` capture may sit inside braces, and when the rules only forbid, an import no rule decides is not flagged unplanned, as on the map. With a target in the scan, the panel also shows each claim's verdict and broken imports and the slices that conform and have moved after the moves, with the scan's counts beside any that change; the move cost stays the scan's. A claim with a `pkg:`, `category:` or `global:` rule keeps its scan verdict, marked "package rules are not previewed", and the conforming count then stays the scan's, marked "(not previewed)". A residual SCC contained in a previous SCC is not called new cycle membership; a new or expanded component is flagged. This membership comparison is a preview explanation, not a CI verdict.

**Rust remains authoritative.** Grab-bag refinement, legacy boundary assumptions, unknown schema/rule fields and unsupported glob syntax visibly mark a limited/raw preview and withhold the no-new-cycle assurance. Dynamic/unresolved imports remain invisible. The preview does not rewrite import strings, check build compatibility, recompute symbol surfaces or execute code. The export covers every previewed move, shell-quoted: one `boundrail simulate graph.json --move A B --move C D` command, one script with `mkdir -p` for destination folders the scan does not contain and a `git mv --` per move, the layout `moves` for the list (exact paths, to paste above other moves; moves a rule cannot express become comments), and the layout rule changes: for each moved file, the `layers` globs and `forbid` `from`/`to` globs embedded in the scan that it stops and starts matching, read with the preview's glob matcher (`src/domain/rules.ts: leaves layer "src/domain/**", enters layer "src/adapters/**"`). A scan without embedded rules says so in one line. With several moves, the `simulate` JSON keeps `move` (the first) and adds `moves`, every move in order. Verify the command before running the script; set its graph JSON path and run from the scanned root. Moving files can still require import and build-configuration edits. Clipboard-denied and local-file environments fall back to selecting the command for manual copying.

The HTML remains self-contained and offline, including maps embedded in history timelines. No runtime package or schema migration is required. A fixture test rebuilds canonical Rust results and compares them with [the committed parity cases](../tests/fixtures/whatif-parity.json); Node tests run those cases against the exact JS embedded in reports. Regenerate intentionally with `UPDATE_WHATIF_FIXTURES=1 cargo test --test whatif`, then run `node --test tests/whatif-parity.test.cjs tests/whatif-ui.test.cjs`. Grab-bag fixtures verify the limitation warning rather than claim raw SCC parity with refined Rust cycles.

### Record the demo

The [capture script](../scripts/record-whatif-demo.cjs) uses a development-only Playwright/Chromium installation and `img2webp` (WebP tools); these are not boundrail dependencies. It drives a real browser mouse drag, clicks Copy, clears the preview, and asserts the browser has no errors and original state returns. It does not paint or fabricate the result.

```sh
python3 - <<'PY' > /tmp/whatif-demo.json
import json
cases = json.load(open('tests/fixtures/whatif-parity.json'))
print(json.dumps(next(c['document'] for c in cases if c['name'] == 'cut-module-cycle')))
PY
boundrail render /tmp/whatif-demo.json -o /tmp/whatif-demo.html
node scripts/record-whatif-demo.cjs /tmp/whatif-demo.html demo.webp a/three.ts b
```

Set `PLAYWRIGHT_MODULE`, `CHROMIUM_EXECUTABLE` or `IMG2WEBP` to existing tool paths when needed. The checked-in [animation](assets/whatif-demo.webp) records the `cut-module-cycle` Rust fixture: moving `a/three.ts` into `b` removes its two-module cycle. The script also writes a PNG poster. The asset demonstrates interaction, not a measured result on an OSS repository.

## Reading order

```sh
boundrail tour                                      # .boundrail/boundrail.json
boundrail tour graph.json > TOUR.md                  # dependencies before their importers
boundrail tour graph.json --reverse                 # entrypoints before dependencies
boundrail tour graph.json --agent                   # include saved export names and locations
boundrail tour graph.json --all-kinds                # include type, lazy and re-export imports
```

`tour` prints Markdown from a saved v1 or v1.1 scan without collecting imports, reading source bodies, or writing files. No schema migration or new dependencies are required. Runtime edges define the reading graph by default; a saved `config.all_kinds` or `--all-kinds` includes all recorded kinds. Dependencies use resolved semantic destinations, so an import through a resolved barrel points at its defining module. Included test files and external packages do not become stops or affect counts.

Each strongly connected component becomes one stop. Leaf components come first; a component's layer is its longest dependency distance to a leaf. Within a layer, stops sort by descending fan-in, then module name. `--reverse` reverses layers, retaining that fan-in order. This starts at dependency roots; it does not identify executable `main` functions or infer domain/application roles from folder names. Disconnected modules remain visible.

Every stop explains how many distinct outside modules import it and how many it imports. Repeated file imports count once. Cyclic stops say to read their modules together and start at the member with the fewest distinct outbound modules, including cycle peers; ties use module name. The stop's aggregate counts exclude its own members.

`--agent` attaches saved public surface export names and declaring files. Without a surface report it falls back to the saved symbol index; without either, it reports unavailable evidence and suggests rescanning with `--symbols`. Partial coverage is labeled. These summaries contain names, not full type signatures or bodies. `--granularity file` also works: export evidence follows its declaring file. The tour reflects only the saved graph; unresolved imports, exclusions and dynamic dependencies can hide relationships.

## Context pruning

```sh
boundrail scan . --symbols -o .boundrail/boundrail.json
boundrail context --task "Fix billing tax rounding" --files src/billing/tax.ts --budget 40k
boundrail context --graph graph.json --root . --task "Fix billing tax rounding" --files src/billing/tax.ts --symbols TaxRate --budget 20k --depth 3 --format markdown
```

`context` reads supported saved scans and prints a standalone JSON reading plan in the current schema (`kind: context`); `--format markdown` prints a pasteable plan. It does not modify files or run collectors. Use the source root corresponding to the saved graph. Current files supply source text, sizes and layouts; graph dependencies and export names reflect the saved scan, so rescan after structural changes. JSON contains ordered `entries` with `tier`, `path`, `reason`, `tokens`, `mode`, optional inline `content`, `depth` and `score`, plus omissions and limitations. Body entries name files for the agent to open; they do not inline their bodies.

Explicit files seed the search; task words match whole words in paths and source text, ignoring case and splitting camelCase and underscores. For example, `pay` matches `payInvoice` but not `payload`. `--symbols` matches saved exported names. Text search reads only production files listed in the saved graph, excluding dependency directories, tests, generated/support files and pure barrels. Missing, unreadable and outside-root paths are reported as omissions. All text matches compete for the budget. Provided files and explicit symbol matches anchor graph traversal. Without those anchors, at most 20 automatic path-keyword seeds expand the graph, with path ties resolved deterministically. Keyword matches do not create independent graph corroboration or alter an existing anchor distance. The selector follows imports and importers (every recorded kind) to `--depth` (default 3), using inverse depth, same-module, and available saved hotspot/co-change evidence. Task words found in fewer searchable files carry more weight than common vocabulary. Distinct direct links to supplied anchors add bounded graph evidence; module affinity and co-change use those anchors when available. Estimated body cost breaks relevance ties, so size alone cannot outweigh stronger evidence. Explicit files rank first. Eligible files in the immediate folder of an explicit non-barrel seed are also candidates, even without a recorded dependency. Text-only and folder-only candidates have null `depth` when no graph path was found. Files outside the saved graph are not discovered automatically. Recorded pure barrels are omitted: direct consumers can redirect a barrel seed to the definitions they actually use, without expanding every re-export. An `index.ts` known to the graph as executable code remains eligible.

Root and seed-ancestor `layout.yaml` files and the saved graph's embedded rules come first. Nested layouts are constraint evidence, not proof that the saved graph enforced them. Different on-disk and saved rules are both labeled. Constraints that exceed the budget cause an error with the required estimate. Then `must-read` covers seeds, text matches and distance 1; `should-read` covers same-module distance 2 and folder fallbacks; and `skim` uses export names for other distance-2 and deeper files. Candidates are packed in rank order; oversized bodies fall back to saved export names. When requested bodies exceed the budget and a related body can fit, each explicit seed gets a body allowance of the budget remaining after constraints divided by the number of explicit seeds plus one. Larger seeds use saved export summaries when available, leaving room for related bodies even when a seed could fit the overall budget. Seed bodies are restored afterward if unused budget permits. Without export evidence, a seed body is still attempted. These summaries contain names, **not full type signatures**; without symbol/surface evidence required summaries are omitted explicitly.

Budgets accept positive integers or decimal suffixes `k` (1000) and `m` (1000000). Estimated tokens use UTF-8 bytes / 4, rounded up, including task/framing, mandatory constraints, selected bodies, and inline export summaries. Greedy packing guarantees `used_tokens <= budget` for this estimate, not an exact model tokenizer limit or the serialized JSON envelope. Large required files may fit only as exports or be omitted. Missing dynamic dependencies and stale graph evidence still require judgment.

The Python 3 hook examples require a POSIX host. The project skill is [boundrail-context](../skills/boundrail-context/SKILL.md). Editor integration is opt-in: merge [Claude settings](../integrations/claude-context.settings.json) into project `.claude/settings.json`, or [Cursor hooks](../integrations/cursor-context.hooks.json) into `.cursor/hooks.json` and copy [the rule](../integrations/cursor-context.mdc) to `.cursor/rules/boundrail-context.mdc`. Keep `integrations/context-hook.py` in the project. Install the skill in the editor's project skill directory when desired. Nothing is installed automatically.

Claude `UserPromptSubmit` receives a compact, constraints-first context handoff and artifact paths; it must read the plan. Cursor `beforeSubmitPrompt` prepares artifacts but cannot inject context. Its asynchronous `sessionStart` guidance is best effort; the always-apply rule reruns the CLI when an artifact's workspace, task, generation or seeds cannot be verified. Hook subprocesses time out after eight seconds and fail softly; failure invalidates previous context for that conversation. `BOUNDRAIL_BINARY`, `BOUNDRAIL_CONTEXT_GRAPH`, `BOUNDRAIL_CONTEXT_BUDGET`, and optional `BOUNDRAIL_CONTEXT_FILES` (JSON array for Claude seeds) configure the adapter. Missing graphs do not block normal editing. See [the preregistered PR benchmark](../benchmarks/CONTEXT.md) for recall methodology and results.

## Package splits

`package-splits` ranks physical folder subtrees that could become packages, including folders spanning several graph modules. **Import-level ready** describes the observed dependency boundary; it does not mean extraction, compilation, packaging, or deployment has been verified.

```sh
boundrail package-splits graph.json --root .
boundrail package-splits graph.json --root . --format json
boundrail package-splits graph.json --root . --output packages.json
boundrail render packages.json -o packages.html
```

Each candidate needs at least two production files, a non-barrel implementation, an internal dependency, and at least one importing production module outside the folder. The score is **distinct importing modules × production files**. Weighted cohesion must be at least **0.8** by default: canonical internal dependencies divided by internal plus unextracted outgoing dependencies. Known dependencies on separate existing packages are externalized for this factor. Written barrel paths and import-ledger destinations still constrain the physical boundary; resolving a symbol into the folder cannot hide a dependency on an outside facade.

- **Free:** no recorded dependency to cut; outgoing dependencies may target proven separate packages. A subfolder's dependency on its own enclosing package is not a free external escape.
- **Cheap:** at most **3** distinct source/target file dependencies to cut by default. Every cut includes its kind and source line when available. These are dependency counts, not a prediction of how many edits are needed.
- **Needs verification:** missing import ledgers, unresolved/namespace imports, low-confidence edges, unscanned local bodies, ambiguous external aliases, unverified workspace metadata, or a package-boundary cycle prevent a readiness claim. A candidate can have a free observed boundary and still need verification.

All import kinds constrain the boundary, including lazy/type imports. Included tests inside the folder contribute boundary dependencies, while production files determine size, cohesion and fan-in. The cycle simulation removes the candidate from the remainder, compares SCC membership with the original graph, and also checks the collapsed package boundary after the listed cuts. Counts can increase when one SCC splits; that alone is not a new cycle. Simulation uses runtime imports by default, or all kinds with `--all-kinds`, and excludes test endpoints. Its SCC counts are for the physical dependency graph, before grab-bag reclassification.

```sh
boundrail package-splits graph.json --root . --max-cuts 0  # free boundaries only
boundrail package-splits graph.json --root . --min-cohesion 0.9 --limit 10
```

`--root` is the **directory originally scanned**, which may be a subfolder of the Git repository. Static inspection supports npm/Yarn workspace declarations, pnpm patterns and exclusions, and explicit Nx project metadata. A [configured Nx project](https://nx.dev/docs/reference/project-configuration) is tracked separately from a manifest-backed package; workspace layout conventions alone are not a package inventory. Existing package/project roots are not suggested again, and ancestors spanning those roots are skipped. A smaller folder inside an existing package can still be a candidate. Nx plugins and package-manager commands are never executed. Unsupported, skipped or incomplete metadata is reported.

New scans record `source.scan_root`, a Git-relative directory, to distinguish sibling roots at the same commit. A mismatched root/revision is rejected. Older graphs, non-Git scans, dirty checkouts, or omitted `--root` without saved metadata cannot establish verified workspace boundaries; results remain available with conservative labels. Keep generated graphs in an ignored output directory when using a clean checkout. Saved workspace evidence is reused when replaying an enriched graph, and candidate scores/cuts are recomputed from its current files and edges.

The map adds dashed island halos without replacing architecture health rings. Panels show free/cheap/unverified status, score factors, all cuts, simulation results and partial-module coverage. Nested suggestions can overlap; review them as alternatives rather than simultaneous extractions. Build configuration, package exports, consumer paths, excluded/shared test utilities, generated code, and environment assumptions remain outside this proof.

No scan-version migration is required: `package_splits` and `source.scan_root` are optional additions to v1.1. Standalone report JSON uses the new `package-splits` kind and cannot be exported with `--legacy-v1`; legacy scan input remains supported.

## Exclusions and included tests

```sh
boundrail show-excludes
boundrail scan . --include-tests --include-generated -o graph.json
```

The defaults cover test names/directories (`*_test.go`, `*.test.*`, `*.spec.*`, `test_*.py`, `*_test.py`, `test/`, `tests/`, `__tests__/`, `conftest.py`, `*_test.exs`, `test_helper.exs`), generated code (`*.generated.*`, `*_pb.*`, `*.pb.go` and language-specific generated files), build/dependency directories (`dist/`, `build/`, `node_modules/`, `vendor/`, `.venv/`, `_build/`, `deps/`), and fixtures, stories and examples. Use `show-excludes` for the complete list. Git ignore rules, including nested `.gitignore` files, and `.boundrailignore` also filter files; `.boundrailignore` uses gitignore syntax, including directory patterns, comments and negation.

Selection precedence is explicit: CLI `--exclude` and layout `exclude` win; layout `include` can override built-in defaults and ignore-file exclusions. `--include-tests` disables the default test category and `--include-generated` disables the generated category; neither cancels explicit excludes or ignore rules. Fixtures, stories and examples are a separate support category; restore them with layout `include` or `--no-default-excludes`, not `--include-generated`. `--no-default-excludes` disables all built-in defaults. Layout paths are relative to the scanned root:

```yaml
version: 1
exclude: ["src/legacy/**"]
include: ["tests/architecture/**", "examples/reference.py"]
```

Rust defaults also exclude `tests.rs`, `*_tests.rs`, `*_test.rs` and `test_support/`. Definite test-only out-of-line modules are classified as tests even with unconventional filenames. `--include-tests` retains separate test files as test modules; inline test references share a production file, so included inline references remain in the import ledger with a limitation and cannot be independently shown as test nodes or counted as per-file test coverage. Included test files carry `class: tests` and belong to separate test modules. Their imports remain available as evidence, but test nodes never participate in runtime or deferred cycle membership. `test_coverage` reports production module counts, how many have direct importing tests and `without_importing_tests`; this is import coverage, not execution or assertion coverage. Python inclusion discovers test files and support files under test directories outside the selected package, using the same AST pass without executing test code.

## Import resolution accounting

```sh
boundrail report .
boundrail unresolved .boundrail/boundrail.json
boundrail unresolved graph.json --json
boundrail scan . --language python --package app --python-path lib --python-path shared -o graph.json
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
boundrail scan . --rules layout.yaml -o graph.json
boundrail check graph.json
boundrail report . --rules layout.yaml
boundrail render graph.json -o graph.html
```

Match identities actually present in `external_nodes[].package`: package roots and subpaths vary by collector and ecosystem. For Go, use full import paths such as `gorm.io/gorm`; `gorm.io/gorm{,/**}` covers that root and its subpackages. `gorm` is not an inferred alias. Standard-library packages count when the collector identifies them as external; external does not mean downloaded or third-party licensed code.

Explicit forbids take precedence over allows, then the configured default applies. Folder selectors such as `to: "**"` match internal files only; use `pkg:**` for all external packages. Externals skip same-module allowances, layers and entrypoint checks. `kinds` filters apply as usual, including every hop of a `reachable: true` path from an internal file to a package. `check`, diff and simulation include package verdicts; known external nodes remain outside internal SCCs, coupling and cohesion. A saved graph without an import ledger cannot enforce an explicit package policy: rescan rather than treating missing evidence as a pass.

Host globals a file uses without declaring them (`process`, `Buffer`, `global`, `globalThis`, `window`, `self`, `document`, `navigator`, `location`, `localStorage`, `sessionStorage`, `indexedDB`, `fetch`, `XMLHttpRequest`, `WebSocket`, `console`, `setTimeout`, `setInterval`, `setImmediate`, `clearTimeout`, `clearInterval`, `queueMicrotask`, `crypto`, `__dirname`, `__filename`; recorded in the scan's `globals` field) take `global:<glob>` as the `to` selector of `forbid` or `allow` in JavaScript and TypeScript scans from the native engine:

```yaml
forbid:
  - { id: domain-no-env, from: "src/domain/**", to: "global:process" }
```

`check` prints `forbidden src/domain/a.ts -> global:process (domain-no-env)`, and SARIF points at the file's first use of `process`. A global becomes an external node (`global: true`, `package` its name) and a runtime edge only when a `global:` selector names it, so a layout without one keeps its verdicts. `pkg:**` never selects a global and `global:*` never selects a package; `from` and `moves[].from` reject `global:`, and it binds no capture. Naming a global makes its uses in other files `default` forbidden or, under the default, unplanned, as for packages: add an `allow` such as `{ from: "**", to: "global:process" }` for the files that may use it. A name declared anywhere in a file hides that global in the whole file, `globalThis.process` counts only `globalThis`, and `window["fetch"]` or shorthand `{ process }` count the outer name or nothing. Reference-engine, other-language and older scans record no globals, so a `global:` rule exits 2 there: rescan with `--engine native`. A forbidden global's recipe has a low-confidence `replace_global_use` step: replace the use with an injected dependency by hand, as `apply` never edits it.

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

For a cycle containing a candidate, boundrail splits that folder into per-file units temporarily and verifies which cycles survive. A `cycle-via-grabbag` finding is emitted only when this removes or shrinks the original module cycle. Its evidence lists original members, candidate factors, involved files/imports and any remaining cycles, with a suggestion to split the folder. Surviving cycles and their cuts remain ordinary cycle findings, using edges verified within the split components. If overlapping surviving components map back into the full original module cycle, boundrail conservatively retains the original cycle instead of claiming the folder split fixed it.

Test modules are never candidates, and test edges never enter these cycle probes. With `--include-tests`, visible weighted fan-in can still include test consumers, consistently with ordinary metric calculations; candidate selection alone is not proof of a cycle artifact. The verified split is the deciding evidence.

Choose graph grouping explicitly with the global option:

```sh
boundrail --granularity file scan . -o files.json
boundrail --granularity folder render files.json -o folders.html
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
boundrail bisect graph.json --root path/to/repo --finding 0 --format markdown
boundrail bisect graph.json --root path/to/repo --edge src/a.ts src/b.ts --from v1.0
boundrail bisect graph.json --root path/to/repo --finding 0 --github
```

`--finding` is the zero-based index in the saved graph; the first cycle finding is selected by default. `--edge` selects a forbidden import. Optional collector options such as `--depcruise`, `--python` and `--java` select the tooling. Probes preserve the graph's module boundaries, exclusion mode, selected Python package/Rust features and embedded rules, so a forbidden-edge introduction is judged under that fixed policy, not a claim that the rule existed historically.

The engine tries rename-following pickaxe candidates for supported import syntax, including imports added to existing files. It verifies graph states before attributing a change. Dynamic or aliased imports can fall back to graph bisect. Verified findings introduced within seven days are marked as recent. A verified result includes author name/email, author/commit dates, age, the introducing commit, and a closing import when that import itself appeared at the transition. Saved Rust graphs that enabled build scripts still require an explicit `--rust-build-scripts` for new probes. PR numbers are inferred from commit messages; `--github` optionally queries GitHub through `gh`. Author names are never converted into invented GitHub handles.

Searches use first-parent history, default to two years and at most 10,000 commits, and have a `--max-scans` budget of 64. `--from` supplies an inclusive lower revision. Presence at the lower boundary, missing collectors/dependencies, ambiguous rename identities and exhausted budgets remain explicit unknown/boundary results. Binary fallback verifies a local transition; nonmonotonic history means it does not prove the earliest-ever birth or uninterrupted presence. A later reintroduction can be a separate transition. Squash/merge authorship describes the Git commit, not necessarily the original contributor.

`report` adds bounded provenance to findings automatically when live Git history is available. All findings can be investigated by their kind/subject predicate; dates appear only after a verified transition. Reports share 64 historical collector calls and reuse probe results across findings. `--no-provenance` skips this cost. Recorded collector input cannot describe historical commits. The output's guard statement is conditional on the checked cycle/rule policy; it does not claim that boundrail CI was installed or ran on the historical PR.

## Hotspots

```sh
boundrail scan . --hotspots --hotspot-authors -o graph.json
boundrail report . --hotspots='12 months ago'
boundrail hotspots graph.json --root . --since '12 months ago' --authors --top 20
boundrail hotspots graph.json --root . --format json -o enriched.json
boundrail render enriched.json -o hotspots.html
```

The Git pass records per-file commits, added/deleted lines, distinct mailmapped author emails and fix-message commits, following detected renames. Only files in the scan participate in normalization; excluded generated/vendored files cannot dominate the score. The default window ends at the graph's commit, not today's date, so historical scans remain reproducible. Relative windows accept `<N> days/weeks/months/years ago`; twelve months is 365 days. `YYYY-MM-DD` dates are also accepted and start at 00:00 UTC.

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
boundrail scan . --symbols -o graph.json
boundrail surface graph.json
boundrail surface graph.json --max-symbols 50 --format json -o enriched.json
boundrail render enriched.json -o surface.html
```

`surface` prints a per-module report to standard output; `--format json` selects JSON. `-o` saves the enriched **scan document** for `render` and `check`, rather than redirecting the report. `boundrail report . --symbols` collects the same evidence and includes it in the report and map.

For each module, the report lists its entrypoints, exposed symbols and their definitions, symbols used from outside the module, and the importing files. Explicit `entrypoints` or existing `public` rules define the intended facade. Otherwise a top-level TypeScript/JavaScript `index` supplies its inferred facade. Python `__init__.py` is an inferred facade only when the saved symbol index proves named exports: an empty or docstring-only initializer does not make ordinary submodule imports violations. Explicit `entrypoints` and `public` policies still apply to empty entry files. A module without a facade exposes its direct file exports or Go package exports. Inferred surface findings are observations, not approved layout policy.

- **Could be internal:** an exposed symbol has no external consumer **inside this scan**. These suggestions appear only when symbol coverage is complete across the scan. This does not prove that a published library's API is unused by downstream projects, and boundrail never removes exports automatically.
- **Deep import:** an outside file bypasses a module's facade. The report lists the importer, target and available symbol names, and suggests importing through the entrypoint, re-exporting the required symbol or declaring a boundary rule.
- **Oversized surface:** external consumers touch more than `--max-symbols` distinct symbols (default 50). This suggests a facade around the operations those consumers need; it does not count every unused export toward the threshold.

`check` prints these advisories when the graph has surface evidence. They do not change its exit status. Declare a strict entrypoint to make every outside bypass forbidden, including imports of files the entrypoint does not re-export:

```yaml
version: 1
entrypoints:
  "src/billing/**": "src/billing/index.ts"
```

```sh
boundrail check graph.json --rules layout.yaml
```

The value names one exact entry file. Captures also work: `"src/domains/<name>/**": "src/domains/<name>/index.ts"` binds the same domain on both sides. A bypass is forbidden even when the entry file is missing. Explicit `forbid` rules take precedence; `entrypoints` rejects bypasses before `allow` rules. Existing `public` rules retain their earlier precedence, allowing explicit `allow` exceptions. Entry imports remain subject to layer rules. See [rule ordering](#rules-file).

Symbol collection uses the installed TypeScript compiler to resolve named/default imports, re-exports and `tsconfig` path aliases; Python's standard `ast` to read `from` imports, static `__all__` and star re-exports; and Go's `go/types` to resolve exported functions, types, methods and fields with qualified names. Go method analysis conservatively retains exports that implicit interface satisfaction might need. The symbol helpers read source without executing project code or fetching dependencies; the underlying file collector still has its [usual trust requirements](#boundrail-guide).

Dynamic Python export lists, unresolved stars, namespace ambiguity and missing dependencies can make coverage incomplete. Rust, Java and Elixir currently report an explicit symbol-collection gap. When `complete` is false, counts are provisional and **all could-be-internal suggestions are withheld**. Missing data can understate the surface, while namespace consumers and implicit Go interfaces conservatively credit extra symbols as used. File-level deep-import evidence remains available without symbols: `surface` accepts older scan documents and reports the missing coverage.

The map uses a bounded logarithmic ring width (1.25–6 pixels) for exposed symbol count while preserving architecture health colors. Selecting a module shows its surface metrics, importers and limitations.

## History

```sh
boundrail history . --append                         # monthly first commits, up to 24 points
boundrail history . --sample commits --every 50 --limit 30
boundrail history . --sample tags --limit 20
boundrail history . --since 2024-01-01 --hotspots --annotate
boundrail history . --diff v3.0 v4.0                 # release-note Markdown
```

The default selects the first first-parent commit of each month over two years ending at the selected `--rev` (default `HEAD`). `--sample commits` restores commit-by-commit sampling; `--every N` chooses intervals in that mode. Tags select release commits. `--limit` caps new selected samples. `--append` retains earlier cached points beyond that cap and scans only missing points. A root inside a monorepo scans that same subdirectory at every point. One language is selected from the current root (or `--language`) for the whole timeline; unrelated language tooling is not executed for cache metadata.

`.boundrail/history.json` contains full scans, adjacent diffs, release tags and summaries: files, modules, edges, module-cycle count, largest module cycle, percentage of modules in cycles, cycles per 100 modules, mean defined instability, forbidden edges (null until rules exist), and optional in-sample hotspot fix-message concentration. Counts use each snapshot's own boundaries; folder/module renames are not reconciled across time. The HTML shows both absolute cycle counts and normalized percentages; selecting a point opens its architecture map. Dashed markers identify tagged releases. `--annotate` verifies provenance between sampled points where cycles increase, then caches that context so append does not repeat those probes.

Full scan documents and sample indices are cached under `.boundrail/history/`. Cache identity includes scan root/options, current binary and dependency lock, explicit rules, selected external collector entry points and tool versions. Explicit `--rules` stays fixed; otherwise rules and ownership come from each commit. `--diff OLD NEW` scans those endpoints and reports new/resolved cycles and forbidden imports in Markdown. Its rule comparison uses each endpoint's own rules unless explicitly fixed.

Snapshots are collected in a disposable clone. The working tree, index, refs and registered worktrees remain unchanged. Failed snapshots are skipped, logged and listed in the report; an entirely failed selection exits 2. Shallow repositories expose only available commits. Ignored dependency installations and submodules are absent from snapshots; boundrail does not install/build historical projects automatically. TypeScript reuses an installed collector from the original checkout or `--depcruise`; Rust snapshots need cached locked dependencies. Missing tooling at old revisions is recorded as a failed point, not a fabricated empty graph.

[The monthly workflow](../.github/workflows/history.yml) restores cached snapshots, runs append on a schedule and uploads the report as an artifact. It is a repository workflow definition; local verification does not execute GitHub Actions.

## Impact

`boundrail impact BASE..HEAD` reports the blast radius of a committed diff: changed files, affected modules and files with shortest reverse-import depth and a decaying weight, selected tests, suggested reviewers, and cycle/hotspot risk. It leaves your checkout untouched and scans committed endpoints in a disposable local clone. It never installs dependencies or runs selected tests.

```sh
boundrail impact origin/main..HEAD
boundrail impact v1.0..v1.1 --root path/to/repo --max-depth 5 --format json
boundrail impact origin/main..HEAD --tests '**/checks/*.ts' --tests-output tests.txt -o impact.json
boundrail impact origin/main..HEAD --max-depth 5 --fail-on-depth 3
boundrail impact origin/main..HEAD --language go --go-packages-output packages.txt
```

Named/default TypeScript imports through pure re-export barrels are resolved to source declarations using the installed TypeScript compiler. Namespace imports, side-effect imports, unresolved symbols and changes to barrels retain conservative edges. The helper parses and typechecks source without executing it; if the compiler is unavailable, output records the fallback. Reverse imports consume one step; remaining pure re-export hops consume none, so an `index.ts` does not hide consumers behind an artificial depth limit. Depth defaults to 3; weight is `1 / (1 + depth)`, including changed files at depth 0. Module depth is the minimum across affected files. `--depth` retains its scan meaning (module folder depth); use `--max-depth` for traversal. Normal analysis succeeds even with risk; `--fail-on-depth N` exits 1 when a module's shortest depth exceeds N. Set `--max-depth` greater than N. Invalid inputs and collector failures exit 2.

Tests include `*_test.go`, `*.spec.*`, `*.test.*`, Python `test_*.py`, files in `test/`, `tests/` or `__tests__/`, and additional repeatable `--tests` globs. Live scans include tests automatically; TypeScript `--include-tests` collects the whole scanned root, including test trees outside `src/`. Python selection covers the chosen Grimp package; external test trees absent from that graph have unknown coverage. For saved scans, use `scan --include-tests`; a graph without test edges cannot establish coverage. `tests.txt` contains one existing head test path per line (newline-containing paths require JSON). An empty selection is not evidence that tests are unnecessary.

Go uses [`go list -deps -test -json`](https://pkg.go.dev/cmd/go#hdr-List_packages_or_modules) per module, including dependencies used only by external tests. This selection is independent of the graph depth cap. `--go-packages-output` exports package directories; run Go tests by package, from the appropriate directory containing `go.mod` in repositories with multiple modules. Selection reflects the current platform with cgo disabled, and requires cached dependencies. Failure retains graph-based tests and records the coverage gap. No project test code is executed.

Reviewers prefer CODEOWNERS for each affected module; modules without declared owners fall back to the five most active contributors over the previous 365 days, anchored to the head commit date. Git commits count once per author/module even when several files changed; names and emails follow Git mailmaps. Declared ownership is not inferred authorship. Hotspot risk uses positive-score files in the top 10% of the scanned root's hotspot ranking; cycle risk uses module cycles. Missing or shallow history is reported as a data gap.

Reuse existing clean endpoint scans to avoid collectors:

```sh
boundrail impact BASE..HEAD --graph head.json --base-graph base.json
```

The recorded commit must match the endpoint, the graph must be clean, and `--root` must match the scanned directory within the repository. Base traversal preserves deleted files and removed imports; returned test paths are filtered to files still present at head. Live scans attempt both endpoints; unavailable base collectors produce a limitation rather than dropping the head result. A saved head graph alone cannot recover missing base dependencies. Counts follow each snapshot's module boundaries, so renamed folders can appear separately.

Results mean **at least these**, not an exhaustive list. Dynamic imports missing from collectors, dependency injection, reflection, build conditions and runtime wiring can add impact. Barrel edges without enough symbol information retain conservative consumers. Vendored/generated paths and explicit scan exclusions stay excluded. This is dependency evidence, not a guarantee that running selected tests alone is sufficient.

For GitHub Actions, use the PR's actual head and base commits (checkout's default merge commit is different). Fetch both endpoints and install boundrail and its collector as usual. Have a read-only analysis job produce clean `head.json` and `base.json` scans with tests included, then pass those artifacts to a posting job:

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
    run: boundrail impact "$IMPACT_BASE..$IMPACT_HEAD" --graph head.json --base-graph base.json --github-actions --tests-output tests.txt -o impact.json
```

`--github-actions` writes `GITHUB_STEP_SUMMARY` and creates a new PR comment via [`gh pr comment --body-file`](https://cli.github.com/manual/gh_pr_comment); it takes the PR number from `GITHUB_EVENT_PATH`, or `--pr N`. Posting failures (including read-only fork tokens) become limitations and leave analysis successful. JSON and exported files remain available for artifact upload or later steps. Lists in comments are capped at 100 items; machine-readable output is complete. Each invocation creates a new comment. Keep write tokens out of untrusted collector/build execution; use separate analysis and posting jobs for fork PRs.

## CI

The action at the repository root runs `report` on every push, and on pull requests compares HEAD with the pull request's base commit. Before publishing a canonical Boundrail release, build from source and supply `binary`. The examples use current main. Pin both the source revision and action revision for a reproducible setup.

```yaml
name: architecture
on: [push, pull_request]
permissions:
  contents: read
jobs:
  boundrail:
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
      - run: cargo +stable install --git https://github.com/dogebonker/boundrail --locked --bin boundrail
      - uses: dogebonker/boundrail@main
        with: { binary: boundrail }
```

To show forbidden imports and broken claims as code scanning alerts, with annotations on the changed lines of a pull request, upload the `sarif` output:

```yaml
name: architecture
on:
  push:
    branches: [main]
  pull_request:
permissions:
  contents: read
  security-events: write
  actions: read # private repositories only
jobs:
  boundrail:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0
        with:
          node-version: 22
      - run: cargo +stable install --git https://github.com/dogebonker/boundrail --locked --bin boundrail
      - id: boundrail
        uses: dogebonker/boundrail@main
        with: { binary: boundrail }
      - uses: github/codeql-action/upload-sarif@9b3254085494b55a0f7c4c2b3d2d52cc712ce370 # v4.39.1
        if: ${{ !cancelled() && steps.boundrail.outputs.sarif }}
        with:
          sarif_file: ${{ steps.boundrail.outputs.sarif }}
```

The upload step runs after a failed boundrail step, since the file is written before the gate result. The SHA is the peeled `^{}` line of `git ls-remote https://github.com/github/codeql-action 'refs/tags/v4*'` for the `v4.39.1` tag; update it from that repository's releases.

- Private repositories need GitHub Code Security enabled; public repositories do not.
- Fork and Dependabot pull requests can upload under `pull_request`. Never use `pull_request_target`, as below.
- When `actions/checkout` uses `path:`, set `checkout_path` on the upload step to that path.
- `upload-sarif` warns about the provided fingerprint only for results that have a line: it skips results without `region.startLine` and warns only when its computed hash differs. That is expected.
- Give each `forbid` rule an `id`. Unnamed rules are numbered `forbid[i]`, so inserting one renumbers the rest and reopens their alerts.

Never run it on `pull_request_target`. That event gets the base repository's write token and secrets, and scanning the pull request's code there hands a privileged job to files the pull request controls: its `tsconfig.json`, `layout.yaml` and file names (see the warning at the top). `pull_request` runs with a read-only token and no secrets for forks.

- **Push:** `report` with `layout.yaml` from `path` (or `rules`), its summary in the job summary. The step fails as `report` exits: on a forbidden import the known list does not hold, or a listed one that no longer occurs.
  The summary includes the `Target` block of `report` (claim verdicts, slices conforming and moved, move cost). With no base ref the step exits with `report`'s status: 1 when an import is forbidden and not in `layout.known.yaml`, or a listed one no longer occurs. A repository whose baseline forbidden imports are all listed passes pushes.
- **Pull request** (or with `base-ref`): `report` on HEAD, then a scan of the base commit, checked out with `git worktree` into `$RUNNER_TEMP/base`, with HEAD's rules file and the same options, then `diff --fail-on <fail-on> --format markdown` into the job summary. The summary's `Target` section shows slices conforming before and after and the slices a claim newly breaks; the gate stays `fail-on`. The step fails only on what the pull request adds: by default new forbidden imports, new cycles, stale known entries and newly vacuous claims (`fail-on: new-forbidden,new-cycles,stale-known,new-vacuous,new-unplanned` adds unplanned imports), so a pull request that fixes a listed import must also drop its entry (`boundrail check --update-known shrink`), and one that moves every file a claim covers must update the claim in the same change. The diff never reads `layout.known.yaml`: a moved file with known debt still counts as a new forbidden import. An older pinned `version` rejects `stale-known` or `new-vacuous` as a usage error (exit 2); set `fail-on: new-forbidden,new-cycles` there. `fetch-depth: 0` keeps the base commit in the checkout; with the default shallow checkout the action fetches that one commit from `origin`, which needs the repository to be public or the credentials kept.
- **Co-change.** On a full checkout (`git rev-parse --is-shallow-repository` prints `false`, as with `fetch-depth: 0`) the push `report` and both pull request scans read the last 300 commits (`--cochange`), so the push summary lists each slice's [co-changed pairs](#claims) and the pull request summary those new in HEAD, advisory and never failing the step; it also adds `hidden-coupling` findings and `git log` time. On a shallow checkout the summary ends `Co-change not read: shallow checkout; check out with fetch-depth: 0.`; outside git nothing is said.
- **Rules changes.** Both commits are judged by HEAD's rules, so a pull request that deletes a `forbid` rule and adds the import it forbade passes the diff. When the rules file differs from the base commit's (changed, added or removed), the job summary opens with a warning and a table of forbidden imports in base and HEAD under each commit's rules, and the output `rules-changed` is `true`. That is a signal, not a failure: `fail-on-rules-change: true` fails the step on it, for teams that review rule changes separately.
- It installs the boundrail release whose version is in the action's own `Cargo.toml`, unless `binary` supplies a source-built executable (`version` overrides the release). Use only tags whose assets follow the canonical Boundrail layout. The collector goes into `$RUNNER_TEMP`, locked: dependency-cruiser 18.4.0 and typescript 6.0.3 with `npm ci` from [ci/tools/package-lock.json](ci/tools/package-lock.json) when `path` has `package.json` or `tsconfig.json`; grimp 3.17 in a venv with `pip install --only-binary :all: --require-hashes` from [ci/tools/requirements.txt](ci/tools/requirements.txt) when it has `pyproject.toml`, `setup.py` or `setup.cfg` (else set `language`), which needs CPython 3.10 to 3.14 as `python3` (`actions/setup-python`); Go 1.27.1 from go.dev, checked against its SHA-256 in [ci/action.sh](ci/action.sh), when it has `go.mod` or `go.work` (no `actions/setup-go` needed; `BOUNDRAIL_GO` names one to use instead). Linux and macOS runners.
- Paths and messages from the scanned repository reach the log with workflow commands turned off (`::stop-commands::`), and the push summary's code fence is longer than any backtick run in it.
- `boundrail.html` and `boundrail.json` are uploaded as the artifact `boundrail-<job id>`, also when the step fails. In a matrix job, set `artifact-name` per entry (`artifact-name: boundrail-${{ matrix.os }}`), since artifact names must be unique in a run.

- Elixir action scans use a preinstalled Elixir 1.14+ / OTP 25+ runtime. Install it before the action (for example, `sudo apt-get update && sudo apt-get install --yes elixir` on Ubuntu), set `language: elixir` when detection is ambiguous, and optionally set `BOUNDRAIL_ELIXIR` to its executable. The action does not install Elixir or run Mix.

Inputs: `path` (default `.`), `rules`, `depth` (default 2; for Java, each package is a module), `language`, `engine` (`reference`, experimental `native`, or TypeScript-only `depcruise`), `package`, `exclude` (globs, one per line or comma-separated), `fail-on`, `fail-on-rules-change` (default `false`), `version`, `binary` (a Boundrail executable to use instead of a release), `base-ref`, `artifact-name`. With `engine: native`, the action forwards the engine to HEAD and base scans and skips all external collector setup, including npm, Python, Go and Elixir. Native Go needs a preinstalled Go toolchain; `BOUNDRAIL_GO` selects its executable without a download. Outputs: `json`, `html` and `sarif` (paths; `sarif` is empty without rules), `forbidden` (count at HEAD), `new-forbidden`, `new-cycles` (counts) and `rules-changed`, the last three for pull requests only. All logic is in [ci/action.sh](ci/action.sh), which reads its inputs from `INPUT_*` environment variables, so it runs outside Actions too.

## Schema contract

New import-analysis documents use `schema: "boundrail/v1.2"`. The current contract is [schema/boundrail.v1.2.schema.json](../schema/boundrail.v1.2.schema.json) (JSON Schema draft 2020-12); [v1.1](../schema/boundrail.v1.1.schema.json) and [v1](../schema/boundrail.v1.schema.json) contracts remain available. v1.2 adds typed recipes, stable `finding_id`s and advisory `diagnostics`; it does not change the edge-kind or weighted-metric definitions. Upgraded readers accept all three versions and recompute current findings from saved graph evidence. Tests validate generated documents against their schema.

- Consumers check `schema` and reject unknown major versions. They ignore unknown fields and unknown finding kinds (`kind` is an open string in the schema; known kinds have typed evidence).
- `source.language` and `source.collector.name` are closed enums that may grow within boundrail/v1 as languages are added (`go` and `go-types` were). An older boundrail, or a consumer validating against an older copy of the schema, rejects a document with a language or collector it does not know: read a language’s scans with a Boundrail that has its collector, including `elixir` / `elixir-ast`.
- A scan has a top-level `summary`: forbidden, unplanned and cycle counts and each module's cohesion, Ca, Ce and instability. A finding's `simulation` is `{ "after": … }`: the same counts after the move, with `modules` listing only the modules whose metrics changed (`null` for a module the move empties). Compare it with the scan's `summary`.
- A scan document whose files, modules and edges disagree (an edge naming a file not in `files`, a file listed twice, a module its config does not give) is an input error, exit 2.
- Scans may carry optional `external_exposure` summaries and separate `external_nodes`/`external_edges` package data. These are additive v1.1 fields; the v1 package-data contract remains unchanged (its language/collector enums may grow), and compatibility exports retain additive package data. Missing occurrence accounting means unavailable exposure.
- Scans may carry optional `symbols` (exports, imports, resolution targets, completeness and limitations) and `surface` (per-module analysis and threshold) blocks. Standalone surface reports use `kind: "surface"`. Older scans remain readable; absent symbol data never establishes that an export is unused.
- v1.1 adds weighted metrics and deferred-cycle accounting and uses canonical edge-kind names. Upgraded readers accept v1, v1.1 and v1.2; frozen v1 readers need the compatibility export described below. Optional fields and unknown finding kinds should be ignored by consumers that do not use them.
- Historical v1 change before the first release, 0.1.0 (the kind names in this paragraph are the old names): `cycles`, and with them `summary.cycles`, each cycle's `break` and `two_way` and a diff's `new_cycles` and `resolved_cycles`, count runtime imports only (`static`, `reexport`). Edge `kind` gained `type-reexport` for `export type … from`, formerly `reexport`. Builds before this change also counted `type`, `dynamic` and type-only re-exports and list more and larger cycles; rescan both sides before diffing across it. Python edges gained `type` and `dynamic` (until then every grimp edge was `static`), and scans have `package_entries` and two-way `registry`/`entry` flags. In the same change an `internals-reach` finding became one per bypassed module (subject: the module id) with `imports`, `importers`, `importing_files` and up to 20 `edges`, replacing one finding per import with `from` and `to`; with rules, one per bound `public` module (subject: its pattern).
- Graph-node paths stay relative to the scan root. Raw import specifiers and explicitly configured external Python search roots may contain absolute paths. Arrays have deterministic ordering. `generated_at` uses `SOURCE_DATE_EPOCH`, Git HEAD time or the Unix epoch; identical trees, Git state, collectors, options and dependency environments produce identical scan bytes.
- Instability and cohesion ratios are rounded to 4 decimals and are `null` when their denominator is zero. In v1.1 they use the weighted totals described below; raw integer counts remain available.

### Edge quality and migration

`check`, `simulate`, `render` and `diff` read `boundrail/v1`, `boundrail/v1.1` and `boundrail/v1.2` and recompute analysis with the current engine. Legacy edge names `static` and `dynamic` are accepted as `runtime` and `lazy`; `type-reexport` becomes `type` with `is_reexport: true`. New exports use the four canonical kinds. TypeScript/JavaScript classification combines dependency-cruiser metadata with a lightweight import tokenizer: function-local `require` and dynamic `import()` are lazy, while dependency-cruiser's type-only usage information takes precedence. This describes the reference engine; the opt-in native engine uses tree-sitter grammar queries.

Default cycles contain only `runtime` edges. Pure structural re-exports do not close a runtime cycle. `type_only_cycles` retains other cyclic components, with their `kinds` and a `contains_runtime_cycle` flag when a larger component contains a smaller runtime cycle. Despite the field name, this accounting includes lazy, re-export and mixed cycles, not just type-only cycles. Summary/report counts identify this broader category. `--all-kinds` runs cycle analysis over every kind for comparison. It is broader than the previous engine at commit `6627a5e`, whose cycle analysis already excluded types and dynamic imports but included value re-exports.

Each edge carries a `weight`. Defaults are runtime **1.0**, lazy **0.3**, type **0.1**, reexport **0.0**; customize them with `edge_weights` in `layout.yaml`. Values must be finite and non-negative. Metric attribution through barrels inherits the consumer's weight. Multiple edges between the same file pair contribute their strongest weight. Weighted Ca/Ce sum the strongest weight for each neighbouring module, rather than adding every file import; weighted internal/outgoing totals sum weighted file pairs. Instability is `weighted_ce / (weighted_ca + weighted_ce)`; cohesion is `weighted_internal_edges / (weighted_internal_edges + weighted_outgoing_edges)`. The integer `ca`, `ce`, `internal_edges`, `outgoing_edges` and `outgoing_to` counts are retained alongside `weighted_*` fields.

For frozen consumers requiring v1 field names and discriminator, add the global JSON export flag:

```sh
boundrail --legacy-v1 scan . -o graph-v1.json
boundrail --all-kinds scan . -o graph-all-kinds.json
```

`--legacy-v1` exports the compatibility shape, including old edge-kind names; it does not restore the old analysis or its unweighted metric ratios. A file-granularity scan exported with this flag is regrouped into the legacy folder view so old readers can validate it; the exported module and cycle numbers describe that folder view. Keep the ordinary v1.2 export for file-view numbers and recipes. Compatibility exports omit recipes/IDs, merge advisory diagnostics back into legacy findings and omit the new explicit forbidden-edge/deep-import finding kinds; verdict and surface evidence remain available. Rescan both sides with the same options before comparing architecture changes across engines. A mixed old/new comparison can reflect methodology changes rather than repository changes. Frozen readers may ignore newer policy fields such as rule `kinds`; use an upgraded, pinned CLI when enforcing those policies in CI.

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

Globs: `*` is one path segment, `**` any depth, `{a,b}` alternation. `<name>` captures one segment and binds it within a rule: `modules` path to id, `public` module to entries, `entrypoints` module to entry file, and `forbid`/`allow` importer to target, so `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }` matches only imports within one module. `to`, `id`, `entries` and entrypoint values may use only their rule's capture, and `layers` take no capture (write `*`). A file matching several layers takes the first. Unknown keys in `layout.yaml` are an error, and so is a control character (a line break, a tab) in any of its strings, since ids, reasons and globs are printed into logs and job summaries; rules embedded in a scan document may carry keys a newer boundrail added.

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

`default: forbidden` turns the rules into a whitelist. A Boundrail older than this key reads a scan document whose embedded rules carry it and ignores the key (unknown keys in embedded rules are tolerated), so its `check` reports those imports as unplanned and exits 0. Pin the boundrail version wherever a whitelist gates CI.

### Claims

Forbid rules sharing an `id`, at least one with a `reason`, form a claim, such as a target architecture's "Dependencies point inward". A claim is judged per slice, one value of its `<name>` capture (rules without a capture form the slice `*`), against every import, whichever rule first decides that import's verdict. A slice counts only when one of the claim's rules has files on both sides in it, so a slice that cannot break is left out. The verdict is `holds`, `holds_in` (some slices break), `broken` or `vacuous` (no slice counts, so the claim protects nothing). A rule side that matches no file adds a note. For a `reachable` rule, a break lists the import that starts the path, not the whole path. `check` prints a line per claim with up to 20 breaking imports, then `target: N of M slices conform`: the captured slices that hold in every claim counting them. The scan document carries the same as `target`; `check --brief` and MCP `target` carry it bounded, trimming slices and moves first, so read the full lists from `check -o <file>`. With co-change read (`report --cochange`, or a scan saved with `--cochange`), a slice also lists `cochanged`: pairs changed together in 3 or more commits (commits touching more than 30 files skipped) with one file on a claim rule's `from` side and the other on its `to` side, `from` side first, and no chain of imports either way among the slice's claim-side files (a handler and a repository joined through the service are the layering at work; a chain through a file on no claim side, such as a shared types file, does not count); `check` and `report` print up to 20 per claim as `  co-change: a <-> b, 3 commits, no import`. `diff` lists the pairs new in the new scan under `target.newly_cochanged`, only when both scans read co-change, with `, 1 newly co-changed pair` on the markdown Target line and a `### Changed together across a claim, no import` list, which `review` also shows (JSON `newly_cochanged`). They are advisory: verdicts, progress, SARIF, the page and exit codes ignore them. Exit codes stay those of the forbidden imports; `diff --fail-on new-vacuous` (on by default in the [Action](#ci) for pull requests) exits 1 when a change makes a non-vacuous claim vacuous.

```yaml
forbid:
  - { id: layered, reason: "Dependencies point inward (slide 8)",
      from: "{internal/<name>/handler*.go,internal/app/<name>/handler/*.go}",
      to: "{internal/<name>/repository*.go,internal/app/<name>/repository/*.go}" }
```

```text
claim layered: holds in 16 of 18 slices; broken in interview, sop reason="Dependencies point inward (slide 8)"
```

### Known violations

A layout written for code that already breaks it fails `check` on every existing import. List them in `layout.known.yaml` beside the rules file (`<stem>.known.yaml`: `ci/strict.yaml` reads `ci/strict.known.yaml`) and `check` fails only on a forbidden import missing from the list, or on a listed one that no longer occurs. The list only shrinks: fixing a listed import fails `check` until the entry goes.

```sh
boundrail check --update-known accept    # list every forbidden import now (creates the file)
boundrail check --update-known shrink    # drop entries that no longer occur, add none
```

Both print `layout.known.yaml: N kept, N dropped, N added` on stderr and then check as usual; `shrink` without the file exits 2, and a saved scan needs `--rules` to name where the file goes. Run them by hand and commit the file; CI only reads it.

```yaml
# Forbidden imports layout.yaml already has. check fails on any other, and on an entry that no
# longer occurs. Shrink with: boundrail check --update-known shrink
version: 1
known:
  - { rule: "forbid[0]", from: "src/adapters/in/http.ts", to: "src/adapters/out/pg.ts" }
```

An entry is the importer, the written target as `check` prints it (`pkg:<package>` for a package) and the rule that decides the import, without a line, sorted one per line. Unknown keys and control characters are an error, as in `layout.yaml`. Moving a file or renaming a rule makes its entries stale and the imports unlisted: run `accept` again. Unnamed rules are numbered (`forbid[0]`), so adding one renumbers the rest: give rules an `id`. An intended exception belongs in `allow`, not here.

`check` leaves listed imports out of its lines and counts them in the totals (`3 forbidden (2 known), 0 unplanned`), and prints `stale known <from> -> <to> (<rule>): no longer forbidden; run boundrail check --update-known shrink` for an entry that no longer occurs. Claims still count a listed import as broken. `report` exits the same way and prints `N forbidden (M known)`. The scan document embeds the list as `config.rules.known`, marks listed edges `known: true` and counts them in `summary.known`, and lists stale entries in `stale_known`; `--rules` replaces the rules and the list together.

### Moves

`moves` maps today's paths to the target's, first match wins, for move cost and progress; verdicts never read it. `from` is a file glob that may capture `<name>`; `to` is a folder ending in `/`, without globs, that may use only that capture. A file goes to the bound `to` plus its path below the literal leading folders of `from`, so `internal/<name>/**` keeps subfolders. A slice has moved when no file still matches a `from`; the total counts every slice with a file matching a `from` or already below a `to`.

```yaml
moves:
  - { from: "internal/<name>/handler*.go",    to: "internal/app/<name>/handler/" }
  - { from: "internal/<name>/service*.go",    to: "internal/app/<name>/service/" }
  - { from: "internal/<name>/repository*.go", to: "internal/app/<name>/repository/" }
```

```text
move project: 3 files, 9 import lines
target: 16 of 18 slices conform, 0 of 23 slices moved
```

`check` prints a `move` line per slice with files left to move, with the import lines to rewrite: for Go, new (importer, imported folder) pairs across folders once that slice's files move; for other languages, the import records (or edges, when the collector keeps none) whose importer or target moves. The scan document's `target.moves` lists every slice with its files and destinations, and `target.notes` names a move rule that matches no file.

On the map, selecting a slice under **Move cost** in the Target panel draws an arrow from each of its files to its destination, and a folder the scan does not have yet as a faint empty disc beside the module the files leave, labelled `new folder <dir>/`; existing discs do not move.

For Go, each slice also lists the unexported names its moves force to export as `names`, `{name, declared_in, used_by}` from `source.collector.bound` (`, 2 names to export` on the `move` line), and as `together` the pairs that hold a method and its type or two platform variants, which split across packages would not compile. A pair whose two files land in one folder after the move is left out, so moving `service.go` with its partner lists nothing. `names` is `[]` when none is needed and `null` when unknown: the native Go engine (with a note in `target.notes`) and other languages.

For Go, boundrail also applies every pending move together and lists the imports that would sit in new package cycles as `new_cycles` (`, 2 imports in new package cycles` on the `move` line): one cycle between two packages shows at least two imports. A cycle belongs to each slice with a source or destination folder on either side of it, so a cycle that needs two slices to move is listed under both. When the moves cannot be applied, such as a destination that already exists, `target.notes` says why and the other move fields still report. Other languages report no `new_cycles`.

Write each claim side over both locations, as in the claim above, so it keeps holding while a slice is half moved: a moved handler importing an unmoved repository still breaks it. Keep the target root free of files: with `{internal/<name>/handler*.go,...}`, a file `internal/app/handler.go` binds a slice `app`. A Boundrail older than `moves` rejects a `layout.yaml` that has it, so pin the boundrail version in CI before adding it.

## Findings

| kind | when | suggestion |
| --- | --- | --- |
| `cycle` | a module-level import cycle, or a file cycle among the imports inside one module, over `runtime` imports by default: type-only, lazy and structural re-export edges are accounted for separately in `type_only_cycles` (all kinds remain in weighted metrics and applicable rule verdicts) | the fewest imports to remove (`break`); with `--all-kinds`, a public entry's re-export is removed only when nothing else breaks the cycle. Public entries for this are package entries and the source root's index: `index.ts`, or the Python package's `__init__.py`, whose imports of its own submodules are re-exports. An internal barrel's re-export of another module's file is the first thing to cut ("drop the re-export"). A module cycle is `grouping` when no file cycle among its members' files (same import kinds) holds files of two or more members: it comes only from files sharing a module. When one registrar file, imported by at least one file (a file nobody imports is an entry point build config names, considered only when it is in a `grouping` cycle's break set, and then flagged `entrypoint`) and not an index (`index.ts`, `__init__.py`: its module's entry), alone forms the other side of 3 or more two-way pairs, the cycle lists it in `registrar_moves` (`file`, `module`, `pairs`, `with`) with a simulated destination `to`: a new folder named after the file in kebab case (`driver/DriverFactory.ts` to `driver/driver-factory/DriverFactory.ts`), in the file's folder when that is a module of its own at the scan's depth, else beside its module's folder, or the folder of its one importer when it has exactly one. The destination removing the most two-way pairs (`pairs_removed`, net over all module cycles) without a new cycle wins, with the cycle's size before and after (`cycle_before`, `cycle_after`); a registrar with no such destination is not listed. The move is the first step only when it removes more than one pair while a single-import cut exists (more than none otherwise) and the cycle shrinks (`cycle_after` below `cycle_before`: removing pairs while the cycle keeps or grows its size only reroutes them); else it follows the cut, with its numbers. When a cycle has no pair to cut, the break import shown first is one into a package entry or the root index if the break has one. Module cycles also list `two_way`: each pair of members importing each other in the direction to cut, with its import counts both ways and up to 20 of its file imports: the lighter direction, unless it holds only wiring or a public re-export and the other does not. A pair whose sides both have at most 2 imports, within a factor of 2, is flagged `even` (a mutual dependency such as a foreign key both ways, listed in the names' order with no lighter side); a pair with no ordinary side is flagged `registry` (only wiring) or `entry` (public re-exports both ways). A pair whose other side is only a registrar listed in `registrar_moves` is flagged `resolved_by_move`: the move resolves it. Flagged pairs sort last and none is offered as a cut. Break sets and lighter directions are graph measurements, not unconditional repair advice; actionable cycle recipes require concrete cyclic import evidence |
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
- Go: references come from type-checking each package without its dependencies outside the repository, so a use reached only through such a dependency's types (a method of a value it returns, an argument of a call to a function it provides) is missing, and an import left with no resolved use counts as unresolved; every platform's files are merged; reflection, `go:linkname`, `go:embed`, assembly and C are invisible. A `go` line in `go.mod` newer than the Go running the scan fails it: boundrail never downloads a toolchain.
- Small repositories have little signal; the `small-repo` diagnostic says so.
- Actionable findings contain recipe steps and verification predicates; advisory diagnostics preserve evidence without a concrete edit. Ranking prioritizes review and is not a target score to optimize. Module colors in `render` are a reading aid.
- Folder boundaries come from `--depth` or rules `modules:`; `--granularity file` offers a file view. Wrong boundaries give wrong metrics; grab-bag findings require a verified split rather than a name heuristic.
- Named imports through proven barrels resolve to their defining files. Legacy recordings and imports lacking usable export evidence can retain conservative barrel attribution; inspect low-confidence edges before treating them as runtime dependency evidence.
