# Design

## Source of truth

Status: Active. Date: 2026-10-25. Surfaces: the service catalog and flow report exported by
`archgraph render`, and the import comparison exported by `archgraph diff --html`.
Both reuse the existing offline report identity.

Evidence: PRODUCT.md, assets/graph.html, src/render.rs, tests/render.rs, existing Node UI tests,
src/services.rs, and docs/plans/service-experience.md. Existing screenshots are documentation
assets for the import report, not references for a replacement identity.

## Brand

A precise developer tool. Trust comes from source locations, explicit confidence, and truthful
limits. Keep the current name. Avoid claims that static candidates prove runtime calls.

## Product goals

Make service boundaries, owned client usage, and service-call evidence easy to inspect. Help a
reader find a service, follow a relationship, and understand its source and rule verdict.
Do not provision infrastructure, infer resource sizing, or imply shared physical infrastructure.

## Personas and jobs

A developer reviews an application before changing it. A reviewer checks a boundary violation.
An agent consumes the same model through JSON or MCP. The browser report serves the human review.

## Information architecture

A header identifies the report and coverage limits. A searchable catalog lists services. A graph
shows directed calls. An evidence inspector presents the selected service, call, or resource.
Global totals remain distinct from filtered totals. Source locations are inspector actions,
not untrusted external links.

The comparison header identifies both snapshots and their policy and collection compatibility.
A module graph shows additions and removals. A searchable change list includes added and removed
imports, changed rule verdicts, and source evidence updates. Selection opens before/after evidence
with file locations, import kinds, rule IDs, and reasons. Removed modules and imports stay selectable.

## Design principles

Keep evidence next to the selected relationship. Show inferred, declared, uncertain, and forbidden
states using text as well as colour. A missing edge is not proof of independence. Every graph
interaction has an equivalent catalog or evidence-list interaction.

## Visual language

Inherit the black canvas and dark surfaces from assets/graph.html. Reuse system sans typography,
monospace source paths, blue focus and selection, and existing purple, teal, orange, and red accents.
Red denotes forbidden evidence or explicit cycle signals. Use subdued separators and restrained
glass treatment for controls and the inspector. No remote fonts, imagery, or new visual identity.
Spacing follows the existing 8/12/16/24px scale. Motion is limited to direct selection feedback.

## Components

Reuse the incumbent conventions for search, compact totals, semantic colour, evidence buttons,
focus, and announcements. New components are the service list, directed call diagram, per-service
resource rows, and the service evidence inspector. Resource IDs identify client usages, not a
single database node shared by every package import.

The comparison reuses search, totals, evidence buttons, and focus treatments. Additions use a plus
label, removals use a minus label and dashed graph edges, and verdict changes use a text label.
Module identity is its exact ID. Display removals and additions rather than guessed renames.

## Accessibility

Use headings, labelled search, native buttons, and a service list outside SVG. Support Enter and
Space on graph selections, visible focus, and a polite status announcement after filtering or
selection. Confidence and verdicts have readable labels. Do not make colour the only distinction.

## Responsive behavior

Use the wide viewport for catalog, graph, and inspector. On narrow screens stack these regions,
keep controls reachable, and allow the diagram to scroll without clipping evidence. Test desktop
and phone widths with long service IDs and source paths.

## Interaction states

Initial view shows the full service inventory. Selection shows details and connected calls.
Search exposes matching services and an explicit count. Empty search, no resources, no inferred
calls, and a report with no services each explain what is absent without claiming completeness.
Malformed inputs fail in the CLI before producing HTML. Clipboard failure leaves selectable text.

For comparisons, missing collection provenance produces a visible unverified state. Known scope,
language, or collector incompatibility fails before export. Policy changes have their own notice.
An empty compatible comparison says no structural changes were observed in the supplied evidence.
Line-only movement changes source evidence, rather than appearing as an added and removed import.

## Content voice

Use literal terms: service, call, client, source, confidence, rule, and caveat. Say "No calls
inferred" for an empty inferred graph. Reserve "declared" for manifest evidence. Do not call
medium-confidence file-import candidates proved runtime behaviour.

## Implementation constraints

A self-contained HTML asset generated by Rust. No new dependencies or network requests. Preserve
the incumbent CSP and escape every `<` in embedded JSON. Build DOM content with textContent and
safe attributes. Validate exported service reports at the input boundary and recompute derived
metrics and verdicts. Existing scan rendering remains unchanged.

The comparison uses the existing two-snapshot diff command and the validated import reader.
Its page includes external package imports. Escape embedded JSON and build repository text with
textContent. Reject output paths that alias either input. Preserve the existing JSON diff and gate
contracts. Browser QA covers keyboard selection, before/after evidence, empty and unverified states,
source text escaping, and desktop and phone layouts.

## Open questions

No blocking design questions remain for this extension. Product naming and deeper configuration
inference remain separate decisions outside the current surface implementation.
