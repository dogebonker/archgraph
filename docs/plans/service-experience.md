# Service flow evidence, HTML report, and MCP

Approved sequence: A1, then A2, then A3. Continue in the existing app-model worktree from
`a086e2c`. Keep the current product name until the user chooses a replacement. No new dependencies.

## A1. Measure real source flows before extending inference

Freeze independently read caller and receiver evidence from the pinned Immich, activepieces,
and Outline sources. Label intended default-deployment HTTP flows separately from runtime traffic.
Activepieces API/worker job delivery is Socket.IO RPC, not a cross-service BullMQ edge. Same-service
queues and external HTTP calls are negative controls.

Add a rerunnable benchmark with source-file hashes, exact caller locations, expected endpoints,
miss reasons, and explicit unreviewed predictions. Missing source evidence is an error. Precision
and recall apply only to the labelled cohort. Duplicate predictions cannot inflate coverage.
Tests must detect a missing edge and a swapped destination with the same total edge count.

Extend supported syntax only when its meaning is established. In particular, `new URL('predict',
unknownBase)` has no known absolute pathname. Do not invent `/predict` or discard a known external
origin. A literal base, rooted path, or local immutable string can provide narrower evidence.
Keep unresolved configuration and unsupported transports visible as misses.

Obtain a pinned public reference application with a genuine producer and consumer in separate
service boundaries if the existing repositories have no positive queue sample. Identify it as a
reference application. A1 ends with reproducible results, regression checks, and at least one
source-backed positive service edge. Unresolved flows remain documented.

Expected files: `benchmarks/service_flows.py`, its labelled JSON and unit tests, bounded parser or
inference changes under `src/collect/native/` and `src/infer_services.rs`, relevant existing Rust
tests, guide and changelog. Preserve collector caches, scan JSON, and manifest report bytes.

## A2. Extend the offline HTML report

`archgraph render services-report.json -o services.html` renders the existing services report.
Keep scan rendering unchanged. Reject scan-only `--rules` on service reports. Validate untrusted
service reports at the read boundary and escape embedded JSON using the existing renderer rule.

Use the incumbent dark report tokens, system typography, semantic colours and evidence inspector.
Provide a searchable service catalog, directed service calls, infrastructure, confidence and rule
verdicts. Selecting a service, resource or call exposes its exact source location. No source code
is fetched or executed. Empty call graphs explicitly say that no calls were inferred.

Keep the artifact self-contained. Preserve keyboard access, focus visibility, readable contrast,
reduced-motion preferences, and a usable narrow-screen layout. DOM tests cover interactions and
injection. Inspect desktop and mobile in a real browser before declaring completion.

Expected files: `src/services.rs`, `src/render.rs`, `src/main.rs`, a service HTML asset, renderer and
UI tests, design context, guide and changelog. No replacement of the existing scan visual system.

## A3. Expose the same model through MCP

Add a read-only `services` tool using `infer_services::infer`. Validate arguments and root before
collection. Bypass import-scan setup for this tool. An optional manifest is repository-relative
and cannot escape through symlinks. Keep existing output caps, counts, omissions and freshness.

Runtime tool schemas and `schema/mcp.v1.schema.json` must agree. Cover the stdio transport,
`mcp-call`, malformed inputs, contained manifests, deterministic refresh and oversized reports.
Existing MCP tools keep their current behaviour and tool output contracts.

Expected files: `src/mcp_tools.rs`, `src/cli/mcp.rs`, MCP schemas and existing tests, guide and
changelog. No new MCP library or protocol feature is required.

## Verification and delivery

Each stage receives an independent code review and targeted verification before the next stage.
Run the full Rust suite, clippy, formatting, UI tests and CI at the final checkpoint. Keep benchmark
artifacts outside the checkout and commit the runner and labels. Publishing remains on the
feature branch. Use unsigned commits if signing would require a prompt, per the user's instruction.

## Results

A1 is measured with frozen baseline and candidate binaries. The labelled cohort improved from
2/11 to 3/11 recovered flows. All three NATS reference flows are recovered; all eight application
HTTP flows remain explicit configuration/wrapper misses. No false positives, unreviewed
predictions, or violated controls occurred in this labelled sample. See
`benchmarks/SERVICE_FLOWS.md` for hashes, pins, assumptions, and reproduction.

A2 supports exported service reports through `render`. The reader validates evidence and rebuilds
derived metrics and verdicts. A separate self-contained report provides catalog search, call and
client inspectors, source locations, and truthful empty states. Desktop 1365x900 and phone
390x844 browser checks passed. The phone graph retains 13px labels with horizontal scrolling and
no page overflow. Search, selection, keyboard activation, source evidence, and no-call output were
exercised with real exported NATS and Immich reports. Browser console errors: zero. The mechanical
design detector reported no findings. Screenshots remain in the validation scratch directory.

A3 exposes live services through MCP with an optional contained manifest and exact service focus.
Both transport and top-level daemon forwarding bypass import refresh for this tool. An actual
`mcp-call` with an unusable reference collector returned the expected pub-to-sub NATS call.
Integration tests cover freshness, containment, malformed scan rules, schema parity, and the
4,000-byte cap with truthful scope and counts. Existing nine tool contracts are unchanged.

Independent code review approved every stage. Final local verification passes 897 Rust tests,
124 UI tests, and 15 benchmark tests. The 38 existing tool-dependent Rust tests remain ignored in
the default suite. Clippy, formatting, and diff checks pass. No dependencies were added.
