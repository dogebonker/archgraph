# Architecture contracts, project skills, and visual change review

Status: W1a implemented and independently reviewed. The remaining outcomes are a product-direction
map, not implemented features or an approved storage/API design.
User direction recorded 2026-10-25 after the service reports and MCP queries shipped.

## Requested outcomes

- W1. Describe a specific architecture formally enough that agents can follow it and verify
  whether a proposed change respects it.
- W2. Analyze a repository and draft project-specific architecture skills for an easier start.
- W3. Integrate architecture into the edit/review process and visualize changes between states.

These outcomes extend the existing import and service evidence. Reuse current rules and analyzers
rather than introducing another policy language merely to combine their presentation.

## What exists and what is missing

| Outcome | Existing implementation | Missing vertical slice |
| --- | --- | --- |
| W1 | `layout.yaml` modules, layers, public surfaces, forbid/allow claims, reasons and ADR sources; target verdicts and new-only edit gates. Separate service/client verdicts in `services` reports. | One contract view for import rules, service rules, provenance and explicitly manual constraints. Service policies do not yet participate in hooks or change checks. |
| W2 | `init` performs native analysis, drafts a layout and synchronizes four owned instruction blocks. Five generic architecture skills already ship with the repository. | A repository-specific skill draft, concrete source-backed examples, and an explicit acceptance/lifecycle bridge that does not confuse observations with intended policy. |
| W3 | Import diffs, pre-edit snapshots, PR review summaries, a history chart and snapshot navigation. | An added/removed/changed relationship overlay. Services and resource usages have no comparison model or history path. |

Source seams: `src/rules.rs`, `src/target.rs`, `src/adr.rs`, `src/cli/init.rs`,
`src/sync_rules.rs`, `src/context.rs`, `src/cli/hooks.rs`, `src/edit_gate.rs`, `src/services.rs`,
`src/mcp_tools.rs`, `src/diff.rs`, `src/history.rs`, and `src/history.html`.

## W1. Contract-to-agent slice

Expose authoritative policy from the existing layout and service manifest together. Preserve
rule IDs, domains, reasons, source references and current verdicts. Domain-qualified identities
avoid collisions between an import rule and a service rule with the same ID.

Classify enforceability explicitly:

- Mechanically checked: supported dependency direction, public entrypoints, service calls and
  client-use rules, within the collected evidence and configured scope.
- Manual or semantic review: requirements such as transaction correctness or business behaviour
  that the import/service model cannot establish.
- Unavailable: a rule requires evidence that this collection did not provide.

Do not turn absence of evidence into a successful check. Service rules currently lack ADR-source
metadata. The `rules` MCP tool covers layout rules. By default, a valid `services` command exits
successfully even when violations are reported. W1a adds an explicit whole-report check. A service
change gate still needs a comparison contract before integration into the edit loop.

Acceptance: an agent can retrieve the relevant contract with reasons and limitations, evaluate a
code change against the same contract, and distinguish a new violation from an existing one or
a policy change. Existing import-rule and service-report compatibility remains covered.

### W1a. Service policy retrieval and an explicit check

The first increment uses the existing `rules`, `target`, and `check_diff` tools for imports.
The `services` MCP tool gains two fields:

- `forbid` contains the complete ordered authored rule array, including rules with no observed match.
  It remains report-wide when `service` narrows the returned evidence. Existing MCP counts and
  truncation markers disclose omitted rules.
- `policy_source` is `null` without an explicit manifest. With a manifest, it contains the
  canonical repository-relative `path` and the `sha256` of the same bytes parsed for this report.

Rule IDs remain in their existing domain. The service tool and rule ID identify a service policy
without adding a second ID format. This increment does not infer intended policy from code.

`services --fail-on-forbidden` checks the full report before any output projection. It emits the
normal report and exits 1 for any forbidden call or resource usage. It exits 0 when the collected
evidence contains no forbidden relationship. Invalid input, failed output writes, and an explicit
check with no authored forbid rules exit 2. The default report command keeps its existing behavior.

This is a whole-report check. It does not distinguish new violations from pre-existing ones,
integrate service policies into edit hooks, store manual constraints, or prove complete compliance.
Those remain W1 follow-up work. Existing import checks already distinguish new violations.

Verification covers separate planted call and resource violations, clean evidence, absent rules,
unchanged default output, saved JSON, brief output, and output errors. MCP verification covers
unmatched rules, report-wide policy with selected evidence, fresh manifest hashes, containment,
published schemas, and explicit policy truncation.

The 62 targeted service, MCP-tool, and stdio tests pass. The full Rust run passes 901 tests with
38 existing tool-dependent tests ignored. Clippy, formatting, and whitespace checks pass.
Independent code review found no issues. No new dependency, policy language, or automatic
adoption step was added.

## W2. Analyze, draft, review, adopt

Extend the existing bootstrap path. A proposed project skill should include the observed module
and service structure, authoritative rule-file references, source-backed allowed/forbidden
examples, commands/MCP calls to run before and after edits, and the constraints requiring manual
review. The skill references the policy files rather than becoming a divergent copy of them.

Keep observations and policy separate. An existing dependency direction is evidence of current
structure, not proof that the project intends to preserve it. Generated output is a draft until
reviewed. Place unaccepted skill drafts outside automatic skill-discovery locations. Reuse owned
fences and preservation/conflict handling when adopting or updating agent instructions.

Acceptance: a fixture repository can be analyzed into a repeatable draft; one planted violation
is detected by the generated checking workflow; source edits and manual instruction edits are
preserved across regeneration; unsupported semantic claims remain clearly unverified.

## W3. Visual change review

First define a source-evidence delta, then render it. The user selected modules, imports and
code-rule violations as the first visual-diff level. Show added and removed imports, module
changes and changed rule verdicts. Services, client usages and service calls follow afterward. Keep source-line
movement separate from a structural architecture change. Do not treat every new forbidden edge
as the complete set of new relationships.

Use the same analysis version, scope and policy basis for both snapshots, or disclose that they
are not directly comparable. Policy changes deserve their own explanation. Inferred service IDs
and root-directory naming need a stable comparison convention; avoid guessing that removals and
additions are renames.

The graph should make additions/removals readable without relying only on red/green colour.
Selecting a changed relationship shows before/after source evidence and its rule/claim effect.
Connect this to the existing snapshot, new-only gate and PR-review workflow rather than a second
independent history mechanism.

Acceptance: one actual code change produces a truthful before/after view, a planted forbidden
relationship is highlighted with its reason, removed relationships stay inspectable, and missing
or incompatible snapshots never look like a clean no-change result.

## Proposed sequence

1. W1: expose existing policy and its checking limits to agents, then add service-policy change checks.
2. W2: project architecture skill drafting and controlled adoption using those contracts.
3. W3: module/import/rule-violation delta and visual overlay, then extend the same interaction to
   services and resource usages.

Confirmed preference: visual diffs start with modules, imports and code-rule violations.
Services/infrastructure comparison is a later extension.
No product rename, new dependency, policy schema, or new command name is selected here.

## HTTP experiment checkpoint

The next HTTP-wrapper experiment failed its acceptance check. It recovered the same 3 of 11
frozen source-labelled flows as the published implementation, with no real application HTTP gain.
Independent review also found stale captured-variable effects and route rollback caused by an
unrelated method call. Passing unit tests did not resolve those failures.

The experiment is preserved locally in `experiment/http-linker-20261025` at `9d53bba`.
It is excluded from `feat/app-model` and the PR. Further work requires a bounded effect model,
the existing precision probes, and the reduced real registration fixture. Reintegration requires
the unchanged corpus to recover A3 and at least 4 of 11 flows while retaining the reference flows
and negative controls. Additional predictions need independent review.
