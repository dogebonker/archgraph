# Edge quality pass

Approved sequence (five separate buildable conventional commits):

1. Canonical kinds, weighted metrics, kind-filtered rules, deferred cycle accounting, v1 migration.
2. Named barrel resolution, depth cap and re-export-loop evidence.
3. Default exclusions/ignore files, included test classification and coverage summary.
4. Import-statement accounting, common resolution gaps, external nodes and unresolved command.
5. Grab-bag split verification, granularity, complete showcase report and generated README table.

Baseline: `6627a5e22013044516a599e86bede437ba76ad10`. All manifest revisions and scan flags stay pinned. The published twenty repositories are the main comparison; the three Go manifest entries are supplemental. Baseline and stage binaries are stored separately with SHA-256 provenance; no changed source is attributed to an earlier commit.

Compatibility retains integer counts and adds weighted measures. New readers accept v1 and v1.1; `--legacy-v1` exports the old discriminator and kind names for strict old readers. It does not reverse the improved analysis. Runtime/lazy/type/structural-reexport weights default to 1/0.3/0.1/0.

Validation uses targeted fixtures, full tests, Clippy, formatting, workflow checks and exact pinned showcase runs at each stage. The final report distinguishes disappearing, shrinking, splitting and merging SCCs, permits multiple evidenced causes, and includes ten manually checked file:line witnesses. README figures are generated from those results. Signing uses the existing Git identity through Bitwarden.
