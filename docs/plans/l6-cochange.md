# L6: files that change together across a claim without an import

Approved 2026-10-21: keep the existing 3-shared-commit threshold, wording as below, two phases.
Context: L6 in [pr-architecture-review.md](../pr-architecture-review.md). CodeScene defines change
coupling as files modified in the same commit, by one author in a period, or under one ticket
(docs.enterprise.codescene.io change-coupling guide); Tornhill: look for change dependencies "that
span architectural boundaries" (adamtornhill.com, Kill the Clones); code-maat defaults
`--min-revs 5 --min-shared-revs 5 --min-coupling 30 --max-changeset-size 30`.

## Findings

- `src/cochange.rs` collects pairs from `git log --numstat -M --no-merges --relative -n<limit>`,
  skipping commits over 30 files (`MAX_FILES`) and keeping pairs with 3 or more shared commits
  (`MIN_COMMITS`); stored as `doc.cochange` (`src/model.rs` `Cochange`, `CochangePair`), restored
  by saved scans, set before the target is computed (`src/lib.rs`).
- The `hidden-coupling` finding (`src/findings.rs` ~1054-1078) already treats a pair as hidden
  when no edge links the files either way.
- `rules.forbid_slices` gives per rule and slice the `from` and `to` files; `target::analyze`
  merges them into `Slice.files`, dropping the split.
- The Action never passes `--cochange`; a default shallow checkout has about one commit.

## Decisions

- D1. A pair (a, b) from `doc.cochange.pairs` belongs to a slice when a is on some claim rule's
  `from` side and b on its `to` side in that slice, with no `doc.edges` edge either way. Same
  thresholds (3 shared commits, 30 files per commit). Advisory: verdict, progress and exits
  unchanged.
- D2. Per slice in `doc.target` as `cochanged: [CochangePair]` ordered from-side then to-side
  (skipped when empty); `check`/`report` text prints up to 20 lines
  `  co-change: {from} <-> {to}, {n} commits, no import` then `  ... and N more`. `diff`/`review`
  show only pairs new in the head (`Delta.newly_cochanged: [{claim, reason, source?, slice, files,
  commits}]`, computed only when both scans have co-change). Not in SARIF, `--fail-on` or the HTML.
- D3. The Action adds `--cochange` only when `git rev-parse --is-shallow-repository` is `false`;
  on a shallow checkout in a repository it writes
  `Co-change not read: shallow checkout; check out with fetch-depth: 0.` to the summary.
- Wording: Target line `, N newly co-changed pair(s)`; section
  `### Changed together across a claim, no import (N)`; rows
  `- {claim}, slice {slice}: {from} ↔ {to} ({n} commits)`.

## Phases (one commit each)

### P1. `feat(target): list files that change together across a claim without an import`
- `src/target.rs` Slice field, pair filter (with a `ponytail:` note on pairs × slices), text,
  `Delta.newly_cochanged`; `src/diff.rs` Target line, section, review section (rows counted in the
  budget) and JSON; schemas (`archgraph.v1.2`, `mcp.v1`); `src/mcp_tools.rs` review schema;
  `src/cli/mcp.rs` brief keys; guide Claims sentence; CHANGELOG.
- Tests (`tests/mcp_tools.rs`, a small `with_cochange(doc, pairs)` helper): with
  `claims(&[(x, d, 2)])` and pairs `(x, d, 4)`, `(y, e, 3)`, `(x, e, 5)`, `(y, src/shared/id.ts, 3)`,
  slice a has no `cochanged` (linked), slice b has exactly
  `[{"files": ["src/m/b/app/y.ts", "src/m/b/domain/e.ts"], "commits": 3}]` (x/e is cross-slice,
  y/shared off the sides) and the text line; old with `(y→e)` and the pair vs new without the
  import and the same pair gives `newly_fixed` b plus the `newly_cochanged` literal, the markdown
  Target line `Target: 2 of 2 slices conform (was 1 of 2), 1 newly co-changed pair.` and the row;
  reverse and a scan without co-change have no key; review shows the section and validates.

### P2. `feat(action): read co-change on full checkouts and show files changing together across a claim`
- `ci/action.sh` adds `--cochange` on non-shallow checkouts, the summary note on shallow ones
  (nothing outside git); guide Action paragraph; CHANGELOG. Verify `shellcheck ci/action.sh`,
  `bash ci/test-native-action.sh`, `bash ci/test-elixir-action.sh`.

## Risks

- R1. 3 shared commits is below code-maat (5) and CodeScene's examples (10); small busy slices may
  list incidental pairs.
- R2. Only direct edges count as a link (a → c → b shows as no import), as in `hidden-coupling`.
- R3. Renames inside a pull request change pair keys, so a pair can show as new.
- R4. `--cochange` also adds hidden-coupling findings and `git log` time to the push `report`.
- R5. `check .` on a live directory collects no co-change; `report --cochange` or a saved scan
  does.
