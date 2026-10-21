# L2: findings on the changed line through SARIF

Branch `feat/l2-sarif` from main 5c048ab. Approved 2026-10-21: vendor the OASIS schema for tests,
no cycles yet (P3 deferred), own fingerprint, upload documented rather than built in. Context: L2
in [pr-architecture-review.md](../pr-architecture-review.md).

## Primary-source facts

- GitHub requires `$schema`, `version` "2.1.0", `runs[]`, `tool.driver.name`, `rules[]` with
  `id`, `message.text`, `locations[]`, `partialFingerprints`, `artifactLocation.uri`,
  `region.startLine`; no empty strings for required properties
  (docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/sarif-support-for-code-scanning).
  Spec: OASIS SARIF 2.1.0 errata01 (§3.13, §3.14.6, §3.19.8, §3.27, §3.29 location without region
  = whole artifact, §3.30 lines from 1).
- Code scanning uses only `primaryLocationLineHash`; upload-sarif computes it when missing (a
  rolling hash of 100 non-whitespace characters from the line, so it spans following lines) and
  keeps a provided value with a warning when they differ (codeql-action `src/fingerprints.ts`).
- New versus existing: GitHub compares the head analysis with the target branch's; "new alerts on
  lines of code changed in the pull request are shown as annotations"; without a base analysis it
  shows "Missing analysis".
- Limits: 10 MB gzip per file, 20 runs, 25,000 results per run (top 5,000 kept), 25,000 rules.
- upload-sarif inputs: `sarif_file`, `checkout_path`, `category` (replaces the job/matrix id),
  `wait-for-processing`, `token`, `ref`, `sha`. Permissions: `security-events: write` (`actions:
  read` in private repositories; Code Security enabled there). `pull_request` from forks and
  Dependabot may upload.

## Decisions

- D1. One source: `archgraph check <doc|dir> --format sarif`, the current state, uploaded on pushes
  to the default branch and on pull requests; GitHub splits new from existing. No `diff`/`review`
  SARIF.
- D2. A finding with no line gets no `region` (whole file); its message ends
  "Line unknown: the <collector> collector records none." Never a fake `startLine: 1`.
- D3. `ruleId` = `Edge.rule` for forbidden imports, the claim id for broken claim slices; `rules[]`
  only ids in use, sorted, each with `shortDescription.text` (reasons joined "; ", else
  "<id> forbids this import"), `help.text` "Source: <sources>" when present,
  `defaultConfiguration.level: "error"`, `properties.tags: ["architecture"]`; every result
  `level: "error"`; `partialFingerprints.primaryLocationLineHash` = first 32 hex of
  sha256("{ruleId}\0{from}\0{to}"), no line, so alerts survive line moves and neighbouring edits.
- D4. The Action writes `archgraph.sarif` when rules exist and exposes output `sarif`; the guide
  documents the upload-sarif step with `if: ${{ !cancelled() && steps.archgraph.outputs.sarif }}`,
  no `category`, push + pull_request, never `pull_request_target`.
- D5. New module `src/sarif.rs`, no trait or format abstraction.
- Results: one per link in `doc.target.claims[].slices[].broken` at `link.from` with
  `target::line`; one per forbidden edge in `edges` and `external_edges` whose (from, to) is not
  already under a broken claim (mirrors `src/diff.rs` review de-duplication). Unplanned imports and
  early smells out of scope.
- Paths: `uri` = `scan_root/path` when `source.scan_root` is set and not `.`, else the path;
  percent-encode every byte outside `A-Za-z0-9-._~/`; `uriBaseId: "%SRCROOT%"`.
- Messages, decisive sentence first: claim
  "{from} imports {to}, breaking claim {id} in slice {s}: {reason} ({source})." (no "in slice" for
  `*`, no parentheses without source); forbidden "{from} imports {to}, forbidden by {rule}:
  {reasons}." (sources in parentheses; "forbidden by {rule}." without reasons).

## Phases

### P1. `feat(check): write SARIF 2.1.0 for GitHub code scanning with --format sarif`
- `src/sarif.rs` (`pub fn sarif(doc: &Document) -> Value`, `$schema`
  https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json,
  driver name/version/informationUri from Cargo env), unit test
  `uri("src/a b#1.ts") == "src/a%20b%231.ts"`; `src/lib.rs` `pub mod sarif;`; `src/target.rs`
  `line` becomes `pub(crate)`; `src/cli/check.rs` local `enum CheckFormat { Text, Sarif }` and
  `--format` (not the shared `Format`), SARIF branch before sync-rules/early smells: `-o` still
  writes the document, print pretty SARIF, exit 1 if any edge is forbidden else 0;
  `docs/guide.md` Commands; CHANGELOG.
- Tests: vendored `tests/fixtures/sarif-schema-2.1.0.json` (source URL and sha256 noted in the
  test, line in THIRD_PARTY_NOTICES.md), validated with `jsonschema::validator_for` as in
  `tests/mcp_tools.rs`; `tests/mcp_tools.rs` `sarif_puts_each_broken_claim_and_other_forbidden_import_on_its_line`
  with the `claims()` fixture (new = y→e line 3, id→d line 1): exact results (claim result with
  fingerprint `bef3e88717e6b19c6929f9300f74b5d5`, forbid[1] result with
  `74053824541e62736c72f6c88aeef584`; recompute and explain if they differ), rules descriptors,
  a line-less + `scan_root = "web"` case (uri `web/...`, no region, "Line unknown…" ending);
  `tests/rules.rs` ts-fda check test extended with `--format sarif`: exit 1, the (ruleId, uri)
  pairs in claim-then-forbidden order, regions present (the recorded scan carries lines 1,1,1,1,3), uris under `tests/fixtures/ts-fda/`, schema-valid.

### P2. `feat(action): write archgraph.sarif and document uploading it to code scanning`
- `ci/action.sh` after the outputs block, before the base-ref branch: when rules exist, run
  `check head/archgraph.json --format sarif > head/archgraph.sarif`, accept exit ≤ 1, write
  `sarif=<path>` to `GITHUB_OUTPUT`. `action.yml` output `sarif`. `docs/guide.md` CI section: a
  workflow snippet (push to main + pull_request; permissions `contents: read`,
  `security-events: write`, `actions: read` for private repos; upload-sarif pinned by SHA with the
  D4 `if:` and no category) with notes on Code Security, forks, `checkout_path`, the expected
  fingerprint warnings and giving forbid rules an `id`. `.github/workflows/ci.yml` `action-push`:
  `jq -e '.runs[0].results | length == 5'` on the output (ts-fda: 4 claim results, 1 `public`).
- Verify: `shellcheck ci/action.sh`, `bash ci/test-native-action.sh`, a local action run printing
  5 results and exiting 1.
- Manual, needs a sandbox public repository: push to main, then a pull request adding one
  forbidden import beside an existing one: one annotation on the new line, the existing alert not
  new, the check failing on 1 new alert, line-less results listed but not annotated.

### P3 (deferred). Cyclic import pairs as results, ruleId `cycle`.

## Risks

- R1. upload-sarif warns once per result with a line about the provided fingerprint (accepted).
- R2. GitHub's table also lists startColumn/endLine/endColumn; if the sandbox shows problems, add
  `startColumn: 1`, `endLine = startLine`.
- R3. Results without a line are listed but never annotated inline.
- R4. Unnamed rules are numbered `forbid[i]`; inserting one renumbers and re-opens alerts: give
  rules an `id`.
- R5. A pull request that loosens a rule shows its alerts fixed (each commit judged by its rules).
- R6. A pair under a broken claim but decided by another rule appears once, under the claim.
- R7. No result cap; add `.take(25_000)` if a repository needs it.
- R8. `level: error` makes code scanning a second gate beside `fail-on`.
- R9. Paths outside the scan root (`../x`) are not normalised.
- R10. `--brief --format sarif` is not handled.
