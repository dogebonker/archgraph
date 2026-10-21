# Review page: findings first, map second

Branch `feat/review-page`. Approved 2026-10-21: the report opens on a review page when the scan has
a target (claims or moves); without one it opens on the map as today. The map stays one click
away, and every row on the review page opens the map focused on that row's files and imports.
Afterwards the L-plan in [pr-architecture-review.md](../pr-architecture-review.md) goes one item
at a time.

## Layout (approved sketch)

```
┌ archgraph · goldengate/apps/api · 502938c ─── [Review] [Map] ┐
│ 16/18 slices conform · 0/23 moved · 2 forbidden · 0 cycles  │
├──────────────────────────────────────────────────────────────┤
│ Claims                      layered        service-no-sql    │
│ "Dependencies point inward"  slide 8        slide 12         │
│ apikey                        ✓              ✗ 1             │
│ interview                     ✗ 1            ✗ 1             │
│ process_map                   ·              ✗ 1             │
│ …                          (· = rule can't fail here)       │
├──────────────────────────────────────────────────────────────┤
│ Move cost        files  imports  names  new cycles  moved    │
│ sop                11      42     227      10        no      │
├──────────────────────────────────────────────────────────────┤
│ Findings  · What hurts · What to do next  (from Assessment)  │
└──────────────────────────────────────────────────────────────┘
```

## Constraints

- One self-contained `assets/graph.html`; no libraries; existing tokens (`--fill1`, `--label2`,
  `--red-s`, `--green-s`, glass) and type scale; dark and light as today; usable at 1280 px and
  keyboard-navigable (the matrix is a real `<table>` with headers, cells are buttons).
- Repository text only through `el()` / `textContent`.
- Reuse, do not duplicate: verdicts and slices from `doc.target`, the target focus selection
  (`sliceSel`, `moveSel`, `targetMoves`) for map focus, and the Assessment's builders for
  findings, so the review page and the side sheet never disagree.
- What-if stays a map feature.

## Phases (one commit each)

- R1. View switch and verdict strip: `[Review] [Map]` in the header, Review default when
  `doc.target` exists, state in the URL hash (`#review`, `#map`) so a link opens the same view;
  verdict strip (slices conform, slices moved, forbidden imports, cycles).
- R2. Claims matrix: rows = slices (union over claims, `*` for uncaptured), columns = claims with
  id, reason and source; cells ✓ (holds), ✗ N (broken, N imports), · (not bound); rows sorted
  broken first; a cell opens the map focused like a Target-panel slice row.
- R3. Move cost table: slice, files, import lines, names to export, imports in new package cycles,
  moved; sortable by column; a row opens the map with that slice's destination discs.
- R4. Findings: the Assessment's "What hurts" and "What to do next" items on the page, each
  opening the map as the side sheet does.

Each phase: Node UI test in a new `tests/review-ui.test.cjs` (marker-cut like the others, listed in
`.github/workflows/ci.yml`), full `node --test ../tests/*.test.cjs`, `cargo test --locked`, and a
browser check on the GoldenGate report.
