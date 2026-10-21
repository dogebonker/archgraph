# L7: propose claims from a direction that holds across sibling slices

Branch `feat/l7-candidates` from main f407c1d. Approved 2026-10-21: flags `--min-slices` (default
5) and `--min-share` (default 0.8), a stable `reason` with the counts in comments, one claim per
role pair, skip-layer proposals left for later. Context: L7 in
[pr-architecture-review.md](../pr-architecture-review.md). Qodo's Rule Miner and Greptile's
suggested rules mine past review comments into prose rules a model enforces
([Qodo](https://docs.qodo.ai/governance/rule-enforcement/rule-miner),
[Greptile](https://www.greptile.com/docs/code-review/custom-standards)); this mines today's import
graph into exact, checkable glob claims.

## Findings

- `infer-layout` reasons about whole modules (`src/infer_layout.rs` keys by `file.module`): on
  GoldenGate it drafts 5 layers and 20 sparse sibling bans, no per-slice claim.
- GoldenGate role matrix (`internal/<name>/`, roles cut at the first `.` or `_`): handler imports
  service in 18 slices, service imports handler in 0; repository imports service in 8 of 16 (Go
  repositories use types declared in `service.go`); handler→repository 2, repository→handler 0. So
  the deck's `handler ↛ repository` is intent, not an observed direction; on its own it holds in 14
  of 16 slices.
- Role globs `internal/<name>/service{/**,.*,_*}` work through `check` today, and
  `Compiled::forbid_slices` (`src/rules.rs`) is what `target::analyze` uses: counts taken from it
  cannot disagree with `check`.

## Decisions

- D1. Extend `infer-layout` (its output is a reviewable, never-applied YAML draft; `init` merges it).
- D2. Roles: for every directory `root` and child folder `name`, the role is the next path segment,
  a file cut at its first `.` or `_` (`handler_documents.go` → `handler`), a folder kept
  (`ui/` → `ui`); glob `<root>/<name>/<role>{/**,.*,_*}`. A family needs at least `--min-slices`
  names; propose `B ↛ A` when A imports B in at least `--min-slices` slices, `B ↛ A` binds at least
  `--min-slices` and holds in at least `--min-share` of them. No skip-layer rules (intent, not
  evidence).
- D3. Output: claims first in `forbid:`, sharing `--max-rules` with the sparse forbids (as external
  proposals do); ordered by bound slices descending, then share, then id; each with comments
  ```yaml
    # internal/<name>: handler imports service in 18 of 18 slices; service imports handler in 0 of 18 (holds in 18).
    - id: infer-claim-1
      from: "internal/<name>/service{/**,.*,_*}"
      to: "internal/<name>/handler{/**,.*,_*}"
      reason: "service does not import handler (drafted from the graph; review intent)"
  ```
  a broken claim also lists `# Breaks in discovery, miner:` and each hit
  `#   "internal/discovery/service.go" -> "internal/discovery/routes.go"`; a header
  `# N eligible slice claims; M omitted by --max-rules.` Root and role escaped with `literal()`.
  One claim per pair; teams merge under one id by hand.

## Phases (one commit each)

### P1. `feat(infer): propose claims from a direction that holds across sibling slices`
- `src/rules.rs` `SliceEvidence::bound()`, used by `src/target.rs`; `src/infer_layout.rs`
  `slice_claims` (one compile, one `forbid_slices` call; `ponytail:` note on pairs × slices ×
  files), options for the two thresholds; `src/cli/infer_layout.rs` flags; guide "Layout inference"
  paragraph; CHANGELOG.
- Tests: unit test in `src/infer_layout.rs` (slices `internal/{a,b,c,d,e}/{handler,service,
  repository}.go`, `handler.go → service.go` in all five plus `e/service.go → e/handler.go`): exactly
  one claim `from internal/<name>/service{/**,.*,_*}` `to internal/<name>/handler{/**,.*,_*}`, the
  comment counts `handler imports service in 5 of 5 slices; service imports handler in 1 of 5
  (holds in 4)` (confirm exact text), the breaking hit comment, no handler/repository claim; adding
  `d/service → d/handler` (3 of 5, 0.6) gives none; four slices give none; `--min-slices 4` with four
  slices gives one. Integration test in `tests/infer_layout.rs`: `infer-layout` then `check --rules
  draft` exits 1 with `claim infer-claim-1: holds in 4 of 5 slices; broken in e reason="service does
  not import handler (drafted from the graph; review intent)"`. Existing fixtures (at most 4
  siblings) unchanged.
- Verify on GoldenGate: the 9 claims (`service ↛ handler` 18/18, `handler ↛ routes` 18/18,
  `repository ↛ routes` 16/16, `service ↛ routes` 16/18 broken in discovery and miner,
  `types ↛ handler` 18/18, `types ↛ service` 19/19, `types ↛ repository` 20/20, `config ↛ client`
  10/10, `types ↛ client` 8/8 in `internal/platform`), confirmed by `check`.

### P2. `docs: point target drafting at claims infer-layout proposes`
- `skills/archgraph-rules/SKILL.md`: `infer-claim-*` as a starting target without a deck; they are
  observed directions; add intent rules (such as handler ↛ repository) by hand and merge claims
  under one `id`. `docs/pr-architecture-review.md` L7 marked done ("from the graph", not "from
  history").

## Risks

- R1. A drafted claim's counts go stale; they live in comments, the reason stays true as intent.
- R2. `init` merges the claims; broken slices show as its existing `CONFLICT` lines.
- R3. Role detection and the glob can disagree on odd names; counts stay exact (from the glob).
- R4. Cost grows with families × role pairs; bounded by the slice prefilter.
- R5. Test files count, as in `check` (sparse inference ignores them).
