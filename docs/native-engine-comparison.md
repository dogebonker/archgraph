# Native engine comparison

Generated from actual scans at the showcase manifest pins. Reference compatibility is not a correctness score. The default remains the reference engine.

20/20 repositories scanned successfully; 20/20 exceed 98% endpoint precision and recall. Sum of per-repository median scan times: reference 135.969s, native 23.259s.

Binary SHA-256: `05047bd564dc5565e9852c49f0df0e79a353e0497586732aabbaf9811896e3bf`. Cargo.lock SHA-256: `f28deb4022b0871e793eb464ab20a3a0afce6724f7c172396703d4d1dbc9f370`.

Timing: 2 end-to-end scan(s) per engine, median seconds; parsing, resolution and analysis included. No isolated parser speed claim. Local caches, run order and concurrent validation on the development host affect wall time; these timings are observational.

| Repository | Files ref/native | Internal edges ref/native | Precision / recall | Shared-file P/R | Unresolved ref/native | Cycles ref/native | Largest ref/native | Seconds ref/native | >98% both |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| nest | 666/669 | 2545/2545 | 100.00% / 100.00% | 100.00% / 100.00% | 4/4 | 5/4 | 15/14 | 11.545/0.747 | yes |
| next.js | 1252/1252 | 4624/4624 | 99.94% / 99.94% | 99.94% / 99.94% | 348/353 | 2/2 | 43/43 | 25.262/4.675 | yes |
| excalidraw | 364/364 | 1479/1479 | 100.00% / 100.00% | 100.00% / 100.00% | 4/5 | 1/1 | 29/29 | 5.243/1.762 | yes |
| immich | 458/458 | 2053/2053 | 100.00% / 100.00% | 100.00% / 100.00% | 1/1 | 2/2 | 9/7 | 4.579/0.717 | yes |
| activepieces | 757/757 | 3100/3100 | 100.00% / 100.00% | 100.00% / 100.00% | 2/2 | 1/1 | 32/32 | 10.745/1.731 | yes |
| payload | 817/817 | 3855/3855 | 100.00% / 100.00% | 100.00% / 100.00% | 10/65 | 1/1 | 21/21 | 4.310/1.117 | yes |
| typeorm | 496/496 | 2598/2598 | 100.00% / 100.00% | 100.00% / 100.00% | 4/4 | 3/3 | 27/27 | 3.912/1.514 | yes |
| vite | 135/135 | 762/762 | 100.00% / 100.00% | 100.00% / 100.00% | 13/14 | 2/2 | 5/5 | 3.029/0.706 | yes |
| outline | 2231/2231 | 11456/11456 | 100.00% / 100.00% | 100.00% / 100.00% | 62/62 | 3/3 | 22/22 | 15.624/2.972 | yes |
| strapi | 236/236 | 364/364 | 100.00% / 100.00% | 100.00% / 100.00% | 6/275 | 2/2 | 3/3 | 2.846/0.810 | yes |
| trpc | 76/76 | 298/298 | 100.00% / 100.00% | 100.00% / 100.00% | 2/2 | 1/1 | 5/5 | 1.309/0.343 | yes |
| django | 706/706 | 3112/3110 | 99.87% / 99.81% | 99.87% / 99.81% | 130/154 | 2/2 | 30/30 | 8.285/1.015 | yes |
| fastapi | 52/52 | 126/126 | 100.00% / 100.00% | 100.00% / 100.00% | 188/189 | 1/1 | 9/9 | 0.832/0.228 | yes |
| flask | 24/24 | 94/94 | 100.00% / 100.00% | 100.00% / 100.00% | 89/92 | 0/0 | 0/0 | 0.530/0.245 | yes |
| pydantic | 79/79 | 362/362 | 100.00% / 100.00% | 100.00% / 100.00% | 142/147 | 1/1 | 8/8 | 3.611/0.707 | yes |
| httpx | 23/23 | 87/87 | 100.00% / 100.00% | 100.00% / 100.00% | 29/30 | 0/0 | 0/0 | 0.590/0.157 | yes |
| scrapy | 194/196 | 980/980 | 100.00% / 100.00% | 100.00% / 100.00% | 314/328 | 2/2 | 18/18 | 2.965/0.513 | yes |
| celery | 161/161 | 616/616 | 100.00% / 100.00% | 100.00% / 100.00% | 413/429 | 2/2 | 5/5 | 2.047/0.281 | yes |
| zulip | 775/772 | 3866/3867 | 99.97% / 100.00% | 99.97% / 100.00% | 2103/2127 | 1/1 | 19/19 | 20.693/2.211 | yes |
| sqlalchemy | 240/240 | 2374/2375 | 98.95% / 98.99% | 98.95% / 98.99% | 106/108 | 2/2 | 3/3 | 8.013/0.810 | yes |

## Scope and disagreements

All-file metrics include missing files, so dropping files cannot inflate recall. Shared-file metrics are diagnostic only. Endpoints are deduplicated directed internal file edges; kind/`original_to` agreement is recorded separately in JSON. Known external packages are excluded from the internal-edge score, but unresolved occurrences are retained.

Every differing edge and its available source line/specifier is retained as unclassified evidence in each `results/<repo>/comparison.json`, beside raw graphs and scan logs. Causes require source inspection; a missing edge is not automatically a native bug. Scope changes, unknown resolution and unreviewed differences block a default flip even when the numeric gate passes. [Source review notes](native-engine-review.md) explain representative cases and remaining limits.

- **nest**: 0 missing / 3 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 93.40%, recall 93.40%.
- **next.js**: 0 missing / 0 extra files; 3 reference-only / 3 native-only edges, unclassified. Kind/original-target precision 99.27%, recall 99.27%.
- **excalidraw**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **immich**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 90.50%, recall 90.50%.
- **activepieces**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 99.77%, recall 99.77%.
- **payload**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 99.12%, recall 99.12%.
- **typeorm**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 99.96%, recall 99.96%.
- **vite**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **outline**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 99.90%, recall 99.90%.
- **strapi**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 98.08%, recall 98.08%.
- **trpc**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **django**: 0 missing / 0 extra files; 6 reference-only / 4 native-only edges, unclassified. Kind/original-target precision 99.84%, recall 99.78%.
- **fastapi**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 97.66%, recall 97.66%.
- **flask**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **pydantic**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **httpx**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **scrapy**: 0 missing / 2 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **celery**: 0 missing / 0 extra files; 0 reference-only / 0 native-only edges, unclassified. Kind/original-target precision 100.00%, recall 100.00%.
- **zulip**: 3 missing / 0 extra files; 0 reference-only / 1 native-only edges, unclassified. Kind/original-target precision 99.95%, recall 99.98%.
- **sqlalchemy**: 0 missing / 0 extra files; 24 reference-only / 25 native-only edges, unclassified. Kind/original-target precision 96.55%, recall 97.07%.

## Reproduce

```sh
python3 -B benchmarks/native_engine.py --archgraph /private/tmp/archgraph-native-comparison-v4/archgraph --work /private/tmp/archgraph-native-comparison-v4 --cache /private/tmp/archgraph-edge-quality/repos --depcruise /private/tmp/archgraph-impact-collectors/node_modules/.bin/depcruise --python /private/tmp/archgraph-impact-python/bin/python --jobs 2 --repeat 2 --order reference-first
```

Full machine-readable manifest, commands, collector metadata and hashes: `/private/tmp/archgraph-native-comparison-v4/comparison.json`.
