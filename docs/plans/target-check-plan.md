# Target check: delivery plan

Brief: [target-check.md](target-check.md). Branch `feat/target-check`, after 9494b25 (Go folder grouping).

## Architecture

A team states a target in `layout.yaml`. Claims are `forbid` rules with an `id` and a `reason`
quoting the proposal, written over today's paths and, with brace alternation, over target paths
too. An optional `moves:` section maps today's paths to target paths. Against today's code
archgraph reports a verdict per claim (`holds`, `holds in N of M slices`, `broken` with the
imports, `vacuous`), a cost per slice for each move (files, import lines, Go unexported names
to export, Go package cycles the moves would create), and progress (slices conforming, slices
moved), in `check` text and JSON, the HTML report, the CI job summary and MCP. A slice is one
value of a rule's `<name>` capture.

| Module | Interface | Depth note | New / changed |
| --- | --- | --- | --- |
| `src/target.rs` | `analyze(&Document, &Compiled) -> Option<Report>`, `delta`, `text`, `Report` | Deep: hides claim grouping, slice binding with vacuity, evaluation independent of first match, move resolution, import lines, Go names, whole-target simulation with per-slice cycle attribution. No separate moves module. | new |
| `src/rules.rs` | `Rules.moves`, `Compiled::forbid_slices(files, edges, externals)`, `Compiled::resolve_moves(files)` | The only module that sees `Pattern`/`bind` (`rules.rs:341-390`, `:635-645`) | changed |
| `src/lib.rs` | one call after `findings::find` (`lib.rs:274`) | wiring | changed |
| `src/model.rs` | `Document.target`, `Diff.target`, `Collector.bound: Vec<BoundPair>`, `Collector::bound_paths()` | data | changed |
| `src/collect/go_dump.go`, `src/collect/go.rs`, `src/cli/scan.rs` | `bound: [{from, to, names?, together?}]` | evidence only | changed |
| `src/cli/check.rs`, `src/cli/report.rs` | print `target::text`, `"target"` in `--brief` JSON | two callers of one printer | changed |
| `src/diff.rs` | `Diff.target`, `### Target` markdown | reuses `esc` (`diff.rs:194`) | changed |
| `assets/graph.html` | `targetContext()`, selection type `"target"` | reuses `edgeOf` (`:1417`), finding focus (`:1748`), `zoomToFinding` (`:1927`) | changed |
| `src/mcp_tools.rs` | tool `target` | generic `bounded` (`src/cli/mcp.rs:396`) trims it | changed |

One shared extraction: the forbid step and `matches_target` in `Compiled::verdict`
(`rules.rs:851-888`) become private methods shared by `verdict` and `forbid_slices`, so `check`
and a claim cannot disagree. Skipped on purpose: report/panel traits, per-language cost
strategies, the capture value on `Edge`, cleaning `reasons`/`sources` loops (`rules.rs:160-204`),
mapping target paths back through `moves`, and `layers` as claims.

## Data model

```yaml
moves:   # first match wins, like layers
  - { from: "internal/<name>/handler*.go",    to: "internal/app/<name>/handler/" }
  - { from: "internal/<name>/service*.go",    to: "internal/app/<name>/service/" }
  - { from: "internal/<name>/repository*.go", to: "internal/app/<name>/repository/" }
```

`from` goes through `Pattern::new`, rejecting `pkg:`/`category:`. `to` passes `check_template`
(`rules.rs:379`), has no glob characters, ends in `/`, and passes `is_relative_path`
(`src/collect/mod.rs:209`). Move strings join the control-character scan (`rules.rs:460-495`)
and `known_keys` (`rules.rs:246`). A file's destination is the bound `to` plus its path below
the literal leading directories of the bound `from`, so `**` keeps subfolders.

```json
"target": {
  "claims": [{"id": "layered", "reason": "Dependencies point inward (slide 8)", "source": null,
    "verdict": "holds_in",
    "slices": [{"slice": "interview", "files": ["..."], "broken": [{"from": "...", "to": "..."}]}],
    "notes": ["forbid[2] `to` matches no file"]}],
  "moves": [{"slice": "project",
    "files": [{"file": "internal/project/handler.go", "to": "internal/app/project/handler/handler.go"}],
    "import_lines": 9,
    "names": [{"name": "Service.load", "declared_in": "internal/project/service.go", "used_by": ["internal/project/handler.go"]}],
    "together": [{"from": "...", "to": "..."}],
    "new_cycles": [{"from": "internal/project/routes.go", "to": "internal/app/project/handler/handler.go"}]}],
  "progress": {"conforming": {"done": 16, "of": 18}, "moved": {"done": 0, "of": 23}},
  "notes": []
}
```

- Claim: every forbid sharing an `id`, at least one with a `reason`. The claim shows the first
  `reason` and `source` in source order. Uncaptured rules of a claim that also has captured rules
  form one extra slice `*`, which progress leaves out.
- Binding: a slice is bound when one of the claim's rules has, in that slice, a file matching
  `from` and a target matching `to`.
- Verdict: `vacuous` with no bound slice, `holds` when all bound slices hold, `holds_in` when
  some do, `broken` when none do. For a reachable forbid, a hit is the `shortest_path` evidence.
- Notes live at two levels: per claim (a rule side matching no file) and in `target.notes`
  (a move rule matching nothing, a simulation error, names unknown for the engine).
- A slice is moved when its `files` is empty. Arrivals (files already at the target) are found
  by matching `to` + `**` as a `Pattern` with the same capture.
- `import_lines`: Go counts distinct (importing file after the move, imported package directory
  after the move) pairs, over `doc.edges` whose files sit in different directories after the
  move, that were not present before the move. Other languages count `doc.imports` records
  whose importer or resolved target moves.
- `names`: `null` unknown (native Go, other languages), `[]` none needed. Pairs whose two files
  land in one directory are left out.
- Progress: slices keyed by bare capture value across claims; uncaptured claims stay out of
  `of`. `moved.of` counts every slice with a file matching a move's `from` plus slices already
  at their target.
- Diff: `"target": {"conforming": [old, new], "moved": [old, new], "newly_broken": [{"claim", "slice", "imports": [Link]}]}`.
- Go carrier: `model::BoundPair {from, to, names (skip if empty), together (skip if false)}`.
  `Collector.bound` is filtered to scanned files. `bound_files` stays only for native Go, which can only say every file is
  bound; readers use `Collector::bound_paths()` (replaces `findings.rs:324`, `:1318`).

## Critic round 1, resolved

- K1 accepted: one `simulate::apply` (`simulate.rs:46-134`) of every pending move, then
  `graph::new_cyclic_import_pairs` (`graph.rs:543-570`), Go package cycles only. A pair belongs
  to slice `s` when `dir_of` of either file is a source or destination directory of `s`'s moves.
  An `apply` error becomes a report note; files, import lines and names still report. Replaces
  custom `conflicts`.
- K2 accepted, no code: globset 0.4.20 with `literal_separator` matches
  `{internal/project/handler*.go,internal/app/project/handler/*.go}` (tested); `<name>` repeats
  safely (`rules.rs:334`, `:367-377`). Guide and skill use this form for mid-migration safety.
- K3 resolved by the user: O1, claim {h↛r, s↛h, r↛h}, 16 of 18.
- K4 accepted: same-directory pairs excluded; names qualified (`key(obj)` at `go_dump.go:662`,
  receiver type for selections, bare for composite-literal keys); a limitation line when pairs
  touching excluded files drop.
- K5 accepted: one carrier (`bound`), `bound_files` unshipped at v0.1.0.
- K6 accepted: exactly two `pub(crate)` lookups.
- K7 accepted: no `holding`, no stderr warning, `holds_in`, notes per claim and in `target.notes`.
- K8 accepted: a note per rule side and per move that match no file.
- K9 accepted: documented slice key; uncaptured claims out of progress.
- K10 accepted: guide line on the no-base push path exiting with `report`'s status.

## Residual risks

- R1: users with reasoned forbids get `target` and longer `check` text; golden test updated.
- R2: simulation reruns per analysis while moves are pending; Phase 3 measures GoldenGate.
- R3: a brace glob over `internal/*/...` binds slice `app` if `internal/app/handler.go` exists;
  the guide warns to keep the target root free of files.
- R4: cycles come from simulating all pending moves together; a cycle that needs two slices
  moving is listed under both.
- R5: an older archgraph rejects `moves`; version-pinning note in the guide.
- R6: `tests/fixtures/record.sh` needs depcruise and grimp; Phase 4 re-records only Go.

## Phases

One phase = one commit. `S=/private/tmp/claude-501/-Users-dan-Work-archgraph/7f3410cb-671a-4322-b572-f08b30dad686/scratchpad`.
Every phase verifies with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` plus the steps listed.

### Phase 1: Report a verdict per claim
- Goal: `check` prints and serializes one verdict per claim; breaks are evaluated against
  every forbid rule, empty rule sides are noted.
- Depends on: none.
- Touches: `src/rules.rs` (extract `matches_target` and the forbid step; `forbid_slices`,
  `SliceEvidence { from, to, hits }`), `src/target.rs` (claims, `progress.conforming`, `text`),
  `src/lib.rs` (after `:274`), `src/model.rs` (`Document.target`, `None` at `lib.rs:500`),
  `src/cli/check.rs` (text after `:145`, brief JSON `:85-91`, exit code unchanged), schema
  (`$defs/target`, scan property near `:1916`), `docs/guide.md` Rules file (`:913`), `CHANGELOG.md`.
- Tests: golden `ts_fda_check_prints_violations_and_exits_one` (`tests/rules.rs:150`) gains claim
  lines and the `target:` line; new `tests/rules.rs` synthetic test (`tests/common/mod.rs:172`)
  with `{id: app-no-domain, from: "src/m/<name>/app/**", to: "src/m/<name>/domain/**", reason}`:
  slice a broken, b holds, c (domain only) unbound → `holds_in` over 2; a claim over
  `src/nowhere/**` is `vacuous`; a claim whose `to` matches nothing gets a note; an earlier
  id-less forbid catching the same import leaves the claim `broken`; `validate(doc)` passes.
- Text: `claim <id>: holds in N of M slices; broken in a, b reason="..."`, up to 20 indented
  imports, notes, `target: N of M slices conform`.
- Verify: `$S/gg-layout.yaml` with three `layered` forbids, reason
  `"Dependencies point inward (slide 8)"`, written out per role (handler ↛ repository,
  service ↛ handler, repository ↛ handler), each side in the form
  `{internal/<name>/handler*.go,internal/app/<name>/handler/*.go}` with the role spelled out; `cargo run --locked -- check /private/tmp/goldengate-archgraph-20261020/backend/archgraph.json --rules $S/gg-layout.yaml`
  prints `holds in 16 of 18 slices; broken in interview, sop` with the two imports, exit 1.
- Commit: `feat(check): report a verdict per claim: holds, holds in N of M slices, broken or vacuous`

### Phase 2: Add `moves`, slices moved, files and import lines
- Goal: `moves:` parses and validates; each slice reports pending files, import lines and
  notes; progress reports slices moved.
- Depends on: Phase 1.
- Touches: `src/rules.rs` (`MoveRule`, `Rules.moves`, `known_keys`, control-character scan,
  `resolve_moves`), `src/target.rs`, `src/cli/check.rs` (`move <slice>: N files, M import lines`),
  schema (rules `moves` near `:2169`, `target.moves`), `docs/guide.md` (moves, brace form, R3, R5),
  `CHANGELOG.md`.
- Tests: `rules_errors` (`tests/rules.rs:462`) rejects `to` with `*`, `to` without `/`, a `to`
  capture `from` does not bind, `from: "pkg:x"`, an unknown key under `moves[0]`; synthetic test
  with slice a pending (2 files, destinations, `import_lines`), slice b already moved (progress
  `1 of 2`), a move matching nothing noted in `target.notes`; `import_lines` asserted exactly
  for a moved importer and for a moved target; mid-migration test where brace sides bind an unmoved,
  a moved and a mixed slice, and the mixed import is broken.
- Notes: first match wins (as `layer_for`, `rules.rs:656`); literal prefix from `instance`.
- Verify: add the three moves to `$S/gg-layout.yaml`; `check` prints
  `target: 16 of 18 slices conform, 0 of 23 slices moved`; record the `project` import-line count.
- Commit: `feat(rules): add moves and report per-slice files, import lines and slices moved`

### Phase 3: Simulate the target: new package cycles
- Goal: each pending slice lists the Go package cycles applying every pending move would
  create.
- Depends on: Phase 2.
- Touches: `src/target.rs` (one `simulate::apply`, `graph::new_cyclic_import_pairs`, attribution,
  apply error as a note), schema (`new_cycles`), `src/cli/check.rs` (`, K new package cycles`), `docs/guide.md` (Go only, simulated together).
- Tests: `tests/golang.rs` on recorded `go-resolve` with a `write_tmp` layout moving
  `server/server.go` (cycle `handle.go` → `server.go` → `base.go`) → non-empty `new_cycles`;
  moving `store/open.go` → empty; a move onto an existing
  path → a report note naming it, other fields present; a TS synthetic doc → empty `new_cycles`.
- Verify: GoldenGate `check` under `time` lists new package cycles for 20 of 23 slices (confirm
  the count on the first run); `kbretire`, `process_map` and `task_miner_export` list none.
  Record `check` runtime vs Phase 2 and time one daemon refresh (`daemon_cache.rs:326`); if the
  refresh cost matters, skip the simulation on the cached path.
- Commit: `feat(target): simulate pending moves and report the package cycles they create`

### Phase 4: Record Go unexported names and list them in move cost
- Goal: Go (reference engine) move cost lists qualified unexported names to export and pairs
  that must move together. Names: a selector with a Selection records `Type.name` and adds
  `n.Sel` to a skip set so the bare ident is not recorded twice; promoted fields name the outer
  receiver type (documented); composite-literal keys are qualified through `info.Types[lit].Type`.
- Depends on: Phase 2.
- Touches: `src/collect/go_dump.go` (names at `:603-605`, `together` at `:523`, `:534`, output
  `:223`), `src/collect/go.rs` (`DumpEdge` `#[serde(default)] names, together` at `:52`,
  `bound_pairs` replaces `bound_files` at `:249`), `src/model.rs` (`BoundPair`, `Collector.bound`,
  `bound_paths()`, doc comment `:157-161`), `src/findings.rs:324`, `:1318`, `src/cli/scan.rs:334`
  (+ dropped-pairs limitation), `src/lib.rs` `validate_scan` (`:113`), `src/target.rs`, schema
  (collector `bound` near `:1405`), re-recorded `tests/fixtures/go-*/go.json`, `docs/guide.md`,
  `skills/archgraph/SKILL.md`.
- Tests: `tests/golang.rs:28` asserts `bound` pairs (`together` on `server/` method pairs, names on
  `store/` pairs), same at `:160`, `:274`, `:282`, `:331`; new move-cost test on `go-resolve`
  (moving one `store` file lists names with `declared_in`/`used_by`; moving it with its partner
  into one directory lists neither; a selector field reports exactly `Server.db` for `handle.go` → `server.go`, so a bare
  duplicate fails); `tests/native_go.rs`
  keeps `bound_files` and `names: null`; move-avoidance tests stay green.
- Verify: plus `cargo test --locked --test live -- --ignored`; `git diff --stat tests/fixtures`
  shows only `go.json`; scan `/Users/dan/Work/DRAWBRIDGE/goldengate/apps/api` (note its commit and dirty state; the
  recorded scan is `502938cb`, dirty) with `$S/gg-layout.yaml` to `$S/gg.json`, `check` lists
  names under `move project`; record size vs the 1,609,009-byte baseline.
- Commit: `feat(go): record qualified unexported names on bound pairs and list them in move cost`

### Phase 5: Show the Target panel in the HTML report
- Goal: each claim with reason, verdict, broken slices and imports, then progress and move
  cost; selecting a row highlights its files and broken imports.
- Depends on: Phase 3 (Phase 4 for names).
- Touches: `assets/graph.html` (`targetContext()` between target-panel markers, called in
  `overview()` before `packageSplitContext()` at `:3853`; selection `{type: "target", ...}` through
  the finding branch `:1748`, `zoomToTarget` `:1941`, `showPanel` `:4343`), `.github/workflows/ci.yml:106`.
- Tests: new `tests/target-ui.test.cjs` cloned from `tests/package-splits-ui.test.cjs`: renders
  `holds in 1 of 2 slices` and the reason; a slice lists its imports; selecting a slice builds
  `fs`/`fe` through `edgeOf`; move rows show files, import lines, names, cycles; no `doc.target`
  renders nothing; no repository text through `innerHTML`.
- Verify: `cd npm && node --test ../tests/target-ui.test.cjs ../tests/package-splits-ui.test.cjs`;
  render `$S/gg.json` to `$S/gg.html` and select `interview`.
- Commit: `feat(report): add a Target panel with claim verdicts, slice progress and move cost`

### Phase 6: Report target progress in the CI summary
- Goal: the pull request summary shows progress before and after and newly broken slices; the
  push path shows the target block from `report`; the gate stays `fail-on`.
- Depends on: Phase 2.
- Touches: `src/target.rs` (`delta`), `src/model.rs` (`Diff.target`, `:849`), `src/diff.rs`
  (`diff`, `### Target` in `markdown` at `:95`), `src/cli/report.rs` (`summary` at `:83`), schema
  (`diff`), `docs/guide.md` (action section, K10). `ci/action.sh` unchanged.
- Tests: `tests/diff.rs` (markdown test at `:182`): head adds an `app → domain` import in slice b;
  markdown has `### Target`, conforming `2 → 1`, `newly_broken` names the claim, slice b and the
  import; `diff --fail-on new-forbidden` exits 1.
- Verify: plus `bash ci/test-native-action.sh` and `shellcheck ci/action.sh`.
- Commit: `feat(diff): show target progress and newly broken claims in the pull request summary`

### Phase 7: Draft a target from a proposal: MCP tool and skill
- Goal: an agent drafts `layout.yaml` from a deck or ADR by following the skill and reads
  verdicts, notes and costs through MCP `target` or `check --brief`; non-dependency claims are
  reported as outside archgraph.
- Depends on: Phase 3.
- Touches: `src/mcp_tools.rs` (schema in `schemas()` `:574`, output inlined like `:588-595`,
  dispatch near `"rules"` `:420`, `validate` `:117`), `schema/mcp.v1.schema.json`,
  `skills/archgraph-rules/SKILL.md` ("Draft a target from a proposal"; read move cost from
  `check --brief`, since the MCP result cap of 2,000 bytes (`mcp.rs:397`) trims `moves` first), `skills/archgraph/SKILL.md:19`.
- Tests: `tests/mcp_tools.rs` via `tool()` (`:36`): a doc with a claim returns `claims[0].verdict`;
  a doc without rules returns `null`, no error; `tests/mcp_stdio.rs:235` tools list and schema.
- Verify: plus `cargo run --locked -- sync-rules . --check`.
- Commit: `feat(mcp): add a target tool and document drafting a target from a proposal`

## ADR-worthy decisions

- ADR-0001: a claim is every forbid sharing an id, judged against every import regardless of
  first-match order (`rules.rs:869-888`), so a claim can be broken by an import `check`
  attributes to an earlier rule.
- ADR-0002: a slice counts only when a rule could fail in it (both sides present); a claim with
  no bound slice is `vacuous`. GoldenGate: 16 of 18, not 23.
- ADR-0003: target paths live in the rules through brace alternation; `moves:` is only for cost
  and progress, never for verdicts (no post-move verdict).
- ADR-0004: move cost comes from one simulation of the whole target, cycles attributed by
  directory, run during analysis while moves are pending.
- ADR-0005: Go bound pairs with qualified names are the single binding evidence; `bound_files`
  only where no pair evidence exists.

## Definition of done

- Phases 1-7 committed, one commit each, fmt, clippy and tests green after each.
- Node UI tests green (set from `ci.yml:106`).
- GoldenGate: 16 of 18 slices conform, broken in `interview` and `sop` with the two imports;
  0 of 23 moved, with names and package cycles per slice; a glob-less claim `vacuous`; an added
  handler → repository import drops conforming and fails `diff --fail-on new-forbidden`.
- Independent verifier or critic check against this plan and the brief.
- Docs: guide Rules file and action sections, both skills, `CHANGELOG.md`, `docs/adr/0001`-`0005`.

## Test spec (Gate 2)

Seams: S1 `check` text and `--brief` JSON; S2 scan document `target` (via `check -o` or the
`synthetic` helper); S3 `layout.yaml` load errors (`rules_errors`); S4 Go collector output
(`collector.bound`); S5 `diff --format markdown` and `--fail-on`; S6 HTML panel (marker-cut Node
test); S7 MCP `target` tool (`tool()` helper, stdio tools list). Expected values are fixed
literals. Each phase is red → green on its own tests.

| Phase | Test | Seam | Behaviour |
| --- | --- | --- | --- |
| P1 | T1 | S1 | ts-fda golden gains `claim shared-no-domain: broken … reason="shared imports no domain"` and a `target:` line; exit 1 |
| P1 | T2 | S2 | slice with an app→domain import broken, a clean slice holds, a domain-only slice unbound: `holds in 1 of 2 slices` |
| P1 | T3 | S2 | a claim over `src/nowhere/**` is `vacuous`; a claim whose `to` matches nothing gets a note |
| P1 | T4 | S2 | an earlier id-less forbid catching the same import leaves the claim `broken` |
| P2 | T5 | S3 | `moves` rejects a glob in `to`, a missing trailing `/`, an unbound capture, `pkg:`, an unknown key, naming the field |
| P2 | T6 | S2 | a pending slice lists destinations and exact `import_lines`; an arrived slice counts moved (`1 of 2`); a move matching nothing noted in `target.notes` |
| P2 | T7 | S2 | mid-migration: a brace-form claim flags a moved handler importing an unmoved repository |
| P3 | T8 | S2 | Go `go-resolve`: moving `server/server.go` lists a new package cycle; moving `store/open.go` lists none |
| P3 | T9 | S2 | a move onto an existing path gives a report note naming it, files and import lines still report; a TS doc has no `new_cycles` |
| P4 | T10 | S4 | `collector.bound` carries `together` on `server/` method pairs and names on `store/` pairs |
| P4 | T11 | S2 | moving one `store` file lists names with `declared_in`/`used_by`; moving it with its partner lists none |
| P4 | T12 | S2 | a selector field reports exactly `Server.db`, no bare `db`; native Go gives `names: null` |
| P5 | T13 | S6 | the panel renders `holds in 1 of 2 slices` and the reason; a slice lists its imports |
| P5 | T14 | S6 | selecting a slice highlights its files and imports; move rows show cost; no target renders nothing; no `innerHTML` with repository text |
| P6 | T15 | S5 | a PR adding an import in slice b: markdown has `### Target`, conforming `2 → 1`, newly broken slice b with the import |
| P6 | T16 | S5 | the same diff with `--fail-on new-forbidden` exits 1 |
| P7 | T17 | S7 | `target` returns `claims[0].verdict` for a doc with a claim, and `null` without error for a doc without rules |
| P7 | T18 | S7 | the tools list includes `target` and its output validates against the schema |
