# 4. Move cost comes from simulating the whole target

Status: accepted (2026-10-20)

## Context

Moving `handler.go`, `service.go` and `repository.go` into their own Go packages leaves
`routes.go` and `types.go` behind, and in 8 GoldenGate slices the repository imports the
service. Counting files, import lines and names alone would price moves that cannot compile.

## Decision

When moves are pending, analysis applies every pending move once through `simulate::apply` and
takes `graph::new_cyclic_import_pairs`, which judges Go cycles by package. Each import in a new
cycle is listed under every slice whose source or destination folder holds either file. A move
that cannot be applied leaves a note, and the other cost fields still report.

## Consequences

The cost reuses the existing simulation instead of a second model. A cycle that appears only
because two slices move together is listed under both. Analysis does one extra build while
moves are pending, about 11 ms on GoldenGate. Other languages report no new cycles.
