# Hotspot validation: 20 repositories

Measured 2026-10-17. All 20 original showcase repositories completed, with full history and exact graph pins.

The highest-scoring 10% of files touched **3,794 of 5,253 unique fix-message commits (72.2%)**. The commit-size-adjusted random expectation is **19.6%**, giving **3.69×** association.

This is an in-sample association with commit-message keywords. It does not establish verified defects, future predictive power, refactoring benefit, or causality. Low-denominator projects such as HTTPX should not be interpreted individually.

| Repository | Pin | Files / top | Fix commits / top hits | Observed | Random | Ratio |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| [activepieces](https://github.com/activepieces/activepieces/tree/55134042a88da01be2229e06302a78c80f209ea0) | `55134042` | 757 / 76 | 675 / 468 | 69.3% | 19.3% | 3.59× |
| [celery](https://github.com/celery/celery/tree/ef37e616ef6726a2e08126635c000f3eacffcbc2) | `ef37e616` | 161 / 17 | 187 / 108 | 57.8% | 15.1% | 3.82× |
| [django](https://github.com/django/django/tree/d430084791a61b31cecec132bce3dca19194f7f2) | `d4300847` | 713 / 72 | 367 / 248 | 67.6% | 15.2% | 4.45× |
| [excalidraw](https://github.com/excalidraw/excalidraw/tree/4c37956ba5f04ec342a9d9d5891a38f22c5b7b4e) | `4c37956b` | 364 / 37 | 133 / 111 | 83.5% | 23.7% | 3.52× |
| [fastapi](https://github.com/fastapi/fastapi/tree/0b919816378eaa773df7ce4204414c712c9fdbea) | `0b919816` | 52 / 6 | 50 / 41 | 82.0% | 14.2% | 5.79× |
| [flask](https://github.com/pallets/flask/tree/d086db856be187255b8ec61ef409357393020f32) | `d086db85` | 21 / 3 | 6 / 5 | 83.3% | 14.3% | 5.83× |
| [httpx](https://github.com/encode/httpx/tree/b5addb64f0161ff6bfe94c124ef76f6a1fba5254) | `b5addb64` | 23 / 3 | 1 / 1 | 100.0% | 24.9% | 4.02× |
| [immich](https://github.com/immich-app/immich/tree/3ae71117413cc0a79e4a0555df015e3ffde11786) | `3ae71117` | 458 / 46 | 302 / 224 | 74.2% | 19.1% | 3.88× |
| [nest](https://github.com/nestjs/nest/tree/1c2f689518412cbc6a6bc09662dd5cfed779f2bf) | `1c2f6895` | 666 / 67 | 316 / 254 | 80.4% | 15.3% | 5.26× |
| [next.js](https://github.com/vercel/next.js/tree/00c4ba8b6e268c792da64207fa9453132bfcec5f) | `00c4ba8b` | 1542 / 155 | 575 / 469 | 81.6% | 23.6% | 3.45× |
| [outline](https://github.com/outline/outline/tree/095b096c2197f35e57057e6cc6ae9d7ef562330a) | `095b096c` | 2243 / 225 | 1229 / 908 | 73.9% | 22.8% | 3.25× |
| [payload](https://github.com/payloadcms/payload/tree/f177a26219996b75afc5d45cd07165001e9afd8c) | `f177a262` | 819 / 82 | 233 / 175 | 75.1% | 20.6% | 3.65× |
| [pydantic](https://github.com/pydantic/pydantic/tree/5adfeaaaea947f81f0f01868604f219084ec151a) | `5adfeaaa` | 79 / 8 | 47 / 26 | 55.3% | 12.4% | 4.45× |
| [scrapy](https://github.com/scrapy/scrapy/tree/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c) | `fbfc89ae` | 194 / 20 | 95 / 55 | 57.9% | 21.4% | 2.71× |
| [sqlalchemy](https://github.com/sqlalchemy/sqlalchemy/tree/262c0f0ef49a46fbc4c58df167a858d58313032b) | `262c0f0e` | 245 / 25 | 219 / 141 | 64.4% | 21.4% | 3.01× |
| [strapi](https://github.com/strapi/strapi/tree/d764743be322a70b98cb7b43c5d1a3e0d67d7efd) | `d764743b` | 236 / 24 | 83 / 57 | 68.7% | 15.6% | 4.40× |
| [trpc](https://github.com/trpc/trpc/tree/10e2ef98f7842b94eb3026e4f9bcfe62cc39b088) | `10e2ef98` | 83 / 9 | 23 / 12 | 52.2% | 16.0% | 3.27× |
| [typeorm](https://github.com/typeorm/typeorm/tree/fe92394287516feeada6e9c308ebe22deca2458e) | `fe923942` | 496 / 50 | 122 / 91 | 74.6% | 18.1% | 4.12× |
| [vite](https://github.com/vitejs/vite/tree/928e721b36690ac0f3f118290a21fc0cc4349055) | `928e721b` | 135 / 14 | 280 / 167 | 59.6% | 13.7% | 4.34× |
| [zulip](https://github.com/zulip/zulip/tree/fb4f7abc378de7308a572d9af375101707e0c1ee) | `fb4f7abc` | 733 / 74 | 310 / 233 | 75.2% | 17.1% | 4.39× |

The score uses log-normalized commits, normalized module coupling, module-cycle/adjacency membership, and the optional normalized author factor (enabled here). Commits touching more than 30 repository files and merge commits were excluded. Rename following uses Git similarity detection; vendor/generated exclusions come from each pinned scan. All normalization uses that scan's root.

The random comparison samples the same number of files uniformly without replacement. For each unique fix-message commit touching k scanned files among N, with T selected top files, its hit probability is `1 - product((N-T-i)/(N-i), i=0..k-1)` (1 if k exceeds N-T). This avoids comparing multi-file commits with a naive 10% baseline. Aggregate shares weight repositories by their unique eligible fix-message commit counts.

## Reproduce

The [runner](validate_hotspots.py) consumes an inventory of prepared graph files and full-history clones. The original 20 names, pins, subroots and exclusions are the first 20 entries in [showcase/repos.json](../showcase/repos.json). Graph URLs are recorded in [the result artifact](results/hotspots-validation.json), along with binary/raw-report hashes, per-repository statistics and three highest-scoring examples.

Each inventory entry needs `name`, `repo`, `commit`, `files`, `graph_path`, `repo_path`, `scan_root`, `pin_matches: true`, and `shallow: false`. Graphs can be downloaded from `https://dogebonker.github.io/archgraph/<name>.json`; verify `source.git.commit` against the manifest. Use full-history clones at that pin; a blob-filtered clone avoids initially downloading every historical file.

```sh
python3 benchmarks/validate_hotspots.py --coverage /tmp/coverage.json \
  --binary target/release/archgraph --out /tmp/hotspot-validation \
  --prefetch --timeout 240 --jobs 3
```

`--prefetch` batches missing partial-clone objects before scoring and leaves Git refs and working trees unchanged. A cold one-object-at-a-time log pass repeatedly timed out; bulk prefetch made all 20 scans possible. Preparation added at least 1.06 GB to Git object stores (not wire-byte traffic); warm execution times in the artifact exclude that earlier download cost. [Git's partial-clone documentation](https://git-scm.com/docs/partial-clone#_handling_missing_objects) describes the same demand-fetch cost and bulk strategy.

## Limits

- The ranking and fix-message counts use the same window, so high-churn files have an inherent advantage; holdout validation would be a separate claim.
- A fix-message keyword is a proxy. Squash messages, inconsistent conventions and unlabelled fixes affect coverage.
- Source graphs can omit dynamic, unresolved or generated dependencies. Collector unresolved counts are retained in the artifact.
- A commit touching a top file can also touch other files; unique commit counts avoid double-counting within each repository.
- The random baseline controls file counts per commit, not module structure, file size or historical churn.
- Timings are warm-data observations and include no project builds.
