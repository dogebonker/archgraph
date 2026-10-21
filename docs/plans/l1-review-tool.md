# L1: a review tool an AI reviewer calls

Branch `feat/l1-review-tool` from main 1a4ef43. Approved 2026-10-21, including a 4,000-byte MCP
cap for `review`. Context: L1 in [pr-architecture-review.md](../pr-architecture-review.md).

## Decisions

- D1. `review` is a new MCP tool and CLI subcommand, not a `check_diff` mode: `check_diff`
  serves the edit loop (findings with `ag-` IDs, snapshot mode for hooks, trimming that keeps
  `findings` last, `src/cli/mcp.rs:482-499`). Both are thin entries over the existing base
  machinery (`src/cli/mcp.rs:46-99`), `diff::diff` and `target::delta`; `src/cli/mcp.rs:54`
  becomes `matches!(args.tool.as_str(), "check_diff" | "review")`, snapshot stays `check_diff`
  only, and the CLI calls `mcp_cli::call` with `tool: "review"`.
- D2. Line evidence: private `line(doc, from, to) -> Option<usize>` in `src/target.rs`: first
  `Edge.line` (`src/model.rs:389`) over `edges` and `external_edges`; else the smallest
  `ImportRecord.line` (`src/model.rs:304-322`) with that `from` and `to` in `targets`. Absent for
  depcruise and line-less recordings: rows print without `:N`, JSON omits `line`, one footer
  `Lines unknown for N imports: the <collector> collector records none.`
- D3. `Delta` gains `newly_fixed` (unreleased, so the shape may change):
  `Delta { conforming, moved, newly_broken: Vec<SliceChange>, newly_fixed: Vec<SliceChange> }`,
  `SliceChange { claim, reason, source?, slice, imports: Vec<Evidence> }`,
  `Evidence { from, to, line? }`, `delta(old: &Document, new: &Document) -> Option<Delta>` (the
  None/None match moves from `src/diff.rs:85-94` into it); one helper builds both lists. A slice
  broken before and now unbound or gone counts as fixed (schema description says so).
- D4. Output: headline, Target line, `### Newly broken (n)`, `### Other new forbidden imports (n)`
  (`Diff.new_forbidden` minus pairs already under newly broken, with `rules.doc.reasons/sources`),
  `### Newly fixed (n)`; rows grouped under a claim line with reason and source; markdown budget
  `REVIEW_BYTES = 1300`, true totals always shown, footer
  ``N more rows in `archgraph review --format json`.``; CLI and MCP give identical bytes. MCP:
  own cap 4000 (`src/cli/mcp.rs:398`, like `explain` 300), text content = the markdown, brief
  drops the JSON arrays (counted as omitted). JSON key `other_forbidden` (not `new_forbidden`,
  which `brief_payload` re-projects). With no rules: `No rules: nothing was checked; add
  layout.yaml or pass --rules.` A `Base <sha12> (<selection>)` line, plus `; rules changed since
  base: both commits are judged by the current rules` when the policy changed.
- D5. Reviewer files live in `integrations/`; neither `init` nor `sync-rules` writes them.

## Phases (one commit each)

### P1. `feat(diff): list newly fixed slices with each import's line and the claim's reason`
- `src/target.rs` types and `delta`/`changed`/`line` (a `ponytail:` note on the O(links×edges)
  scan); `src/diff.rs:85-94` calls `delta(old, new)`; `src/model.rs` doc comment; schema
  `schema/archgraph.v1.2.schema.json` (~3634-3700: `newly_fixed` required, items with
  `reason`/`source`, imports `{from, to, line?}`); CHANGELOG.
- Tests: `tests/diff.rs` (~490) exact `newly_broken` with claim, reason, slice, imports and
  `newly_fixed == []`, and the reverse diff; `tests/mcp_tools.rs` helper `claims(edges)` (files
  `src/m/{a,b}/app/{x,y}.ts`, `src/m/{a,b}/domain/{d,e}.ts`, `src/shared/id.ts`; rules
  `{id: app-no-domain, from: "src/m/<name>/app/**", to: "src/m/<name>/domain/**", reason: "apps
  never reach a domain", source: "docs/adr/0001.md"}` and `{from: "src/shared/**", to: "src/m/**",
  reason: "shared stays a leaf"}`; old `[(x→d, line 2)]`, new `[(y→e, line 3), (id→d, line 1)]`):
  broken b line 3, fixed a line 2, conforming `[{1,2},{1,2}]`. One Go case from
  `tests/golang.rs` fixtures pins the record-line fallback (R1).

### P2. `feat(review): add archgraph review and an MCP review tool for AI reviewers`
- `src/diff.rs`: extract `progress(t)` from the Target line code; `pub fn review(old, new) ->
  Value` returning `{markdown, progress, newly_broken, newly_fixed, other_forbidden}`.
- `src/mcp_tools.rs`: `ReviewArgs {base, brief}`, validate arm (base non-empty), call arm,
  schema entry. `src/cli/mcp.rs`: the `matches!`, Base line, text content = markdown, brief drop,
  cap 4000, `Review` CLI args (`--base`, `--format markdown|json`, default markdown) and `fn
  review()`. `src/main.rs`: subcommand "What a pull request changed in the target, for an AI
  reviewer to quote", exit 0 on success. `schema/mcp.v1.schema.json` review input/output.
  `docs/guide.md` MCP table row, CHANGELOG.
- Tests: `tests/mcp_tools.rs` exact markdown for the P1 fixture:
  ```
  **archgraph review**: 1 newly broken slice, 1 other new forbidden import, 1 newly fixed slice.
  Target: 1 of 2 slices conform.

  ### Newly broken (1)

  - **app-no-domain**: apps never reach a domain (docs/adr/0001.md)
    - slice b: src/m/b/app/y.ts:3 → src/m/b/domain/e.ts

  ### Other new forbidden imports (1)

  - src/shared/id.ts:1 → src/m/a/domain/d.ts, forbid\[1\]: shared stays a leaf

  ### Newly fixed (1)

  - **app-no-domain**: apps never reach a domain (docs/adr/0001.md)
    - slice a: src/m/a/app/x.ts:2 → src/m/a/domain/d.ts
  ```
  then `validate("review", &out)`; a budget test (40 slices broken, no lines, fixed-width rows,
  `md.len() <= 1300`, `### Newly broken (40)`, exact footer and lines-unknown line); add review to
  the examples list (~242); `tests/mcp_stdio.rs` tool list, 4000 limit, a `review` call with
  `base: "HEAD"` whose text equals `data.markdown` and shows `Base <sha12>`; a `src/cli/mcp.rs`
  unit test that `bounded("review", …)` stays ≤ 4000 with the markdown intact.

### P3. `docs(review): ship an Amp check and a Claude Code reviewer that quote archgraph review`
- `integrations/amp-review.check.md` (copy to `.agents/checks/archgraph.md`; frontmatter `name`,
  `description`) and `integrations/claude-review.agent.md` (copy to
  `.claude/agents/archgraph-reviewer.md`; `name: archgraph-reviewer`, description "Reviews a
  branch's architecture against layout.yaml with archgraph; use when reviewing a pull request";
  no `tools`, so MCP tools are inherited). Body: run `archgraph review` (or MCP `review`), quote
  it verbatim; report every Newly broken and Other new forbidden row at file:line with its
  reason, Newly fixed briefly; do not re-judge import direction, layering or forbidden imports;
  repeat a HEAD-fallback or rules-changed Base note; judge only what archgraph does not check
  (validation placement, SQL in handlers, duplicated helpers, naming); if it fails or is missing,
  say the architecture check did not run, never that it passed.
- `skills/archgraph/SKILL.md:19` one clause; `docs/guide.md` section "Reviewing a pull request
  with an AI reviewer" with the two copy commands.

## Risks

- R1. Go reference edges have no `Edge.line`; the record fallback is unverified (pin a case).
- R2. Both commits are judged by HEAD's rules, so loosening a rule reads as "fixed"; the Base
  line discloses a rules change.
- R3. Shallow CI clones fall back to HEAD (empty review); the Base line shows it and the check
  tells the reviewer to repeat it.
- R4. An extra break in an already-broken slice decided by another rule shows that rule's
  reason; one decided by `allow` does not show.
- R5. A single path longer than the budget would overflow it; accepted.
