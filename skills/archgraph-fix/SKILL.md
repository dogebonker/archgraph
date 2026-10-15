---
name: archgraph-fix
description: Fixes one archgraph finding end to end, keeping the change only when a re-scan and the tests prove it. Use when asked to fix, apply or act on an archgraph finding, break an import cycle, move a misplaced file, or route an import through a module's public entry.
license: MIT OR Apache-2.0
compatibility: Needs git, the archgraph binary (setup in the archgraph skill), and the project's own type checker and test command.
---

# archgraph-fix

One run fixes one finding, as one reviewable change. Every step ends on evidence; the change survives only if the last step is green.

## Steps

1. **Pick one finding.** Scan the repository (Setup and flags in [the archgraph skill](../archgraph/SKILL.md)) into a temporary directory. Take the finding the user named; otherwise take the first that exists, in this order:
   1. a forbidden import (`summary.forbidden` > 0; `archgraph check` lists them),
   2. a two-way pair in a module cycle whose lighter side is a single import (`cycles[].two_way`, `imports: 1`),
   3. a `misplaced-file` finding with a `suggestion`,
   4. an `internals-reach` finding,
   5. a `sink-split` component whose `moves` are filled.
   
   State it to the user with its evidence (the JSON fields, quoted). When the scan carries a `small-repo` finding, say the signal is weak before proceeding. Done when one finding is chosen and stated.
2. **Check the ground.** The git working tree must be clean (ask the user to commit or stash otherwise). Find the type check and test commands in the project (`package.json` scripts, `pyproject.toml`, `Makefile`, CI config) and run both; run Python with `PYTHONDONTWRITEBYTECODE=1` so the run leaves no `__pycache__` behind, and remove any other files the run created. Done when both pass and `git status` is clean again. If they fail before any change, stop: a red baseline cannot prove the fix safe.
3. **Plan and simulate.**
   - Move (misplaced-file, sink-split): run `archgraph simulate <graph.json> --move <file> <dest>`. Proceed only when `new_forbidden` is empty, the cycle count does not grow, and the two modules' combined cohesion rises. Quote before and after.
   - Cut (two-way pair, forbidden import, internals reach): decide the code change that removes the import, in this order of preference: import the module's public entry; move the used symbol to the module that should own it, re-exporting it from its old place when other code imports it there; take the needed value as a parameter, or depend on an interface the lower module owns and inject the implementation from the composition root. Prefer the smallest change that keeps existing public names importable. The prediction for a cut is the finding itself: the import named in `break` or `two_way.edges` disappears and its cycle with it. When the cut moves a file, simulate that move as above.
   
   Done when the plan names every file to touch and, for moves, the simulation passes. When the user did not name the finding, present the plan with its evidence and wait for their approval before applying it.
4. **Apply.**
   - Move with `git mv`, then rewrite every import of the moved file: relative specifiers from every importer, path aliases (`@/…`, tsconfig `paths`), barrel re-exports (`index.ts`, `__init__.py`), Python dotted imports, mocks and test files. Also update the moved file's own relative imports. Search for the old path, the old module name and the file name to find them all.
   - Cut: make the planned edit and nothing else.
   
   Done when a search for the old path or symbol finds no stale reference.
5. **Verify.** Run the type check and the tests; then scan again and run `archgraph diff <before.json> <after.json> --fail-on new-forbidden,new-cycles,new-unplanned --format markdown` (exit 0 passes). Compare the after-scan with the prediction from step 3: the simulation for a move, the removed import for a cut. Done when all hold: type check and tests pass, the diff exits 0, the chosen finding is gone (or the cycle lost its pair), and the after numbers match the simulation.
6. **Keep or revert.** All green: leave the change staged for review (commit only when the user asked for commits), with a proposed commit message naming the finding and quoting before and after numbers. Anything red: revert completely from the repository root (`git restore --source=HEAD --staged --worktree -- <the files the plan named>`, and `git clean -f -- <paths this run created>`), then report what failed and why. Done when the tree holds either the verified change or no change.

## Scope

- One finding per run. When the user asks for several, repeat the whole loop per finding and re-scan between them: moves interact.
- Code changes stay within the plan from step 3. Leave unrelated cleanup, renames and formatting out of the change.
- `layout.yaml` is policy: a fix never edits it. If the right fix is a rule change, say so and stop.
