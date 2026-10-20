# Target what-if: delivery plan

Branch `feat/target-whatif` from 923e3b2. Builds on [target-check.md](target-check.md) and its
plan. Scope approved 2026-10-20: B1 export moves, B2 destination folder hints, B3 one summary
line, B4 live target in what-if. Approved decisions: D3 (diff `target.conforming`/`moved` become
`[{done, of}, {done, of}]`), D6 (sync all drifted preview layout keys).

## Findings that shape the plan

- F1. The preview's layout key lists (`assets/whatif.js:123`, `:128-129`) drifted from Rust
  `known_keys` (`src/rules.rs:261-291`): no `moves`, forbid/allow `source`, `layer-reason`,
  `early-smells`. Any of these sets `compiled = null` (`whatif.js:139`), so the preview reports
  no verdicts. GoldenGate: preview `forbidden 0` vs Rust 2.
- F2. `pattern()` globs each piece between captures separately (`whatif.js:48`, `:70`), so a
  capture inside braces (`{internal/<name>/handler*.go,internal/app/<name>/handler/*.go}`, the
  ADR-0003 form) throws "unbalanced glob braces". `whatifRuleShifts` (`whatif-ui.js:34`) too.
- F3. Claims-only layouts would paint every undecided import unplanned in preview
  (`whatif-ui.js:313`, chip `:177`); the main map suppresses this via `unflagged`
  (`graph.html:885`, `:891`, `:904`).
- F4. Selection is off during what-if (`whatif-ui.js:115`, `graph.html:4436`, `:1702`): the B4
  block renders inside `whatif-result`; B2 hints belong to normal-mode selection.
- F5. `Delta` stores done counts only (`src/target.rs:95-102`, `:386-393`); unreleased.
- F6. The finding move overlay already draws dashed arrows, ghost circle and "moves to" label
  (`graph.html:2325-2339`), positioned by `destination()` (`:1206-1220`), which falls back to an
  arbitrary module centre for folders the scan lacks.
- F7. `simulate::apply` builds without `target` (`src/simulate.rs:118`); parity needs
  `target::analyze(&after, rules)` in `tests/whatif.rs` `finish` (`:110-125`).
- F8. `report` already prints the target progress line (`src/cli/report.rs:242-247`).
- F9. Preview reachable forbids use a BFS from `e.to` over kind-included edges
  (`whatif.js:167-170`), no path; Rust claims record `(e.from, written)` (`src/rules.rs:937-940`).
- F10. GoldenGate: 59 new destination folders, at most 3 per slice, 95 files; 1,766 external
  edges keep the preview marked partial, but claims are file globs so target parity holds.

## Decisions

- D1 (B1): export exact paths, no `<name>` generalization. Exact `from` entries form slice `*`,
  fully priced by `check`, left out of progress.
- D2 (B2): placeholders only while one move row is selected; existing discs never move; a
  placeholder sits beside the module its first file leaves, fanned away from the map centre.
- D3 (B3): `Delta.conforming: [Count; 2]`, `Delta.moved: Option<[Count; 2]>`. Wording:
  `Target: 15 of 18 slices conform (was 16 of 18), 0 of 23 slices moved, 1 newly broken slice.`
  "(was …)" only when that count changed.
- D4 (B3): `report` unchanged (F8).
- D5 (B4): claim ids, reasons and sources come from `doc.target.claims`; JS computes
  `{id, verdict, slices: [{slice, broken}]}` plus `progress`.
- D6 (P1): add `layer-reason`, `early-smells` too.

## Phases

One commit each. Every phase verifies with `cargo fmt --check && cargo clippy --locked
--all-targets -- -D warnings && cargo test --locked --no-fail-fast` and
`cd npm && node --test ../tests/history-ui.test.cjs ../tests/context-ui.test.cjs ../tests/surface-ui.test.cjs ../tests/package-splits-ui.test.cjs ../tests/target-ui.test.cjs ../tests/whatif-parity.test.cjs ../tests/whatif-ui.test.cjs ../tests/exposure-ui.test.cjs`.
`S=/private/tmp/claude-501/-Users-dan-Work-archgraph/7f3410cb-671a-4322-b572-f08b30dad686/scratchpad` (`$S/gg.json`: GoldenGate scan with target rules and moves).

### P1. The preview accepts target layouts (F1, F2, F3)
- `assets/whatif.js`: `:123` add `layer-reason`, `early-smells`, `moves`; `:128`/`:129` add
  `source`; `checkFields(rules.moves || [], ['from','to'])`. `glob(raw, token, sub)`: when
  `token && raw.startsWith(token, i)` emit `sub` and skip the token. `pattern()`:
  `any = glob(raw, token, '[^/]*')`; `bind` returns
  `path.split('/').find(v => glob(raw, token, esc(v)).test(path))`; `matches` uses
  `glob(text, token, esc(value))` when bound; delete `fragments` (`:48`). Fix at the matcher so
  `whatifRuleShifts` works too.
- `assets/whatif-ui.js`: `:313` tone `unplanned` only when `!unflagged`; `:177` skip the
  unplanned chip when `unflagged`.
- `docs/guide.md` what-if paragraph (~469), `CHANGELOG.md`.
- Tests: `tests/whatif.rs` case `target-layout` (edges `src/a/handler.ts→src/a/repo.ts`,
  `src/b/handler.ts→src/shared/id.ts`, `src/b/repo.ts→src/shared/id.ts`,
  `src/app/c/handler/h.ts→lib/x.ts`; rules with `layer-reason`, forbid `layered` with reason and
  source over `{src/<name>/handler*.ts,src/app/<name>/handler/*.ts}` ↛
  `{src/<name>/repo*.ts,src/app/<name>/repo/*.ts}`, moves `src/<name>/handler*.ts →
  src/app/<name>/handler/` and `lib/x.ts → core/`; move `src/a/handler.ts →
  src/app/a/handler/handler.ts`), pin `("target-layout", 1, 1)` in the loop at `:471-484`,
  regenerate with `UPDATE_WHATIF_FIXTURES=1 cargo test --test whatif` (parity requires
  `warnings == []`, `whatif-parity.test.cjs:21`); `tests/whatif-parity.test.cjs`
  `bind('{a/<n>/h*.go,b/<n>/h/*.go}', 'b/x/h/m.go') === 'x'`; `tests/whatif-ui.test.cjs`
  `unflagged: false` in the context (`:36`) plus a test that `unflagged: true` leaves an
  unplanned import neutral in `whatifDrawing.files`.
- Verify: `node -e "const p=require('./assets/whatif.js'),d=require('$S/gg.json');console.time('a');const r=p.analyze(d);console.timeEnd('a');if(r.forbidden!==2||r.warnings.some(w=>/Rules preview/.test(w)))throw Error(r.forbidden+' '+r.warnings)"`, time vs 8 ms.
- Commit: `fix(report): preview rules written for a target: moves, sources and captures inside braces`

### P2. B3: one target line in the pull request summary
- `src/target.rs`: `Count` derives `PartialEq, Eq`; `Delta` fields `[Count; 2]` /
  `Option<[Count; 2]>`; `delta` = `[old.progress.conforming, new.progress.conforming]`,
  `moved: old.progress.moved.zip(new.progress.moved).map(|(a, b)| [a, b])`.
- `src/diff.rs`: after the headline (`:110-117`) append `\nTarget: …\n` (D3, last part
  `count(n, "newly broken slice")`); replace `:157-179` with `### Newly broken slices (N)`, rows
  as at `:170-176`.
- Schema `schema/archgraph.v1.2.schema.json:3641-3660`: items become `{done, of}`.
- `docs/guide.md:377`, `CHANGELOG.md` diff entry.
- Tests (`tests/diff.rs:394-466`): `target_scan` rules gain
  `moves: [{ from: "src/m/<name>/app/**", to: "src/app/<name>/" }]`; new vs old:
  `"\nTarget: 1 of 2 slices conform (was 2 of 2), 0 of 2 slices moved, 1 newly broken slice.\n"`
  then `### Newly broken slices (1)` with the row, no `Conforming slices`; JSON
  `target.conforming == [{"done":2,"of":2},{"done":1,"of":2}]`,
  `moved == [{"done":0,"of":2},{"done":0,"of":2}]`; old vs old
  `"\nTarget: 2 of 2 slices conform, 0 of 2 slices moved.\n"`, no `Newly broken`; plain case no
  `Target:`; `--fail-on new-forbidden` exits 1.
- Verify: `bash ci/test-native-action.sh`.
- Commit: `feat(diff): lead the pull request summary with the target's slices conforming and moved`

### P3. B1: copy the what-if move list as layout moves
- `assets/graph.html:643-644`: 4th labelled textarea `whatif-moves` + button `whatif-copy-moves`.
- `assets/whatif-ui.js`: `:3-7` add `moves: $("whatif-moves")`; `:21`
  `whatifUI.moves.value = whatifLayoutMoves(moves)`; `:108` clear; `:358` listener.
- New `whatifLayoutMoves(moves)` beside `whatifRuleShifts`: header
  `moves:   # exact paths, first match wins: keep these above other moves`; each move
  `  - { from: <JSON string>, to: <JSON string of dirname(to) + "/"> }`, basename `*?[]{}`
  wrapped as `[c]`; a comment `  # A → B: not expressible as a move rule` when the destination
  is the root, a source folder has `*?[]{}` (`src/rules.rs:966-971`), any segment has `<`/`>`
  (`src/rules.rs:340-349`), or the destination folder has glob characters (`src/rules.rs:628-640`).
- `docs/guide.md` export paragraph, `CHANGELOG.md`.
- Tests: `tests/whatif-ui.test.cjs` chained test (`:85-107`) exact literal for two moves, empty
  after Clear all; `whatifLayoutMoves` with `app/[id].ts→web/[id].ts` (`from: "app/[[]id[]].ts"`),
  `a/x.ts→x.ts` (comment), `p/[q]/x.ts→r/x.ts` (comment); `tests/rules.rs` test near `:278` via
  `edges_scan` with the JS literal for `app/[id].ts` and `app/helper.ts`: `target.moves ==
  [{slice: "*", files: [...]}]`, `progress.moved == {done: 0, of: 0}`.
- Commit: `feat(report): copy the what-if move list as layout moves`

### P4. B2: planned destination folders for a selected move
- `assets/graph.html`: `const scanDirs` near `:823` (every folder prefix of `doc.files`;
  `whatif-ui.js:18` uses it, delete its local `known`); `targetMoves(m)` in the target panel
  block, one overlay entry per file `{file, to, at, r?, note?}`: existing destinations `at` from
  `destination(to)` (`:1206`); new folders `r = SP * 0.56 * Math.sqrt(n) + R_DOT + 1` (`:1013`),
  `at` getter at distance `modR[home] + GAP/2 + r` from the home module centre, angle away from
  the mean of `mx`/`my`, offset `(k - (n-1)/2) * 0.7`; first entry per folder `note`
  (`new folder <dir>/` or `moves to <dir>/`), rest `note: null`. `:3981` `moveSel` adds
  `moves: targetMoves(m)`; `:1758` target focus sets `overlay = { moves: t.moves || [], links: [] }`;
  `:1946` passes `moves: t.moves || []`; draw loop `:2325-2339` uses `mv.r ?? dotR * 2`, skips
  ghost and label when `mv.note === null`, label `mv.note ?? "moves to " + short(mv.to)`.
- Tests (`tests/target-ui.test.cjs`): stubs `mx=[0]`, `my=[0]`, `modR=[30]`, `files=[{mod:0},…]`,
  `SP=10`, `R_DOT=2.6`, `GAP=34`, `scanDirs=new Set(['internal','internal/sop'])`, `destination`;
  selecting the `sop` move gives one entry, `note === 'new folder internal/sop/handler/'`, `r`
  9.2, `at` `[56.2, 0]`; an existing destination has no `r` and `at` from `destination`; wiring
  test (`:110-117`) matches `overlay = { moves: t.moves`.
- Verify: render `$S/gg.json`, select `sop` under Move cost: 3 discs, no existing disc moved.
- Commit: `feat(report): show planned destination folders for a selected move as empty discs`

### P5. B4: live claim verdicts and slice progress while previewing moves
- `assets/whatif.js`: extract `forbids(r, e)` from `verdict` (`:164-171`), mirroring Rust
  `forbids` (`src/rules.rs:1015-1044`): bound value (`null` without capture) or `undefined`,
  reusing the BFS (F9); `verdict` loops over it. Compile `moves` (`:131-138`):
  `{p: pattern(m.from), arrived: pattern(m.to + '**')}` (`src/rules.rs:641`). After verdicts,
  when `doc.target && compiled`, follow `src/target.rs:141-227`: per `doc.target.claims[].id`
  the rules with that id; per bound value `from` files via `bind`, `to` files via `matches`,
  `hits` every edge where `forbids !== undefined` as `[from, original_to || to]`; bound when a
  hit or both sides; precedence vacuous, holds, broken, holds_in (`src/target.rs:189-194`);
  `conforming` over non-null values; pending = first move whose `from` binds the file, arrivals
  add the key; `moved` only with moves, non-null keys; slices sorted `null` (`*`) first then
  string; broken sorted by `(from, to)` tuple. Return as `projection.target`, else `null`.
- `assets/graph.html`: `targetContext(target = doc.target, out = panel, was)`; with `was`, the
  progress line adds "(was X of Y)" per changed count, the heading reads
  "Move cost, from the scan (not previewed)", `targetRow` has no "Show on map".
- `assets/whatif-ui.js` `whatifShow` after the instability details (`:280`): when `doc.target &&
  result.after.target`, call `targetContext({ ...doc.target, claims: after.claims merged with scan
  reason/source by id and notes: [], progress: after.progress, notes: [] }, box,
  doc.target.progress)` and append `box`.
- `docs/guide.md` what-if paragraph, `CHANGELOG.md`.
- Tests: `tests/whatif.rs` `projection` (`:16-73`) gains `"target"`
  (`{claims: [{id, verdict, slices: [{slice, broken}]}], progress}` or `null`); `finish` sets
  `after.target = archgraph::target::analyze(&after, rules)`; cases `target-claim-binding`
  (`sliced_scan` edges `tests/rules.rs:186-193`, move `src/m/a/domain/d.ts → src/shared/d.ts`:
  holds_in → holds, 1/2 → 1/1), `target-first-match` (id-less forbid first, `tests/rules.rs:258-260`:
  broken stays broken), `target-reachable-claim` (edges `:302-303` with reason, move
  `server/private.ts → shared/private.ts`: broken → vacuous); `target-layout` pins moved
  1/3 → 2/3; literals beside `:471`; regenerate. `whatif-parity.test.cjs` unchanged (whole
  projection `deepEqual` covers it). `whatif-ui.test.cjs`: `target-layout` preview with a stubbed
  `targetContext` records its arguments (claims `[{id: 'layered', reason, verdict: 'holds_in', …}]`,
  `progress.moved {done:2, of:3}`, `was` = `doc.target.progress`). `target-ui.test.cjs`:
  `targetContext(target(), box, {conforming: {done:2, of:2}, moved: {done:1, of:2}})` reads
  `1 of 2 slices conform (was 2 of 2); 1 of 2 slices moved.` and
  `Move cost, from the scan (not previewed)`; opened rows have no "Show on map".
- Verify (real data parity): `node -e "const p=require('./assets/whatif.js'),a=require('assert'),d=require('$S/gg.json');console.time('a');const t=p.analyze(d).target;console.timeEnd('a');a.deepStrictEqual(JSON.parse(JSON.stringify(t)),{claims:d.target.claims.map(c=>({id:c.id,verdict:c.verdict,slices:c.slices.map(s=>({slice:s.slice,broken:s.broken}))})),progress:d.target.progress})"`; in the rendered page preview `internal/sop/handler.go → internal/app/sop/handler`.
- Commit: `feat(report): show live claim verdicts and slice progress while previewing moves`

## Risks

- R1. P5 runs the forbid step per edge per claim rule and `bind` builds a RegExp per segment:
  measure on `gg.json` in P1 and P5; above ~100 ms memoize `bind` per path (`ponytail:` comment).
- R2. A placeholder can overlap a neighbouring disc; it is faint and only shown on selection.
- R3. Regenerated fixtures add `"target": null` to every case: large, mechanical diff.
- R4. P1 changes GoldenGate previews from "Rules preview incomplete" to real verdicts (intended).
- R5. Exported exact moves must sit above capture moves (first match wins); a header comment says so.
- R6. JS `<` compares UTF-16 code units, Rust bytes: astral-plane path order can differ (existing).
