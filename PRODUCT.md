# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Developers and coding agents inspecting a repository, changing its structure, or checking that
architecture rules still hold. These users need source evidence and repeatable checks.

## Product Purpose

Boundrail reads source relationships and checks architecture rules. Its service model infers
application boundaries, infrastructure client usage, and supported HTTP and queue relationships.
An offline service catalog and graph make that model explorable.

## Positioning

Local, source-based architecture evidence without requiring an application SDK or running the
application. The service model is incomplete static evidence, not a runtime traffic trace.

## Operating Context

Rust CLI, JSON and Markdown reports, self-contained browser reports, and read-only MCP tools.
Repositories can supply a service manifest to declare relationships or override boundaries.
Existing import collectors have their own tooling and safety constraints documented in the guide.

## Capabilities and Constraints

- Native service facts cover TypeScript, JavaScript, Go, and Python.
- Rules can judge service calls and explicit package targets for infrastructure clients.
- Confidence, source locations, caveats, and omitted evidence must remain visible.
- Resources describe per-service client usage. They do not establish physical database ownership.
- Service inference and report viewing do not provision, deploy, or contact infrastructure.
- Configuration, indirect wrappers, and unsupported transports can hide real relationships.
- The HTML artifact works offline and does not fetch source code or third-party assets.

## Architecture workflow

The [architecture workflow plan](docs/plans/architecture-workflow.md) records the delivered increments
and their limits. Service-report diffs distinguish newly observed violations from existing ones
under the later policy. Project skills have separate inactive drafting and explicit adoption.
Import comparison pages show module, dependency, rule, and source-evidence changes.

Visual diffs cover modules, imports, and code-rule violations. Service and infrastructure visual
diffs remain a later extension. Missing provenance and incomplete static evidence stay visible.

Existing code structure is evidence for a draft. Authored policy defines the intended architecture.
Generated skills must preserve that distinction and reference the policy files.

## Brand Commitments

The product is Boundrail and its executable is `boundrail`. The user selected the name on
2026-10-25. Keep the established report palette and use direct technical copy. Distinguish inferred
evidence from declared evidence and measured outcomes. Historical captures retain their original name.

Use Boundrail consistently in report schemas, project state, owned markers, finding IDs,
environment variables, and repository URLs. The README explains current capabilities with an
illustrative overview and actual report captures. Registry publication is a separate operation.

## Evidence on Hand

README.md, docs/guide.md, docs/plans/app-model.md, docs/plans/service-experience.md, services-report
schema and fixtures, and the existing assets/graph.html report. The pinned real-application sample
has no inferred service edges. New labelled cohorts must report that limitation rather than imply
whole-repository accuracy from reference examples.

## Product Principles

- Show the source location behind a relationship.
- Measure labelled evidence and disclose unresolved or unreviewed cases.
- Preserve existing manifest output and import-report behaviour.
- Reuse the analyzer across the CLI, browser report, and MCP.

## Accessibility & Inclusion

Browser reports support keyboard navigation, visible focus, readable contrast, narrow screens,
and reduced-motion preferences. Essential evidence remains available outside the diagram.
