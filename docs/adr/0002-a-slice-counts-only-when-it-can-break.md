# 2. A slice counts only when a rule could fail in it

Status: accepted (2026-10-20)

## Context

Progress is counted per slice, one value of a rule's `<name>` capture. On the GoldenGate
backend, counting every slice where the `from` side matches would report slices such as
`process_map` as holding the claim "handlers never import repositories" when they have no
repository to import, and the total would move with whichever rules a team happened to write.

## Decision

A slice is bound to a claim only when one of the claim's rules has a file on the `from` side and
a target on the `to` side in that slice. A claim with no bound slice is `vacuous`, and each rule
side that matches no file adds a note.

## Consequences

"Holds" can always fail, and globs that protect nothing are reported. Denominators are smaller:
GoldenGate reads 16 of 18 slices, not 23. Changing the rule later changes every progress number
teams have tracked.
