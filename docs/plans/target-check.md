# Target check and migration progress

Brief from the 2026-10-20 discovery session. First customer: the GoldenGate backend
(`apps/api`, Go, 432 files) and its proposal deck "A New Architecture" (July 2026).

## Outcome

A team states a target architecture. Archgraph says, against today's code, which parts of
the target already hold, where each part breaks, what moving to the target costs, and how far
the migration has come. CI keeps the parts that hold from regressing and reports progress on
every pull request. Nothing in it is specific to one company or language.

## Terms

- **Target**: the `layout.yaml` a team writes for where it wants to be. Its rules use today's
  paths and file roles (`internal/*/handler*.go`), so they are enforceable before any file moves.
  An optional `moves` section maps today's paths to target paths.
- **Claim**: a rule with an `id` and a `reason` that quotes or cites the proposal
  ("Dependencies point inward", slide 8). The proposal check reports one verdict per claim.
- **Verdict**: `holds`, `holds in N of M slices`, `broken` (with the offending imports), or
  `vacuous` (its globs match no file, so it protects nothing).
- **Slice**: one value of a rule's `<name>` capture (`internal/<name>/**` gives `project`,
  `auth`, ...). Progress counts slices. A rule without a capture has a single verdict.
- **Move**: `{ from: "internal/<name>/handler*.go", to: "internal/app/<name>/handler/" }`.
  A slice is moved when no file still matches `from`.
- **Move cost**, per slice: files moved, import lines to rewrite, and, for Go, the unexported
  names that must be exported or moved together, with their using and declaring files.
- **Progress**: slices that conform to every claim bound to them, and slices moved, each as
  N of M, with the change against the base commit in CI.

## How a target is written

1. Hand-written `layout.yaml`, using the existing rule syntax plus `moves`.
2. Drafted from a proposal document (deck, markdown, ADR) by the user's agent, following the
   archgraph skill or MCP tool. Archgraph then checks the draft: every glob matches files,
   every slice resolves, every claim gets a verdict. Archgraph does not parse documents or call
   a model.

## Where results appear

- HTML report: a Target panel with each claim, its reason and verdict, slice progress and
  move cost. Selecting a row shows its files on the map.
- `archgraph check`: claim verdicts and progress in the text output and in the JSON, for agents
  and MCP.
- GitHub Action job summary: progress and its change in this pull request, plus newly broken
  claims. The existing `fail-on` ratchet (new forbidden imports or cycles against the base
  commit) stays the gate.

## Non-goals

- Parsing pptx or markdown in archgraph, or calling an LLM.
- Checking code patterns that are not dependencies: validation tags, raw SQL in tests,
  duplicated helpers. The agent reports these as outside what archgraph checks.
- Executing moves. `apply` moves single files today; package splits stay a later step.
- Names to export for Java (package-private). Java reports files and imports until its
  collector records names.

## Acceptance on GoldenGate

- A slice counts toward a claim only when one of the claim's rules has files on both sides in
  that slice, so "holds" can always fail. The claim {handler ↛ repository, service ↛ handler,
  repository ↛ handler} over today's paths holds in 16 of 18 slices, broken in `interview` and
  `sop`, with the exact imports.
- Moves from `internal/<name>/{handler,service,repository}*.go` to
  `internal/app/<name>/{handler,service,repository}/` give a cost per slice that lists the
  unexported names to export and the package cycles the moves would create, and 0 of 23
  slices moved (every slice with a file matching a move's `from` counts).
- A claim whose globs match nothing is reported `vacuous`, not `holds`.
- A pull request that adds a handler → repository import shows the claim's slice count drop
  and fails through the existing `fail-on new-forbidden`.
