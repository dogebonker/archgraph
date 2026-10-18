# Native input-quality regression comparison

Same actual working trees; no source edits or recipes applied. Frozen binary copies identify each engine. Times include native collection, Git co-change/hotspot work and analysis; repeated scans alternate engine order and benefit from OS caches.

**Grouping:** MoneyLust backend uses old `--depth 3` versus new `--depth 2` because Rust now normalizes the `src/` root. A separate saved-graph render applies identical old physical-folder module rules to both graphs; this comparison layout is a measurement device, not an approved architecture policy.

Raw graphs, source paths, recipe buffers and logs remain in the private artifact directory. Source/index proofs cover Git-visible tracked and nonignored untracked files, modes and symlinks; `.archgraph/`, ignored install/build artifacts and submodule contents are outside the reused preservation check.

Before binary SHA-256: `d7f1052b607b810df9c5faa680ee41ed70bc06691e89f2d459e515fd03205421`. After: `2593bf84da1f0a5550d3e961cc91b930e6781d2f717d099e3f479d5aa69f75d6`.

| Scan | Engine | Files | Modules | Internal edges | Resolved % | Module SCCs / largest | Deep imports | Median seconds (min–max) |
| --- | --- | ---: | ---: | ---: | ---: | --- | ---: | --- |
| glarity-backend-symbols | before | 370 | 77 | 1565 | 89.2 | 5 / 9 | 1225 | 21.014 (20.886–29.723) |
| glarity-backend-symbols | after | 370 | 77 | 1565 | 100.0 | 5 / 9 | 129 | 4.348 (3.800–5.728) |
| glarity-backend-no-symbols | before | 370 | 77 | 1565 | 89.2 | 5 / 9 | 0 | 2.097 (2.046–2.135) |
| glarity-backend-no-symbols | after | 370 | 77 | 1565 | 100.0 | 5 / 9 | 0 | 2.238 (2.148–2.408) |
| moneylust-backend | before | 943 | 38 | 3923 | 99.4 | 1 / 36 | 0 | 4.561 (4.470–4.566) |
| moneylust-backend | after | 798 | 38 | 3015 | 99.8 | 1 / 27 | 0 | 6.693 (6.663–6.706) |
| moneylust-frontend | before | 1106 | 83 | 3578 | 100.0 | 0 / 0 | 0 | 1.205 (1.144–1.357) |
| moneylust-frontend | after | 1106 | 83 | 3578 | 100.0 | 0 / 0 | 0 | 1.195 (1.150–1.197) |

| Scan | Engine | Imports / unresolved | Edge kinds | File SCCs / largest | Findings / diagnostics |
| --- | --- | ---: | --- | --- | --- |
| glarity-backend-symbols | before | 3144 / 338 | lazy: 150, reexport: 44, runtime: 1357, type: 14 | 0 / 0 | 1232 / 63 |
| glarity-backend-symbols | after | 3144 / 0 | lazy: 150, reexport: 44, runtime: 1357, type: 14 | 0 / 0 | 137 / 62 |
| glarity-backend-no-symbols | before | 3144 / 338 | lazy: 150, reexport: 44, runtime: 1357, type: 14 | 0 / 0 | 8 / 62 |
| glarity-backend-no-symbols | after | 3144 / 0 | lazy: 150, reexport: 44, runtime: 1357, type: 14 | 0 / 0 | 8 / 62 |
| moneylust-backend | before | 9259 / 53 | reexport: 313, runtime: 3610 | 96 / 18 | 101 / 1 |
| moneylust-backend | after | 6800 / 12 | reexport: 807, runtime: 2208 | 14 / 7 | 16 / 2 |
| moneylust-frontend | before | 4287 / 0 | lazy: 21, reexport: 410, runtime: 2738, type: 409 | 0 / 0 | 6 / 3 |
| moneylust-frontend | after | 4287 / 0 | lazy: 21, reexport: 410, runtime: 2738, type: 409 | 0 / 0 | 6 / 3 |

| Same-folder Rust comparison | Modules | Module SCCs | Largest |
| --- | ---: | ---: | ---: |
| before | 38 | 1 | 36 |
| after | 35 | 1 | 27 |

## Preservation hashes

| Checkout | Source SHA-256 | Git index SHA-256 | Unchanged |
| --- | --- | --- | --- |
| glarity | `1860c625d6540625e4c442802bf55a64146a510625521be2c50801501b1813df` | `35c8b249853ca00e02d5c5fdf7049856a1234e3098841d2562765b82de8f69ec` | True |
| moneylust | `33b578c88b9bc969a3e59cdee8b335db5019b5f7d77860e74167d179b6dfcd85` | `b1b3754e6ad2abf195e8ac0019f07685768be77607ba188cedc15256717fa9a8` | True |

## Commands

Each scan is repeated with both frozen binaries. Placeholders keep local paths private.

```sh
<before-binary> --no-daemon scan <glarity> --engine native --language python --package bitxchange --cochange=300 --hotspots --hotspot-authors --symbols -o <artifacts>/glarity-backend-symbols-before-1.json
<after-binary> --no-daemon scan <glarity> --engine native --language python --package bitxchange --cochange=300 --hotspots --hotspot-authors --symbols -o <artifacts>/glarity-backend-symbols-after-1.json
<before-binary> --no-daemon scan <glarity> --engine native --language python --package bitxchange --cochange=300 --hotspots --hotspot-authors -o <artifacts>/glarity-backend-no-symbols-before-1.json
<after-binary> --no-daemon scan <glarity> --engine native --language python --package bitxchange --cochange=300 --hotspots --hotspot-authors -o <artifacts>/glarity-backend-no-symbols-after-1.json
<before-binary> --no-daemon scan <moneylust>/backend --engine native --language rust --depth 3 -o <artifacts>/moneylust-backend-before-1.json
<after-binary> --no-daemon scan <moneylust>/backend --engine native --language rust --depth 2 -o <artifacts>/moneylust-backend-after-1.json
<before-binary> --no-daemon render <artifacts>/moneylust-backend-before-1.json --rules <artifacts>/rust-same-boundaries.json -o <artifacts>/rust-same-boundaries-before.html
<after-binary> --no-daemon render <artifacts>/moneylust-backend-after-1.json --rules <artifacts>/rust-same-boundaries.json -o <artifacts>/rust-same-boundaries-after.html
<before-binary> --no-daemon scan <moneylust>/frontend --engine native --language typescript -o <artifacts>/moneylust-frontend-before-1.json
<after-binary> --no-daemon scan <moneylust>/frontend --engine native --language typescript -o <artifacts>/moneylust-frontend-after-1.json
```

Counts describe static dependency components, not runtime initialization cycles. A smaller SCC alone does not establish correctness; Rust test/containment and Python package/facade regressions are checked separately by fixture tests. No application build or production architecture-policy audit was performed.
