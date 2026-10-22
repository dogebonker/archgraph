# Globals: `global:` rule targets for host globals

Branch `feat/globals-wiring` from main 78597da. Approved 2026-10-22: `global:<glob>` as a `to`
selector in `forbid`/`allow` for JS/TS on the native engine, so a file that uses a host global
without declaring it gets a verdict. None of the 24 trial runs in
`bench/results/usefulness-2026-10-20-fixed-harness` added a global or an event, so `global:` is
built on design grounds, not trial evidence. The injection collector moves to a follow-up plan that
infers the application model (services, routes, DB clients, queues, outbound HTTP) from code, to
replace the hand-written services manifest.

## Decisions

- D1. Uses persist in their own optional scan field `globals: [{from, name, line}]`, one per file
  and global at its first use, carried in `Config.globals` (`#[serde(skip)]`) like
  `import_records` (`src/model.rs:101,246-247`, `src/lib.rs:128,489`). Not `ImportRecord`s: import
  accounting (`src/resolution.rs:5-22`), exposure (`src/exposure.rs:51-113`), package splits and
  recipe cut steps (`src/recipes.rs:341-353`) count every record as an import. `None` means not
  collected (reference engine, other languages, older scans).
- D2. A global becomes an external node and edge only when a `global:` selector of `forbid` or
  `allow` matches its name. Every unmatched external is `default` forbidden or `none` unplanned
  (`src/rules.rs:1112-1117`), so adding every `console` use would change existing results. Layouts
  without `global:` keep identical verdicts, external arrays and `check` output. The scan JSON
  only gains `globals`. Edge: kind `runtime`, `specifier` the name, `line` the first use. Node: id
  `external:<lang>:global:<name>` through `external_id` (`src/resolution.rs:132-137`), `package`
  the name (the schema requires it non-empty, `schema/archgraph.v1.2.schema.json:1958-1975`), and
  a new `global: true` on `ExternalNode` (`src/model.rs:364-369`), serialized only when true.
  Every consumer chaining `external_edges` handles it unchanged, with two exceptions:
  `recipes::external_matches` (`src/recipes.rs:337-341`, used at `:350` and `:707`) requires
  `!global`, so an `import process from 'process'` record never matches a global node, and
  `report`'s known external packages count (`src/cli/report.rs:138-142`) skips global nodes.
- D3. The id-to-name map handed to rules carries `pkg:<package>` or `global:<name>`, built by one
  helper `resolution::policy_names` replacing `src/lib.rs:386-391`, `src/target.rs:181-183` and
  `known::target` (`src/known.rs:97-103`). `matches_target` (`src/rules.rs:1013-1039`) requires
  the selector's prefix on the name, so `pkg:**` never matches a global and `global:*` never a
  package, and its file branch (`:1030-1034`) also rejects `global:`. A prefix inside `package`
  was rejected: package names already contain colons (`pkg:node:fs`, `tests/rules.rs:496`).
- D4. `global:<glob>` is valid only as `to` of `forbid`/`allow`, non-empty, without captures (as
  `category:`, `src/rules.rs:624-630`). `from` and `moves[].from` reject it (`:619`, `:650`), and
  `relevant_instances` skips it (`:880`). `require_policy_accounting` (`src/resolution.rs:113-130`)
  fails when a rule names `global:` and `globals` is `None`, with `global rules require a native
  JavaScript/TypeScript scan, rescan with --engine native`. `validate_external_identities`
  (`:140-158`) covers global ids. The what-if preview treats `global:` like `pkg:`
  (`assets/whatif.js:113,194`), and the map labels the node `global:<name>`
  (`assets/graph.html:1610,2639,3458,4476,4494,4500`).
- D5. Collection needs no `--symbols`: the identifier capture runs in `parse`
  (`src/collect/native/typescript.rs:391-424`) on every native TS scan (`:869-873`). `.d.ts` files
  are skipped. A use is a node of kind `identifier` (not `type_identifier`) outside `type_context`,
  from a fixed list (process, Buffer, global, globalThis, window, self, document, navigator,
  location, localStorage, sessionStorage, indexedDB, fetch, XMLHttpRequest, WebSocket, console,
  setTimeout, setInterval, setImmediate, clearTimeout, clearInterval, queueMicrotask, crypto,
  __dirname, __filename), bound nowhere in the file. Today a `type_query` identifier goes to
  `value_uses` (`:394-424`), so `type W = typeof window` must be checked against `type_context`
  alone, while a runtime `typeof window !== 'undefined'` (no `type_query` node) counts. `require`,
  `module`, `exports` stay out. `binds_require` (`:96-125`) becomes `bound_names(root)` and
  `shadowed` (`:293-294`) reads `contains("require")`. Beyond its current kinds it covers
  `arrow_function` `parameter`, `catch_clause` `parameter`, `for_in_statement` `left` (also
  for-of), `assignment_pattern` and `object_assignment_pattern` `left`, `pair_pattern` `value`,
  `rest_pattern`, `import_specifier` `alias` and `namespace_import`. Property names and object keys
  are other node kinds. `queries/javascript.scm` gains `(identifier) @identifier` (no type uses in
  JS, so implicit types at `:958-961` are unchanged). A limitation beside `:1194` says a name
  declared anywhere in a file hides that global in the whole file, and `globalThis.process` counts
  only `globalThis`.
- D6. Additive v1.2 schema fields `globals` and `external_nodes[].global`, as `external_nodes` was
  (`CHANGELOG.md:81`). `ScanInput` reads `globals` with `#[serde(default)]`. `compat::v1`
  (`src/compat.rs`) strips nothing and the v1 schema is open, so `--legacy-v1` needs no change.
  `validate_scan` (`src/lib.rs:181-186,285`) checks scanned `from` and positive lines.

## Phases (one commit each)

### P1. `feat(collect): record host global uses in native JavaScript and TypeScript scans`
- `src/collect/native/typescript.rs` `Parsed.globals`, `bound_names`, the list, the `.d.ts` skip,
  the limitation; `queries/javascript.scm`; `RawGraph.globals` (`None` in other collectors);
  `src/model.rs` `GlobalUse`, `Document`/`ScanInput`/`Config` fields, `hydrate_config`; the field
  beside each `import_records` assignment (`src/cli/scan.rs:402` and retain `:330-332`,
  `src/cli/daemon_cache.rs:443`, `src/cli/provenance.rs:165`, `src/lib.rs:128,489`,
  `src/simulate.rs:78-88` renaming `from`, `src/recipes.rs:697`); validation; schema; CHANGELOG
  line for the `globals` field.
- Tests: `tests/native_typescript.rs` new `host_globals_are_recorded_once_per_file_unless_declared`.
  Counted: two `process` uses give one entry at the first line, `typeof window !== 'undefined'`,
  `setTimeout` in a `.js` file. None, one case per binding kind: `const process`, a `fetch`
  parameter, `docs.map(document => document)`, `catch (location) {}`, `for (const location of
  xs)`, `function f(crypto = 1) {}`, `const { a: crypto } = o`, `const [...navigator] = xs`,
  `import { x as crypto }`, `import * as console from 'c'`. Also none: `x.process`,
  `{ process: 1 }`, `let w: Window`, `type W = typeof window`, `require`, `module.exports`, any use
  in `a.d.ts`. `tests/native_engine.rs` `native_scan_and_symbols_need_no_external_collectors`
  asserts `globals` and schema validity. Run `cargo test --test native_typescript --test
  native_engine --test incremental_collect --test daemon`.

### P2. `feat(rules): forbid and allow host globals with global:<glob>`
- `src/resolution.rs` matched global nodes and edges, `policy_names`, accounting, identities;
  `src/rules.rs` D3 and D4; `ExternalNode.global`; `src/recipes.rs` `external_matches`;
  `src/cli/report.rs` count; `src/known.rs`, `src/target.rs`, `src/lib.rs` call `policy_names`;
  `src/infer_layout.rs:692-702` test map value `pkg:prisma`; `assets/whatif.js`,
  `assets/graph.html`; schema; guide after the package rules (`docs/guide.md:648-672`):
  `{ id: domain-no-env, from: "src/domain/**", to: "global:process" }`, the limitation, and that
  naming a global makes its other uses `default` forbidden or unplanned, fixed with an `allow`;
  CHANGELOG. Regenerate `tests/fixtures/whatif-parity.json` if the parity test asks for it.
- Tests: `tests/exposure_cli.rs` new `global_verdicts_reach_check_simulate_diff_and_legacy_export`
  on a saved scan with `globals` and a `typeorm` import. The rule exits 1 with
  `forbidden src/domain/a.ts -> global:process`. `pkg:**` does not flag it, `global:*` does not
  flag `typeorm`, `global:process` does not flag a file named `process`. Simulate's move removes
  it, diff lists it in `new_forbidden`, `--legacy-v1` validates, `--update-known accept` writes
  `to: "global:process"`, `report` counts one known external package. Without `globals` it exits 2
  with the D4 message. A layout without `global:` keeps `external_nodes` unchanged.
  `tests/exposure.rs` `category_and_threshold_validation_and_capture_binding` rejects empty,
  `from` and captured `global:`, and
  `external_ids_cannot_spoof_ordinary_file_policy_targets_or_move_destinations` covers a file named
  like a global id. Run `cargo test --test exposure_cli --test exposure --test rules --test
  infer_layout --test recipes --lib` and `node --test tests/whatif-parity.test.cjs
  tests/whatif-ui.test.cjs tests/exposure-ui.test.cjs`.

### P3. `docs(plans): record global use counts on trpc and excalidraw`
- Scan both pins from `showcase/repos.json` with `cargo run --release -- scan <clone>/<path>
  --engine native --language typescript -o <scratchpad>/<name>.json` and their manifest flags:
  trpc `10e2ef98f7842b94eb3026e4f9bcfe62cc39b088` at `packages/server`, excalidraw
  `4c37956ba5f04ec342a9d9d5891a38f22c5b7b4e`. Record under `## Results` here: files, `globals`
  entries, distinct names, files with any, top five names, JSON size against main.
- Gate: per repository, read the first ten entries sorted by `(from, name)`. More than one local
  binding misread as a global fails the gate and sends the fix back to P1.

## Risks

- R1. Under-reporting: a name declared in any scope hides the global in its whole file.
  `globalThis.process`, `window["fetch"]` and shorthand `{ process }` count the outer name or
  nothing. The guide and the limitation say so.
- R2. Naming a global in one rule turns its uses in other files into `default` forbidden edges or,
  under the default, unplanned edges, as for packages. The guide says to add an `allow`.
- R3. Reference-engine, non-JS and older saved scans cannot enforce `global:` (exit 2), including a
  diff base made by an older pinned version.
- R4. A forbidden global gets a non-mechanical cut step (no import record matches,
  `src/recipes.rs:341-378`), so `apply` never edits it.
- R5. Scans grow by one `globals` entry per file and global. P3 measures it.
