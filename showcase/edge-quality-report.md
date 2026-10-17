# Edge-quality validation

Measured stages: baseline, fix1, fix2, fix3, fix4, fix5, corrected. Pending: none.

All five fixes have measured snapshots.

## Methodology

The first 20 entries in `showcase/repos.json` are the published README cohort (11 TypeScript/JavaScript and nine Python repositories). Three supplemental Go repositories are reported separately. Every stage scans the exact manifest commit and the same scanned root and explicit flags; defaults change only with the corresponding fix.

The baseline is the frozen pre-change source at `6627a5e22013044516a599e86bede437ba76ad10`. Targets are separate clean checkouts. No target installation, target build scripts, or annotation processors run. The runner captures commands, engine/source hashes, graph hashes, logs, and raw collector stdout/stderr under `<work>/<stage>/<repo>/`; cached source repositories are never checked out or reset.

**Counting units:** edges are canonical file-dependency records, not source import statements. One named import can resolve to several defining files and increase the edge count. Runtime/lazy/type/re-export columns in the architecture tables count edges, and legacy static/dynamic names are mapped to runtime/lazy for comparison. The separate resolution table counts source specifier occurrences only when a ledger exists; missing coverage is never inferred from graph edges or the old unresolved counter. External nodes and edges stay outside internal module metrics.

The baseline already excluded type-only and dynamic edges from runtime cycles. Fix 1 additionally excludes structural re-exports, so an observed decrease is not automatically a newly detected type-only cycle. Deferred SCCs containing an existing runtime SCC are reported separately from wholly non-runtime SCCs. JavaScript re-exports can still cause module evaluation: these results describe the requested graph policy, not proof that all runtime execution is acyclic.

## Python exact-case correction

The original baseline/fix1–fix5 measurements were collected on macOS. A case-insensitive filesystem lookup in the Python resolver treated differently cased names as existing module files and introduced synthetic capitalized paths. The corrected stage requires exact path-component spelling and rescans all 23 pins. Its counts are authoritative for the final report and README. Earlier archives remain unchanged as historical engine measurements; synthetic-path removal must not be credited as architectural improvement.

| Repository | Files: fix5 → corrected | Edges: fix5 → corrected | Removed synthetic paths (examples) |
| --- | ---: | ---: | --- |
| django | 716 → 706 | 3175 → 3119 | `django/contrib/gis/gdal/DataSource.py`, `django/contrib/gis/geos/LineString.py`, `django/contrib/gis/geos/Point.py` |
| scrapy | 198 → 194 | 1004 → 988 | `scrapy/Selector/__init__.py`, `scrapy/http/Headers.py`, `scrapy/http/Request/__init__.py` |
| celery | 164 → 161 | 640 → 617 | `celery/utils/dispatch/Signal.py`, `celery/worker/consumer/Consumer.py`, `celery/worker/consumer/Tasks.py` |
| sqlalchemy | 245 → 240 | 2500 → 2392 | `sqlalchemy/engine/Result.py`, `sqlalchemy/engine/Row.py`, `sqlalchemy/engine/URL.py` |

Cross-platform check against the **prior Linux deployment**, source `22ee0fb9c7f52d00606e3886be8c0815d9ccc65d`: all 23 corrected repository counts, module/file-cycle counts, findings, import-resolution ledgers and external-package counts exactly match that deployment's cached index (`published-index.json`, SHA-256 `0be0887b0cc078c0d5f75483f70e03a36ef5a33b782a7ba8a08627a32a8790d3`). Full semantic JSON also matches the cached published graphs for django, scrapy, celery, sqlalchemy. This validates platform consistency after the correction, not the status of a new deployment or an architectural improvement claim.

## Engine provenance

| Stage | Source commit | Binary SHA-256 |
| --- | --- | --- |
| baseline | `6627a5e22013044516a599e86bede437ba76ad10` | `655ce6f06b4e37b3e79d7f3191fb87d30dd7723537d7a3262a10246736708346` |
| fix1 | `15928d30034179e496becaa30ea6b0da79fbf9a6` | `3119755b7a48c0bd8d0043a05f37b69d80ce9bcc9cc7f54153e92623aeac00f0` |
| fix2 | `e6a1d9dc49e6283ff8286131b443321e9e5022fb` | `b55dc298d7565fb6f337242970fba4ae222ed5b21434d41cd892f7b329130b96` |
| fix3 | `18532c9d54ab403ef09b6a4f1d2261aa65b07392` | `3bc780de20a87347a98194dde4be176d0c95a20c46790cd5c8ea5c7fc593fdf5` |
| fix4 | `b7f82fc644797b031a099b09d90188f6b9183c67` | `ad570486800f416a1081c918d6da8dcd8d69d07a43c626a66ac17d7e2293b7cb` |
| fix5 | `07985946a40b8917b6d1e8f0d456dec4c3858da0` | `7334d5ea0d483c7037bd7a05e3d0a4f1f7e62a6cfbeffdd823ab1aad379f9d7e` |
| corrected | `0a7d4a928dac9b1677e8ed92e9c707c4f2a5fb8a` | `ed11fe1e29caafbf00bd9459f229e70b9a03ff988d6920d8031e230b060a4ebb` |

Collector versions are recorded per scan in `metadata.json`. Reproduce a stage with:

```sh
python3 -B showcase/edge_quality.py --engine /absolute/frozen-archgraph \
  --engine-commit FULL_SOURCE_COMMIT --stage fixN
python3 -B showcase/edge_quality.py --report-output showcase/edge-quality-report.md
```

## Published 20 repositories

Arrow cells compare baseline → corrected. SCC means a strongly connected component with a cycle; largest is its module count. All intermediate stages remain in the cohort totals, provenance and full SCC inventory.

| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (corrected) | Barrels (corrected) | Excluded candidate files | Non-runtime / extended-runtime SCCs | Via grab-bag: complete / partial |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| nest | 666 → 666 | 2206 → 2546 | 6 → 5 | 16 → 15 | 1534 / 6 / 545 / 461 | 95 | 1377 | 1 / 2 | 0 / 1 |
| next.js | 1542 → 1252 | 6058 → 4627 | 2 → 2 | 62 → 43 | 3358 / 150 / 1019 / 100 | 43 | 1232 | 0 / 1 | 0 / 1 |
| excalidraw | 364 → 364 | 1447 → 1483 | 1 → 1 | 29 → 29 | 1162 / 5 / 249 / 67 | 6 | 110 | 1 / 1 | 0 / 0 |
| immich | 458 → 458 | 2053 → 2053 | 2 → 2 | 9 → 9 | 1555 / 1 / 497 / 0 | 0 | 151 | 0 / 1 | 0 / 0 |
| activepieces | 757 → 757 | 3100 → 3100 | 1 → 1 | 32 → 32 | 2992 / 1 / 101 / 6 | 1 | 545 | 0 / 0 | 0 / 0 |
| payload | 819 → 817 | 3805 → 3856 | 1 → 1 | 21 → 21 | 2138 / 1 / 1528 / 189 | 51 | 113 | 0 / 1 | 0 / 0 |
| typeorm | 496 → 496 | 2586 → 2598 | 3 → 3 | 27 → 27 | 1097 / 0 / 1440 / 61 | 2 | 0 | 0 / 1 | 0 / 0 |
| vite | 135 → 135 | 749 → 762 | 2 → 2 | 5 → 5 | 503 / 5 / 247 / 7 | 3 | 291 | 0 / 0 | 0 / 0 |
| outline | 2243 → 2231 | 10778 → 11469 | 1 → 3 | 40 → 22 | 9983 / 94 / 1259 / 133 | 42 | 385 | 0 / 1 | 0 / 1 |
| strapi | 236 → 236 | 361 → 365 | 2 → 2 | 5 → 3 | 280 / 0 / 43 / 42 | 8 | 116 | 0 / 1 | 0 / 0 |
| trpc | 83 → 76 | 270 → 298 | 1 → 1 | 5 → 5 | 120 / 0 / 117 / 61 | 13 | 32 | 0 / 1 | 0 / 0 |
| django | 713 → 706 | 3170 → 3119 | 2 → 2 | 31 → 30 | 2788 / 156 / 0 / 175 | 24 | 201 | 0 / 1 | 0 / 0 |
| fastapi | 52 → 52 | 123 → 128 | 1 → 1 | 14 → 9 | 117 / 3 / 1 / 7 | 4 | 0 | 0 / 1 | 0 / 1 |
| flask | 21 → 24 | 69 → 95 | 0 → 0 | 0 → 0 | 55 / 5 / 24 / 11 | 1 | 0 | 1 / 0 | 0 / 0 |
| pydantic | 79 → 79 | 362 → 362 | 1 → 1 | 8 → 8 | 269 / 36 / 57 / 0 | 1 | 26 | 0 / 1 | 0 / 0 |
| httpx | 23 → 23 | 87 → 87 | 0 → 0 | 0 → 0 | 74 / 1 / 6 / 6 | 1 | 0 | 2 / 0 | 0 / 0 |
| scrapy | 194 → 194 | 942 → 988 | 2 → 2 | 18 → 18 | 714 / 15 / 248 / 11 | 3 | 0 | 0 / 1 | 0 / 0 |
| celery | 161 → 161 | 614 → 617 | 2 → 2 | 5 → 5 | 545 / 45 / 11 / 16 | 6 | 0 | 0 / 1 | 0 / 0 |
| zulip | 733 → 775 | 3172 → 4073 | 1 → 1 | 14 → 19 | 3930 / 108 / 5 / 30 | 2 | 946 | 0 / 0 | 0 / 0 |
| sqlalchemy | 245 → 240 | 2342 → 2392 | 2 → 2 | 3 → 3 | 1554 / 58 / 579 / 201 | 6 | 18 | 0 / 1 | 0 / 0 |

### Published cohort totals by measured stage

| Stage | Files | Canonical file edges | Module SCCs | Largest module SCC |
| --- | ---: | ---: | ---: | ---: |
| baseline | 10020 | 44294 | 33 | 62 |
| fix1 | 10020 | 44294 | 32 | 61 |
| fix2 | 10020 | 46203 | 32 | 61 |
| fix3 | 9697 | 44514 | 32 | 44 |
| fix4 | 9764 | 45221 | 32 | 44 |
| fix5 | 9764 | 45221 | 34 | 43 |
| corrected | 9742 | 45018 | 34 | 43 |

## Supplemental Go repositories

| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (corrected) | Barrels (corrected) | Excluded candidate files | Non-runtime / extended-runtime SCCs | Via grab-bag: complete / partial |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| etcd | 571 → 571 | 1982 → 1982 | 0 → 0 | 0 → 0 | 1982 / 0 / 0 / 0 | 0 | 528 | 0 / 0 | 0 / 0 |
| prometheus | 429 → 420 | 2183 → 2159 | 0 → 0 | 0 → 0 | 2159 / 0 / 0 / 0 | 0 | 322 | 0 / 0 | 0 / 0 |
| cli | 535 → 526 | 3293 → 3138 | 0 → 0 | 0 → 0 | 3138 / 0 / 0 / 0 | 0 | 439 | 0 / 0 | 0 / 0 |

The fix3 directory rules exclude real source as well as generated/test artifacts: Next.js `packages/next/src/build/index.ts:1115` declares its framework build function; Django `django/test/testcases.py:1378` declares its public TestCase API; Payload `src/cli/commands/build/` contains CLI implementation. The measured reduction must be described as changed scan scope. Layout include overrides can restore Next.js build files and their cycle participation. Including Django/test restores files but they retain the test class and remain excluded from cycles; production-cycle analysis of test-named APIs requires a future classification override.


## Confidence limits on runtime projection

Low-confidence zero-weight re-export fallbacks are retained in JSON but excluded from the default runtime projection. They include un-narrowed namespace/star imports, ambiguous exports, and named exports whose metadata is unproven. These are uncertainty categories, **not proven type-only imports**. Resolved-file coverage and absence of a reported cycle do not prove complete runtime coverage; dynamic wiring and these fallbacks can hide dependencies.

| Repository | Zero-weight low-confidence re-export edges | Namespace/star | Ambiguous | Unproven named metadata | Other |
| --- | ---: | ---: | ---: | ---: | ---: |
| nest | 72 | 72 | 0 | 0 | 0 |
| next.js | 22 | 11 | 0 | 11 | 0 |
| excalidraw | 27 | 1 | 0 | 26 | 0 |
| immich | 0 | 0 | 0 | 0 | 0 |
| activepieces | 6 | 0 | 0 | 6 | 0 |
| payload | 3 | 3 | 0 | 0 | 0 |
| typeorm | 1 | 1 | 0 | 0 | 0 |
| vite | 0 | 0 | 0 | 0 | 0 |
| outline | 2 | 1 | 0 | 1 | 0 |
| strapi | 4 | 4 | 0 | 0 | 0 |
| trpc | 5 | 3 | 2 | 0 | 0 |
| django | 75 | 72 | 3 | 0 | 0 |
| fastapi | 0 | 0 | 0 | 0 | 0 |
| flask | 1 | 1 | 0 | 0 | 0 |
| pydantic | 0 | 0 | 0 | 0 | 0 |
| httpx | 1 | 0 | 1 | 0 | 0 |
| scrapy | 0 | 0 | 0 | 0 | 0 |
| celery | 2 | 2 | 0 | 0 | 0 |
| zulip | 0 | 0 | 0 | 0 | 0 |
| sqlalchemy | 172 | 171 | 0 | 1 | 0 |
| etcd | 0 | 0 | 0 | 0 | 0 |
| prometheus | 0 | 0 | 0 | 0 | 0 |
| cli | 0 | 0 | 0 | 0 | 0 |
| All 23 | 393 | 342 | 6 | 45 | 0 |

## Import resolution: source specifier occurrences

These counts come only from `import_resolution` and the source-occurrence ledger. They include known external packages and do not use the file-edge denominator. An unavailable ledger stays unavailable; a zero-import ledger has no percentage. Percentages are rounded to one decimal, so 100.0% with a nonzero unresolved count is rounding, not complete coverage. Runtime/lazy/type/re-export columns in this table count source occurrences, unlike the architecture table above.

| Repository | Imports | Resolved | Resolved % | Unresolved | Runtime / lazy / type / re-export occurrences | Known external packages |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| nest | 2473 | 2469 | 99.8% | 4 | 1546 / 28 / 437 / 462 | 49 |
| next.js | 6812 | 6464 | 94.9% | 348 | 4390 / 686 / 1608 / 128 | 51 |
| excalidraw | 2926 | 2922 | 99.9% | 4 | 2255 / 15 / 568 / 88 | 34 |
| immich | 2992 | 2991 | 100.0% | 1 | 2458 / 3 / 531 / 0 | 91 |
| activepieces | 5993 | 5991 | 100.0% | 2 | 5869 / 9 / 106 / 9 | 110 |
| payload | 4350 | 4340 | 99.8% | 10 | 2302 / 6 / 1801 / 241 | 54 |
| typeorm | 2676 | 2672 | 99.9% | 4 | 1146 / 3 / 1462 / 65 | 23 |
| vite | 1386 | 1373 | 99.1% | 13 | 828 / 40 / 504 / 14 | 77 |
| outline | 15874 | 15812 | 99.6% | 62 | 13436 / 183 / 2120 / 135 | 236 |
| strapi | 853 | 847 | 99.3% | 6 | 549 / 22 / 240 / 42 | 59 |
| trpc | 326 | 324 | 99.4% | 2 | 123 / 0 / 141 / 62 | 8 |
| django | 4336 | 4206 | 97.0% | 130 | 4061 / 275 / 0 / 0 | 98 |
| fastapi | 492 | 304 | 61.8% | 188 | 471 / 20 / 1 / 0 | 33 |
| flask | 390 | 301 | 77.2% | 89 | 317 / 24 / 49 / 0 | 35 |
| pydantic | 908 | 766 | 84.4% | 142 | 737 / 74 / 97 / 0 | 42 |
| httpx | 223 | 194 | 87.0% | 29 | 185 / 21 / 17 / 0 | 31 |
| scrapy | 2289 | 1975 | 86.3% | 314 | 1765 / 88 / 436 / 0 | 81 |
| celery | 1567 | 1154 | 73.6% | 413 | 1402 / 141 / 24 / 0 | 72 |
| zulip | 7450 | 5347 | 71.8% | 2103 | 7238 / 190 / 22 / 0 | 85 |
| sqlalchemy | 8630 | 8524 | 98.8% | 106 | 6858 / 184 / 1588 / 0 | 57 |
| etcd | 3009 | 3009 | 100.0% | 0 | 3009 / 0 / 0 / 0 | 152 |
| prometheus | 3377 | 3377 | 100.0% | 0 | 3377 / 0 / 0 / 0 | 260 |
| cli | 4767 | 4767 | 100.0% | 0 | 4767 / 0 / 0 / 0 | 152 |

## Resolution and exclusions

Grab-bag reclassifications are measured from `grabbags`: complete means no residual module SCC, while partial retains verified residual runtime SCCs. Import-statement totals and resolved percentages are shown only for snapshots with the fix4 ledger; older snapshots remain unavailable. Excluded candidate files are the actual length of `config.excluded_files` in the latest snapshot: they include pre-existing defaults and explicit manifest exclusions, not just files newly removed in fix3. Non-runtime SCCs have no runtime cycle; extended-runtime SCCs include a runtime SCC and must not be counted as removed cycles.

## Every baseline module SCC

This inventory includes all original SCCs, including shrinkage. An old SCC absorbed into a larger SCC still persists; it is not counted as removed. Through corrected, 22 original SCCs remain exactly unchanged, 9 have shrunk or split, 1 are eliminated, and 1 persist in larger SCCs. Fix 2 expands the surviving Nest common SCC from five to six modules.

| Repository / baseline SCC | Original modules | fix1 | fix2 | fix3 | fix4 | fix5 | corrected | Explanation of changed membership |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| nest #1: `packages/common, packages/common/decorators, packages/common/exceptions, …` | 10 | shrunk or split (5 original modules remain cyclic) | shrunk or split (6 original modules remain cyclic) | shrunk or split (6 original modules remain cyclic) | shrunk or split (6 original modules remain cyclic) | shrunk or split (5 original modules remain cyclic) | shrunk or split (5 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. Named barrel resolution at fix2 restores common itself to the surviving SCC (5 → 6); this is expansion, not another disappearance. Fix5 verifies a cycle via grab-bag; residual real cycles are retained. See the complete/partial evidence table below. |
| nest #2: `packages/core, packages/core/adapters, packages/core/discovery, …` | 16 | shrunk or split (15 original modules remain cyclic) | shrunk or split (15 original modules remain cyclic) | shrunk or split (15 original modules remain cyclic) | shrunk or split (15 original modules remain cyclic) | shrunk or split (15 original modules remain cyclic) | shrunk or split (15 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| nest #3: `packages/microservices, packages/microservices/client, packages/microservices/context, …` | 12 | shrunk or split (11 original modules remain cyclic) | shrunk or split (11 original modules remain cyclic) | shrunk or split (11 original modules remain cyclic) | shrunk or split (11 original modules remain cyclic) | shrunk or split (11 original modules remain cyclic) | shrunk or split (11 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| nest #4: `packages/platform-fastify, packages/platform-fastify/adapters, packages/platform-fastify/decorators` | 3 | eliminated | eliminated | eliminated | eliminated | eliminated | eliminated | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| nest #5: `packages/testing, packages/testing/interfaces` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| nest #6: `packages/websockets, packages/websockets/adapters, packages/websockets/context, …` | 5 | shrunk or split (4 original modules remain cyclic) | shrunk or split (4 original modules remain cyclic) | shrunk or split (4 original modules remain cyclic) | shrunk or split (4 original modules remain cyclic) | shrunk or split (4 original modules remain cyclic) | shrunk or split (4 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| next.js #1: `build, build/adapter, build/analysis, …` | 62 | shrunk or split (61 original modules remain cyclic) | shrunk or split (61 original modules remain cyclic) | shrunk or split (44 original modules remain cyclic) | shrunk or split (44 original modules remain cyclic) | shrunk or split (43 original modules remain cyclic) | shrunk or split (43 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. Fix3 directory exclusions remove 220 files in this original SCC; examples: `src/build/adapter/build-complete.ts`, `src/build/adapter/fallback-shell-runs.ts`, `src/build/adapter/setup-node-env.external.ts`. Next.js build code and Django testing APIs are real source excluded by directory convention; this is a scope change, not proof that those dependencies were erroneous. Fix5 verifies a cycle via grab-bag; residual real cycles are retained. See the complete/partial evidence table below. |
| next.js #2: `next-devtools, next-devtools/dev-overlay` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| excalidraw #1: `actions, animatedTrail.ts, clients.ts, …` | 29 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| immich #1: `constants.ts, cores, decorators.ts, …` | 9 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| immich #2: `maintenance, middleware, services` | 3 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| activepieces #1: `app, app/action-run, app/agents, …` | 32 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| payload #1: `admin, auth, checkPayloadDependencies.ts, …` | 21 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| typeorm #1: `driver, driver/aurora-mysql, driver/aurora-postgres, …` | 27 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| typeorm #2: `persistence, persistence/subject-builder` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| typeorm #3: `query-builder, query-builder/relation-id` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| vite #1: `module-runner, module-runner/sourcemap` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| vite #2: `node, node/optimizer, node/plugins, …` | 5 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| outline #1: `app, app/actions, app/components, …` | 40 | unchanged | unchanged | unchanged | unchanged | shrunk or split (11, 22, 5 original modules remain cyclic) | shrunk or split (11, 22, 5 original modules remain cyclic) |  Fix5 verifies a cycle via grab-bag; residual real cycles are retained. See the complete/partial evidence table below. |
| strapi #1: `configuration, core-api/routes, ee, …` | 5 | shrunk or split (3 original modules remain cyclic) | shrunk or split (3 original modules remain cyclic) | shrunk or split (3 original modules remain cyclic) | shrunk or split (3 original modules remain cyclic) | shrunk or split (3 original modules remain cyclic) | shrunk or split (3 original modules remain cyclic) | Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering. |
| strapi #2: `domain/module, registries` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| trpc #1: `unstable-core-do-not-import, unstable-core-do-not-import/error, unstable-core-do-not-import/http, …` | 5 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| django #1: `__init__.py, apps, conf, …` | 31 | unchanged | unchanged | shrunk or split (30 original modules remain cyclic) | shrunk or split (30 original modules remain cyclic) | shrunk or split (30 original modules remain cyclic) | shrunk or split (30 original modules remain cyclic) |  Fix3 directory exclusions remove 7 files in this original SCC; examples: `django/test/__init__.py`, `django/test/client.py`, `django/test/html.py`. Next.js build code and Django testing APIs are real source excluded by directory convention; this is a scope change, not proof that those dependencies were erroneous. |
| django #2: `contrib/admin, contrib/auth, contrib/contenttypes, …` | 4 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| fastapi #1: `__init__.py, _compat, applications.py, …` | 14 | unchanged | unchanged | unchanged | unchanged | shrunk or split (9 original modules remain cyclic) | shrunk or split (9 original modules remain cyclic) |  Fix5 verifies a cycle via grab-bag; residual real cycles are retained. See the complete/partial evidence table below. |
| pydantic #1: `_internal, config.py, errors.py, …` | 8 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| scrapy #1: `__init__.py, addons.py, core, …` | 18 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| scrapy #2: `cmdline.py, commands` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| celery #1: `app, canvas.py, concurrency, …` | 5 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| celery #2: `exceptions.py, platforms.py, utils` | 3 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| zulip #1: `actions, context_processors.py, data_import, …` | 14 | unchanged | unchanged | unchanged | persists in a larger cycle | persists in a larger cycle | persists in a larger cycle |  Fix4 changes type-only or resolved runtime links; exact counts and pinned source witnesses are listed in the transition table below. |
| sqlalchemy #1: `dialects, engine, sql` | 3 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |
| sqlalchemy #2: `exc.py, util` | 2 | unchanged | unchanged | unchanged | unchanged | unchanged | unchanged | Unchanged through measured stages. |

## Fix 4 runtime-link changes behind SCC membership changes

For every original SCC whose disposition changes between fixes 3 and 4, this table groups its old runtime edges by their actual new kind or changed target, plus new runtime links touching its modules. These are edge-level explanations, not reasons guessed from a lower cycle count. Added links can expand SCCs; they are not counted as removed edges. The complete inventory is reproducible with `transition_kinds()` and the archived JSON.

| Original SCC | Fix3 → fix4 disposition | Changed runtime links | Source witnesses |
| --- | --- | --- | --- |
| zulip #1 | unchanged → persists in a larger cycle | new or redirected runtime link: 330 | [zerver/actions/create_realm.py:11](https://github.com/zulip/zulip/blob/fb4f7abc378de7308a572d9af375101707e0c1ee/zerver/actions/create_realm.py#L11) (new or redirected runtime link; target `confirmation/settings.py`)<br>[analytics/lib/counts.py:23](https://github.com/zulip/zulip/blob/fb4f7abc378de7308a572d9af375101707e0c1ee/analytics/lib/counts.py#L23) (new or redirected runtime link; target `zerver/lib/timestamp.py`) |

## Grab-bag candidate calibration (corrected)

The chosen defaults (cohesion <0.2, weighted fan-in ≥5, raw internal edges/file <0.5, at least two files) select 26 candidates across all 23 repositories; 18 are currently cyclic. This is candidate calibration, not a claim that all are grab-bags. Benign contexts, enums and two-file barrel modules can satisfy the metrics. Reclassification still requires the fix5 split-graph proof.

| One threshold changed | Candidates across 23 repositories |
| --- | ---: |
| Chosen defaults | 26 |
| Cohesion <0.1 | 23 |
| Weighted fan-in ≥8 | 16 |
| Weighted fan-in ≥10 | 13 |
| Raw internal edges/file <0.25 | 17 |
| At least three files | 21 |

## Verified grab-bag reclassifications

A candidate metric is not a finding. Each record below is actual split-graph evidence. Complete reclassification leaves no residual module SCC; partial reclassification retains and reports real residual SCCs and their cut witnesses. These categories are not added to the number of removed SCCs.

| Repository | Original modules | Candidate modules | Result | Residual SCC sizes | File witnesses |
| --- | ---: | --- | --- | --- | --- |
| nest | 6 | `packages/common` (cohesion 0.0, weighted fan-in 39.9, internal/file 0.0000)<br>`packages/common/utils` (cohesion 0.0361, weighted fan-in 46.0, internal/file 0.0667) | partial; real residual cycles retained | 5 | [packages/common/decorators/core/catch.decorator.ts:1](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/decorators/core/catch.decorator.ts#L1) → `packages/common/constants.ts`<br>[packages/common/decorators/core/controller.decorator.ts:1](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/decorators/core/controller.decorator.ts#L1) → `packages/common/constants.ts`<br>[packages/common/decorators/core/controller.decorator.ts:9](https://github.com/nestjs/nest/blob/1c2f689518412cbc6a6bc09662dd5cfed779f2bf/packages/common/decorators/core/controller.decorator.ts#L9) → `packages/common/utils/shared.utils.ts` |
| next.js | 44 | `client/lib` (cohesion 0.0, weighted fan-in 5.0, internal/file 0.0000)<br>`telemetry/events` (cohesion 0.0, weighted fan-in 5.3, internal/file 0.0000) | partial; real residual cycles retained | 43 | [packages/next/src/cli/next-upgrade.ts:23](https://github.com/vercel/next.js/blob/00c4ba8b6e268c792da64207fa9453132bfcec5f/packages/next/src/cli/next-upgrade.ts#L23) → `src/telemetry/events/agent-upgrade.ts`<br>[packages/next/src/client/components/app-router-instance.ts:24](https://github.com/vercel/next.js/blob/00c4ba8b6e268c792da64207fa9453132bfcec5f/packages/next/src/client/components/app-router-instance.ts#L24) → `src/client/lib/javascript-url.ts`<br>[packages/next/src/client/components/app-router-state.ts:41](https://github.com/vercel/next.js/blob/00c4ba8b6e268c792da64207fa9453132bfcec5f/packages/next/src/client/components/app-router-state.ts#L41) → `src/client/lib/javascript-url.ts` |
| outline | 40 | `app` (cohesion 0.0426, weighted fan-in 16.2, internal/file 0.3333)<br>`server/commands` (cohesion 0.031, weighted fan-in 10.1, internal/file 0.2083)<br>`server/middlewares` (cohesion 0.0348, weighted fan-in 17.0, internal/file 0.1053)<br>`shared` (cohesion 0.0, weighted fan-in 31.0, internal/file 0.0000)<br>`shared/helpers` (cohesion 0.0, weighted fan-in 7.2, internal/file 0.0000) | partial; real residual cycles retained | 11, 22, 5 | [app/actions/definitions/apiKeys.tsx:5](https://github.com/outline/outline/blob/095b096c2197f35e57057e6cc6ae9d7ef562330a/app/actions/definitions/apiKeys.tsx#L5) → `app/env.ts`<br>[app/actions/definitions/developer.tsx:18](https://github.com/outline/outline/blob/095b096c2197f35e57057e6cc6ae9d7ef562330a/app/actions/definitions/developer.tsx#L18) → `app/env.ts`<br>[app/actions/definitions/documents.tsx:116](https://github.com/outline/outline/blob/095b096c2197f35e57057e6cc6ae9d7ef562330a/app/actions/definitions/documents.tsx#L116) → `app/env.ts` |
| fastapi | 14 | `openapi` (cohesion 0.1111, weighted fan-in 5.0, internal/file 0.4000) | partial; real residual cycles retained | 9 | [fastapi/_compat/v2.py:19](https://github.com/fastapi/fastapi/blob/0b919816378eaa773df7ce4204414c712c9fdbea/fastapi/_compat/v2.py#L19) → `fastapi/openapi/constants.py`<br>[fastapi/applications.py:17](https://github.com/fastapi/fastapi/blob/0b919816378eaa773df7ce4204414c712c9fdbea/fastapi/applications.py#L17) → `fastapi/openapi/docs.py`<br>[fastapi/applications.py:22](https://github.com/fastapi/fastapi/blob/0b919816378eaa773df7ce4204414c712c9fdbea/fastapi/applications.py#L22) → `fastapi/openapi/utils.py` |

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

## Final interpretation

The published cohort has 0 complete and 4 partial grab-bag reclassifications. Real residual runtime SCCs remain reported. SCC count can rise when one component splits: Outline changes from one 40-module SCC to components of 11, 22 and five modules. The aggregate component count is not a monotonic quality score.

Through fix4, no additional baseline runtime SCC was fully removed solely by type-only refinement, test exclusions or resolver changes. The complete baseline Fastify SCC disappearance is the structural re-export policy change; shrinkage and the Zulip expansion are listed individually above. Low-confidence exclusions limit runtime-coverage claims.

The archived fix5 graphs preserve exactly the fix4 input arrays. The corrected final stage changes Python resolution inputs as documented above; removing synthetic paths is a collector correctness fix, not an architectural improvement.

## Verification and reproduction

All 23 pinned repositories completed baseline and fixes 1–5, plus any separately listed correction stage. Report generation checks full-cohort coverage, manifest commits, clean scan provenance and archived graph hashes. README generation uses the latest complete verified final cohort, including the corrected stage when present, and checks that each suggested cut is an existing runtime edge.

```sh
python3 -B -m unittest discover -s showcase/tests
python3 -B showcase/edge_quality.py --report-output showcase/edge-quality-report.md
python3 -B showcase/edge_quality.py --readme-output README.md
```

## Open questions and next steps

- Broad directory defaults intentionally change scope. Layout include overrides restore Next.js build implementation and its cycles. Including Django/test restores its public testing API files, but they remain test-classified and outside cycles. An explicit classification override for production APIs in test-named folders is future work, not an implemented option. Payload CLI build code is also excluded by convention.
- Unresolved imports and low-confidence re-export metadata are a visible backlog. Improve the most common missing/alias cases with fixtures; dynamic imports and dependency injection still limit coverage.
- The checked Nest interface-only import is now type, while mixed imports retain runtime bindings. This verifies the concrete counterexamples; a lightweight tokenizer is not a complete TypeScript type checker.
- Candidate thresholds alone do not prove a grab-bag. Keep split-graph verification and real residual-cycle witnesses when extending calibration to other repositories.
