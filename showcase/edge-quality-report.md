# Edge-quality validation

Measured stages: baseline, fix1, fix2. Pending: fix3, fix4, fix5.

This is an interim report until all five fixes have measured snapshots. README numbers must not be replaced from an incomplete run.

## Methodology

The first 20 entries in `showcase/repos.json` are the published README cohort (11 TypeScript/JavaScript and nine Python repositories). Three supplemental Go repositories are reported separately. Every stage scans the exact manifest commit and the same scanned root and explicit flags; defaults change only with the corresponding fix.

The baseline is the frozen pre-change source at `6627a5e22013044516a599e86bede437ba76ad10`. Targets are separate clean checkouts. No target installation, target build scripts, or annotation processors run. The runner captures commands, engine/source hashes, graph hashes, logs, and raw collector stdout/stderr under `<work>/<stage>/<repo>/`; cached source repositories are never checked out or reset.

**Counting units:** edges are canonical file-dependency records, not source import statements. One named import can resolve to several defining files and increase the edge count. Runtime/lazy/type/re-export columns below count edges, and legacy static/dynamic names are mapped to runtime/lazy for comparison. Import-statement totals and resolution percentages remain pending until the resolution ledger exists; neither is inferred from graph edges or the old unresolved counter.

The baseline already excluded type-only and dynamic edges from runtime cycles. Fix 1 additionally excludes structural re-exports, so an observed decrease is not automatically a newly detected type-only cycle. Deferred SCCs containing an existing runtime SCC are reported separately from wholly non-runtime SCCs. JavaScript re-exports can still cause module evaluation: these results describe the requested graph policy, not proof that all runtime execution is acyclic.

## Engine provenance

| Stage | Source commit | Binary SHA-256 |
| --- | --- | --- |
| baseline | `6627a5e22013044516a599e86bede437ba76ad10` | `655ce6f06b4e37b3e79d7f3191fb87d30dd7723537d7a3262a10246736708346` |
| fix1 | `15928d30034179e496becaa30ea6b0da79fbf9a6` | `3119755b7a48c0bd8d0043a05f37b69d80ce9bcc9cc7f54153e92623aeac00f0` |
| fix2 | `e6a1d9dc49e6283ff8286131b443321e9e5022fb` | `b55dc298d7565fb6f337242970fba4ae222ed5b21434d41cd892f7b329130b96` |

Collector versions are recorded per scan in `metadata.json`. Reproduce a stage with:

```sh
python3 -B showcase/edge_quality.py --engine /absolute/frozen-archgraph \
  --engine-commit FULL_SOURCE_COMMIT --stage fixN
python3 -B showcase/edge_quality.py --report-output showcase/edge-quality-report.md
```

## Published 20 repositories

Within each arrow cell the stages are baseline → fix1 → fix2. SCC means a strongly connected component with a cycle; largest is its module count.

| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (fix2) | Barrels (fix2) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| nest | 666 → 666 → 666 | 2206 → 2206 → 2546 | 6 → 5 → 5 | 16 → 15 → 15 | 1889 / 6 / 185 / 466 | 95 |
| next.js | 1542 → 1542 → 1542 | 6058 → 6058 → 6075 | 2 → 2 → 2 | 62 → 61 → 61 | 4397 / 204 / 1362 / 112 | 50 |
| excalidraw | 364 → 364 → 364 | 1447 → 1447 → 1481 | 1 → 1 → 1 | 29 → 29 → 29 | 1162 / 5 / 247 / 67 | 6 |
| immich | 458 → 458 → 458 | 2053 → 2053 → 2053 | 2 → 2 → 2 | 9 → 9 → 9 | 1556 / 1 / 496 / 0 | 0 |
| activepieces | 757 → 757 → 757 | 3100 → 3100 → 3100 | 1 → 1 → 1 | 32 → 32 → 32 | 2992 / 1 / 101 / 6 | 1 |
| payload | 819 → 819 → 819 | 3805 → 3805 → 3864 | 1 → 1 → 1 | 21 → 21 → 21 | 2144 / 1 / 1529 / 190 | 51 |
| typeorm | 496 → 496 → 496 | 2586 → 2586 → 2598 | 3 → 3 → 3 | 27 → 27 → 27 | 1097 / 0 / 1440 / 61 | 2 |
| vite | 135 → 135 → 135 | 749 → 749 → 749 | 2 → 2 → 2 | 5 → 5 → 5 | 498 / 5 / 239 / 7 | 3 |
| outline | 2243 → 2243 → 2243 | 10778 → 10778 → 11544 | 1 → 1 → 1 | 40 → 40 → 40 | 10063 / 102 / 1246 / 133 | 42 |
| strapi | 236 → 236 → 236 | 361 → 361 → 365 | 2 → 2 → 2 | 5 → 3 → 3 | 280 / 0 / 43 / 42 | 8 |
| trpc | 83 → 83 → 83 | 270 → 270 → 313 | 1 → 1 → 1 | 5 → 5 → 5 | 141 / 0 / 107 / 65 | 14 |
| django | 713 → 713 → 713 | 3170 → 3170 → 3223 | 2 → 2 → 2 | 31 → 31 → 31 | 2865 / 180 / 0 / 178 | 25 |
| fastapi | 52 → 52 → 52 | 123 → 123 → 128 | 1 → 1 → 1 | 14 → 14 → 14 | 117 / 3 / 1 / 7 | 4 |
| flask | 21 → 21 → 21 | 69 → 69 → 69 | 0 → 0 → 0 | 0 → 0 → 0 | 38 / 5 / 15 / 11 | 1 |
| pydantic | 79 → 79 → 79 | 362 → 362 → 362 | 1 → 1 → 1 | 8 → 8 → 8 | 269 / 36 / 57 / 0 | 1 |
| httpx | 23 → 23 → 23 | 87 → 87 → 87 | 0 → 0 → 0 | 0 → 0 → 0 | 74 / 1 / 6 / 6 | 1 |
| scrapy | 194 → 194 → 194 | 942 → 942 → 987 | 2 → 2 → 2 | 18 → 18 → 18 | 713 / 14 / 249 / 11 | 3 |
| celery | 161 → 161 → 161 | 614 → 614 → 615 | 2 → 2 → 2 | 5 → 5 → 5 | 545 / 43 / 11 / 16 | 6 |
| zulip | 733 → 733 → 733 | 3172 → 3172 → 3614 | 1 → 1 → 1 | 14 → 14 → 14 | 3516 / 63 / 5 / 30 | 2 |
| sqlalchemy | 245 → 245 → 245 | 2342 → 2342 → 2430 | 2 → 2 → 2 | 3 → 3 → 3 | 1581 / 61 / 579 / 209 | 7 |

## Supplemental Go repositories

| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (fix2) | Barrels (fix2) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| etcd | 571 → 571 → 571 | 1982 → 1982 → 1982 | 0 → 0 → 0 | 0 → 0 → 0 | 1982 / 0 / 0 / 0 | 0 |
| prometheus | 429 → 429 → 429 | 2183 → 2183 → 2183 | 0 → 0 → 0 | 0 → 0 → 0 | 2183 / 0 / 0 / 0 | 0 |
| cli | 535 → 535 → 535 | 3293 → 3293 → 3293 | 0 → 0 → 0 | 0 → 0 → 0 | 3293 / 0 / 0 / 0 | 0 |

## Resolution and exclusions

Source import-statement totals, resolved percentages, default-excluded file counts, and grab-bag reclassification counts: **pending fixes 3–5**. Old collector dependency counts have a different denominator and are not presented as import-statement coverage.

## Every baseline module SCC

This inventory includes all original SCCs, including shrinkage. An old SCC absorbed into a larger SCC still persists; it is not counted as removed. Fixes 1–2 have one fully eliminated original SCC, six shrunken original SCCs, and 26 unchanged original SCCs. Fix 2 also expands the surviving Nest common SCC from five to six modules.

| Repository / baseline SCC | Original modules | fix1 | fix2 | Explanation of changed membership |
| --- | ---: | --- | --- | --- |
| nest #1: `packages/common, packages/common/decorators, packages/common/exceptions, …` | 10 | shrunk or split (5 original modules remain cyclic) | shrunk or split (6 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. Named barrel resolution at fix2 restores common itself to the surviving SCC (5 → 6); this is expansion, not another disappearance. |
| nest #2: `packages/core, packages/core/adapters, packages/core/discovery, …` | 16 | shrunk or split (15 original modules remain cyclic) | shrunk or split (15 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| nest #3: `packages/microservices, packages/microservices/client, packages/microservices/context, …` | 12 | shrunk or split (11 original modules remain cyclic) | shrunk or split (11 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| nest #4: `packages/platform-fastify, packages/platform-fastify/adapters, packages/platform-fastify/decorators` | 3 | eliminated | eliminated | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| nest #5: `packages/testing, packages/testing/interfaces` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| nest #6: `packages/websockets, packages/websockets/adapters, packages/websockets/context, …` | 5 | shrunk or split (4 original modules remain cyclic) | shrunk or split (4 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| next.js #1: `build, build/adapter, build/analysis, …` | 62 | shrunk or split (61 original modules remain cyclic) | shrunk or split (61 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| next.js #2: `next-devtools, next-devtools/dev-overlay` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| excalidraw #1: `actions, animatedTrail.ts, clients.ts, …` | 29 | unchanged | unchanged | Unchanged through measured stages. |
| immich #1: `constants.ts, cores, decorators.ts, …` | 9 | unchanged | unchanged | Unchanged through measured stages. |
| immich #2: `maintenance, middleware, services` | 3 | unchanged | unchanged | Unchanged through measured stages. |
| activepieces #1: `app, app/action-run, app/agents, …` | 32 | unchanged | unchanged | Unchanged through measured stages. |
| payload #1: `admin, auth, checkPayloadDependencies.ts, …` | 21 | unchanged | unchanged | Unchanged through measured stages. |
| typeorm #1: `driver, driver/aurora-mysql, driver/aurora-postgres, …` | 27 | unchanged | unchanged | Unchanged through measured stages. |
| typeorm #2: `persistence, persistence/subject-builder` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| typeorm #3: `query-builder, query-builder/relation-id` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| vite #1: `module-runner, module-runner/sourcemap` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| vite #2: `node, node/optimizer, node/plugins, …` | 5 | unchanged | unchanged | Unchanged through measured stages. |
| outline #1: `app, app/actions, app/components, …` | 40 | unchanged | unchanged | Unchanged through measured stages. |
| strapi #1: `configuration, core-api/routes, ee, …` | 5 | shrunk or split (3 original modules remain cyclic) | shrunk or split (3 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| strapi #2: `domain/module, registries` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| trpc #1: `unstable-core-do-not-import, unstable-core-do-not-import/error, unstable-core-do-not-import/http, …` | 5 | unchanged | unchanged | Unchanged through measured stages. |
| django #1: `__init__.py, apps, conf, …` | 31 | unchanged | unchanged | Unchanged through measured stages. |
| django #2: `contrib/admin, contrib/auth, contrib/contenttypes, …` | 4 | unchanged | unchanged | Unchanged through measured stages. |
| fastapi #1: `__init__.py, _compat, applications.py, …` | 14 | unchanged | unchanged | Unchanged through measured stages. |
| pydantic #1: `_internal, config.py, errors.py, …` | 8 | unchanged | unchanged | Unchanged through measured stages. |
| scrapy #1: `__init__.py, addons.py, core, …` | 18 | unchanged | unchanged | Unchanged through measured stages. |
| scrapy #2: `cmdline.py, commands` | 2 | unchanged | unchanged | Unchanged through measured stages. |
| celery #1: `app, canvas.py, concurrency, …` | 5 | unchanged | unchanged | Unchanged through measured stages. |
| celery #2: `exceptions.py, platforms.py, utils` | 3 | unchanged | unchanged | Unchanged through measured stages. |
| zulip #1: `actions, context_processors.py, data_import, …` | 14 | unchanged | unchanged | Unchanged through measured stages. |
| sqlalchemy #1: `dialects, engine, sql` | 3 | unchanged | unchanged | Unchanged through measured stages. |
| sqlalchemy #2: `exc.py, util` | 2 | unchanged | unchanged | Unchanged through measured stages. |

## Hand spot-checks

Eleven source reads at the exact pins below check one complete SCC disappearance and individual membership removals in shrunken SCCs. They are **not eleven removed SCCs**. For each witness, the baseline edge is a re-export that participated in the old module projection; fix1/fix2 retain the edge as structural and omit it from the runtime projection. No corresponding import was silently erased.

| Repository | Source evidence at pinned commit | Checked consequence | Classification |
| --- | --- | --- | --- |
| nest | [`packages/common/index.ts:12`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/index.ts#L12) — `export * from './file-stream/index.js';` | common/file-stream leaves the common SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/common/index.ts:78`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/index.ts#L78) — `export * from './module-utils/index.js';` | common/module-utils leaves the common SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/common/index.ts:79`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/index.ts#L79) — `export * from './pipes/index.js';` | common/pipes leaves the common SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/common/index.ts:80`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/index.ts#L80) — `export * from './serializer/index.js';` | common/serializer leaves the common SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/core/index.ts:27`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/core/index.ts#L27) — `export * from './repl/index.js';` | core/repl leaves the core SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/microservices/index.ts:18`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/microservices/index.ts#L18) — `export * from './module/index.js';` | microservices/module leaves its SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/websockets/index.ts:10`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/websockets/index.ts#L10) — `export * from './decorators/index.js';` | websockets/decorators leaves its SCC | SCC shrink; barrel/re-export policy |
| nest | [`packages/platform-fastify/index.ts:8`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/platform-fastify/index.ts#L8) — `export * from './adapters/index.js';` | Fastify parent/adapters/decorators SCC disappears | SCC eliminated; barrel/re-export policy |
| next.js | [`packages/next/src/client/components/client-boundary-params.browser.ts:3`](https://github.com/vercel/next.js/blob/00c4ba8b6e268c792da64207fa9453132bfcec5f/packages/next/src/client/components/client-boundary-params.browser.ts#L3) — `export { createRenderParamsFromClient as createClientParams } from '../request/params.browser'` | client/request leaves the 62-module SCC | SCC shrink; barrel/re-export policy |
| strapi | [`packages/core/core/src/utils/index.ts:3`](https://github.com/strapi/strapi/blob/d764743be322a70b98cb7b43c5d1a3e0d67d7efd/packages/core/core/src/utils/index.ts#L3) — `export { getDirs } from '../configuration/get-dirs';` | configuration leaves the five-module SCC | SCC shrink; barrel/re-export policy |
| strapi | [`packages/core/core/src/utils/ee.ts:1`](https://github.com/strapi/strapi/blob/d764743be322a70b98cb7b43c5d1a3e0d67d7efd/packages/core/core/src/utils/ee.ts#L1) — `export { default as ee } from '../ee';` | ee leaves the five-module SCC | SCC shrink; barrel/re-export policy |

Additional positive controls confirm that named runtime imports remain runtime and explicit type imports remain type after barrel resolution:

| Repository | Import source | Defining source | Observed kind / weight |
| --- | --- | --- | --- |
| nest | [`packages/core/application-config.ts:12`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/core/application-config.ts#L12) | [`packages/common/interfaces/global-prefix-options.interface.ts:6`](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/interfaces/global-prefix-options.interface.ts#L6) | type / 0.1 |
| excalidraw | [`packages/excalidraw/actions/actionCanvas.tsx:35`](https://github.com/excalidraw/excalidraw/blob/4c37956ba5f04ec342a9d9d5891a38f22c5b7b4e/packages/excalidraw/actions/actionCanvas.tsx#L35) | [`packages/excalidraw/scene/normalize.ts:7`](https://github.com/excalidraw/excalidraw/blob/4c37956ba5f04ec342a9d9d5891a38f22c5b7b4e/packages/excalidraw/scene/normalize.ts#L7) | runtime / 1.0 |
| django | [`django/conf/urls/__init__.py:1`](https://github.com/django/django/blob/d430084791a61b31cecec132bce3dca19194f7f2/django/conf/urls/__init__.py#L1) | [`django/urls/conf.py:17`](https://github.com/django/django/blob/d430084791a61b31cecec132bce3dca19194f7f2/django/urls/conf.py#L17) | runtime / 1.0 |
| typeorm | [`packages/typeorm/src/commands/CommandUtils.ts:4`](https://github.com/typeorm/typeorm/blob/fe92394287516feeada6e9c308ebe22deca2458e/packages/typeorm/src/commands/CommandUtils.ts#L4) | [`packages/typeorm/src/data-source/DataSource.ts:56`](https://github.com/typeorm/typeorm/blob/fe92394287516feeada6e9c308ebe22deca2458e/packages/typeorm/src/data-source/DataSource.ts#L56) | type / 0.1 |

Fastify return-path check: `packages/platform-fastify/adapters/fastify-adapter.ts:73` and `packages/platform-fastify/decorators/route-config.decorator.ts:2` import parent-folder `constants.js`; the parent `index.ts:8,10` exports children. These are different files grouped into one parent module, explaining the projected loop.

No wholly removed SCC attributable to type-only imports, test exclusions, resolution fixes, or grab-bag splitting has yet been measured. Those categories remain pending rather than being inferred from a net count.

## Remaining validation

- Measure fixes 3–5 and update the full SCC inventory with stage-specific edge/file evidence.
- Fix the implicit TypeScript type-use gap found in Nest: `packages/common/decorators/core/use-guards.decorator.ts:2,29` imports `CanActivate` only as a type; its interface declaration is `packages/common/interfaces/features/can-activate.interface.ts:14`, but the fix2 snapshot still marks the edge runtime. Mixed type/value imports must retain their runtime bindings. This gap is pending bounded refinement, not counted as solved.
- Fill the import-statement ledger, resolution coverage, exclusion counts, and grab-bag classifications from actual engine fields.
- Rebuild generated showcase outputs and README numbers only after all stages pass.
- Fix any source witness or resolution failure that contradicts the intended dependency graph before publishing final claims.
