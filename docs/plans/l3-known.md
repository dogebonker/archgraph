# L3: a committed list of known violations that only shrinks

Branch `feat/l3-baseline` from main 992d62b. Approved 2026-10-21: stale entries fail until shrunk,
identity `(rule, from, to)`, file `layout.known.yaml`, three phases. Defaults kept: no warning when
the file grows (Q4), no mapping through `moves:` yet (Q5). Context: L3 in
[pr-architecture-review.md](../pr-architecture-review.md); borrowed from dependency-cruiser
(`--baseline`, `--ignore-known`, `--baseline-mode shrink-only`), ArchUnit `FreezingArchRule`
(auto-shrink, `allowStoreCreation=false`), Packwerk `package_todo.yml` (`update-todo`, `strict`),
jQAssistant baselines.

## Decisions

- D1. Identity `(rule, from, to)`: `rule` is the edge's first-match label (claim id, `forbid[i]`,
  `layers`, `public`, `entrypoints`, `default`, `src/rules.rs:1078-1164`); `from` the importer; `to`
  the written target (`original_to` else `to`) printed as `check` prints it, `pkg:<package>` for
  externals (`policy_target`, moved from `src/cli/check.rs:25-31` into the new module). No line, no
  kind. Mirrors the SARIF fingerprint.
- D2. A moved file or renamed rule makes the entry stale and the import unlisted; `check
  --update-known accept` records it again. Unnamed rules renumber: the guide says to give rules an
  `id`.
- D3. A stale entry fails (exit 1) with
  `stale known <from> -> <to> (<rule>): no longer forbidden; run archgraph check --update-known shrink`.
- D4. File `<rules stem>.known.yaml` beside the rules file (`layout.yaml` → `layout.known.yaml`,
  `ci/strict.yaml` → `ci/strict.known.yaml`); not a key in layout.yaml (comments, rules-changed
  check, unknown-key rejection); the word "baseline" avoided (edit-gate snapshots, recipe
  `baseline_id`, MCP base scan already use it). Format: one flow mapping per line, JSON-quoted
  scalars, sorted by `(from, to, rule)`, deduplicated, strict (unknown keys, control characters):
  ```yaml
  # Forbidden imports layout.yaml already has. check fails on any other, and on an entry that no
  # longer occurs. Shrink with: archgraph check --update-known shrink
  version: 1
  known:
    - { rule: "forbid[0]", from: "src/adapters/in/http.ts", to: "src/adapters/out/pg.ts" }
  ```
  No per-entry reason (the rule's reason explains it; an intended exception is an `allow`).
- D5. `archgraph check [graph|dir] --update-known shrink|accept`: `shrink` keeps listed entries
  that still occur, never adds, exit 2 without a file; `accept` rewrites to every forbidden import
  now (creates the file). Both print `layout.known.yaml: N kept, N dropped, N added` and then exit as
  `check` would. A saved scan without `--rules` bails "pass --rules". CI never writes the file.
- D6. Pull requests stay judged by `diff` against the base (never reads the file), plus a new
  `--fail-on stale-known` (default on in the action) so a debt fix must shrink the file; pushes use
  `report`'s exit status, which honours the file (fixes the K10 note).
- D7. Claims and progress unchanged: a listed break still counts as broken.
- D8. SARIF keeps known results and adds
  `"suppressions":[{"kind":"external","justification":"Listed in layout.known.yaml."}]` (GitHub
  probably ignores it; removing results would falsely close alerts).
- D9. Additive optional v1.2 fields: `config.rules.known: {path, entries[]}`, `edges[].known` (only
  when true), `summary.known` (only when nonzero), top-level `stale_known[]` (only when non-empty).
  Saved scans carry the list inside `RulesRef`; `--rules` replaces rules and list together; input
  `known` flags are reset and recomputed.

## Phases (one commit each)

### P1. `feat(check): fail only on forbidden imports missing from layout.known.yaml, and on entries that no longer occur`
- `src/known.rs` (new, no trait): `File { path, entries }`, `Entry { rule, from, to }`,
  `load(rules_path) -> Result<Option<File>>`, `mark(edges, file, externals)` (overwrites `known`),
  `stale(doc) -> Vec<Entry>`, `render(doc, Mode) -> String`, `pub fn policy_target`.
- `src/rules.rs` `load` (~234-258) sets `RulesRef.known`; `src/model.rs` `RulesRef.known`,
  `Edge.known`, `Summary.known`, `Document.stale_known`; `src/lib.rs` `known::mark` after
  `decide_with_externals` (~384-391), reset in the no-rules branch, `doc.stale_known` beside
  `doc.target`; `src/simulate.rs` counts `known`; `src/cli/check.rs` `--update-known`, exits become
  `forbidden && !known || !stale.is_empty()` at every exit site, text skips known lines, prints stale
  lines, totals `3 forbidden (2 known), 0 unplanned`, `--brief` gains `stale_known`;
  `src/cli/report.rs` same exit, Rules section `N forbidden (M known)` and stale lines; schema; guide
  `### Known violations` after Claims; CHANGELOG.
- Tests: `tests/rules.rs` `check_fails_only_on_forbidden_imports_missing_from_the_known_file` with
  the ts-hexagonal layout (forbidden: `src/adapters/in/http.ts -> src/adapters/out/pg.ts (forbid[0])`,
  `src/adapters/out/queue.ts -> src/adapters/in/cli.ts (forbid[1])`,
  `src/domain/user.ts -> src/adapters/out/pg.ts (layers)`): two listed → exit 1, stdout exactly the
  forbid[1] line plus `3 forbidden (2 known), 0 unplanned`; all listed → exit 0; an extra entry not
  occurring → exit 1 with the D3 line; a wrong-rule entry → stale plus unlisted; `shrink` rewrites to
  the exact three-line file (header kept), exit 0; `accept` without a file creates the same bytes;
  `shrink` without a file → exit 2; unknown key → exit 2 `unknown field`; `check -o` validates.
  `tests/golang.rs` `report_summary_prints_the_target_block`: `report` exits 0 when every forbidden
  import is listed.

### P2. `feat(diff): fail pull requests on stale known entries and mark known imports suppressed in SARIF`
- `src/main.rs` `FailOn::StaleKnown` (`stale-known`); `src/model.rs` `Diff.stale_known` (new scan);
  `src/diff.rs` markdown `### Known entries no longer forbidden (N)`; `ci/action.sh` and `action.yml`
  default `new-forbidden,new-cycles,stale-known`; `src/sarif.rs` `suppressions` on known pairs;
  guide K10 paragraph and action bullets rewritten.
- Tests: `tests/diff.rs` `--fail-on stale-known` exits 1 with the markdown row; the SARIF block in
  `tests/rules.rs` gets the suppression on the listed result, schema-valid, exit 1 only from
  unlisted ones. Verify `bash ci/test-native-action.sh`, `shellcheck ci/action.sh`.

### P3. `feat(review): show which new forbidden imports are listed as known, and stale known entries`
- `src/diff.rs` review: a known other-forbidden row ends `, listed in layout.known.yaml`, JSON
  `"known": true`; one line `N known entries no longer occur: run archgraph check --update-known
  shrink.`; `src/mcp_tools.rs` `check_diff` gains `stale_known`; `schema/mcp.v1.schema.json`;
  `assets/graph.html` header stat `3 forbidden imports (2 known)` (edges stay red); the review page
  verdict `'3 imports break a rule, 2 of them known.'`.
- Tests: `tests/mcp_tools.rs` review test with a known edge (row text and JSON); `tests/mcp_stdio.rs`;
  `tests/review-ui.test.cjs` verdict.

## Risks

- R1. A debt fix costs one extra command; the failure message names it.
- R2. Moving files with known debt still fails pull request diffs (as today).
- R3. GitHub probably ignores SARIF `suppressions`; known alerts likely stay open.
- R4. The daemon cache must reload when the known file changes (verify; extend
  `warm_custom_rules_reload_instead_of_retaining_old_verdicts` if not).
- R5. Input `known` flags must be overwritten, never merged.
- R6. An older pinned binary rejects `stale-known` in the new default `fail-on` (note in the guide).
