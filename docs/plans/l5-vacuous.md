# L5: fail a pull request that makes a claim protect nothing

Branch `feat/l5-vacuous` from main 03671bb. Approved 2026-10-21: `new-vacuous` on by default in
the Action for pull requests, listed in `review`; the Action's push path and `every-file-in-layer`
left for later. Context: L5 in [pr-architecture-review.md](../pr-architecture-review.md).

## Findings

- ArchUnit fails a rule whose should-part meets no class by default (user guide §10.4,
  `allowEmptyShould`, `archRule.failOnEmptyShould`), on the current code only. Deptrac's
  `--fail-on-uncovered` is opt-in and concerns dependencies into no layer; `debug:unassigned` and
  `debug:unused` are debug commands.
- Archgraph already computes claim vacuity (`src/target.rs:203-208`) and notes rule sides that
  match no file (`:168-176`), shown in `check`, `report`, `doc.target`, MCP `target`; not in `diff`,
  `review` or SARIF. No exit code reacts to it.
- "Uncovered" already exists: unplanned imports (`src/rules.rs:1105-1110,1161-1165`),
  `default: forbidden`, `fail-on: new-unplanned`, and the `every-file-in-layer` early smell.
- Vacuous by design is normal (target claims about folders that do not exist yet, preset drafts),
  so only a delta against the base is safe as a default; pushes and `check` stay unchanged.

## Decisions

- `diff --fail-on new-vacuous`: fails when a claim is vacuous in the head scan and present and not
  vacuous in the base (both judged by the head's rules in the Action). A claim missing from the
  base never counts.
- `target::Delta.newly_vacuous: Vec<VacuousClaim { claim, reason, source?, notes }>` (skipped when
  empty); schema optional field.
- Markdown: the Target line gains `, N newly vacuous claim(s)`; a `### Newly vacuous claims (N)`
  section with `- {id}: {notes joined "; "}` (or `- {id}`).
- `review` lists newly vacuous claims, one line each, inside the byte budget.
- Not in L5: per-rule vacuity inside a claim, non-claim rules matching nothing, typos in new claims
  (the base is judged by the head's rules), the Action push path running early smells.

## Phases (one commit each)

### P1. `feat(diff): fail pull requests on a claim the change makes vacuous`
- `src/target.rs` struct, `Delta` field, `delta` logic; `src/main.rs` `FailOn` value
  `new-vacuous`, its arm, expected-values text; `src/diff.rs` Target line, section, `review` section;
  schema; CHANGELOG.
- Tests (`tests/diff.rs`, `target_scan`): moving both apps out (`("src/m/a/app/x.ts",
  "src/app/a/x.ts")`, `("src/m/b/app/y.ts", "src/app/b/y.ts")`) gives exit 1 with
  `**archgraph diff: fail** (new-vacuous): `, the Target line ending `1 newly vacuous claim.`, the
  section row ``- app-no-domain: forbid[0] `from` matches no file``, the JSON `newly_vacuous`
  literal and schema validity; the reverse and `old old` exit 0 without the key; a partial move
  does not mention vacuous; review shows the line (`tests/mcp_tools.rs` review fixture). Confirm
  the literals on the first run.

### P2. `feat(action): fail pull requests that make a claim vacuous by default`
- `action.yml` and `ci/action.sh` default `new-forbidden,new-cycles,stale-known,new-vacuous`;
  guide: fail-on values, the Action defaults (a pull request emptying a claim updates the claim in
  the same change; older pinned versions reject the value), the Claims exit sentence; CHANGELOG.
- Verify `shellcheck ci/action.sh`, `bash ci/test-elixir-action.sh`, `bash ci/test-native-action.sh`.

## Risks

- R1. An older pinned `version` rejects `new-vacuous` in the default (as `stale-known`).
- R2. Finishing a migration fails when claims still name the old paths: rewrite the claim, or use
  the brace form over both locations.
- R3. Partial loss (one slice or one rule of a claim) is not gated; the Target line shows it.
- R4. A typo in a new claim never fires.
