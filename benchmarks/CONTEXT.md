# Context pruning: held-out PR files

This is a pre-registered retrieval experiment, not a measure of agent correctness or developer productivity. The experiment freezes its PRs and holdouts before running either ranker. Results below use the frozen 30-PR sample and unchanged ranking parameters.

These results archive the earlier path-seeded selector at the measured commit below. The current selector also searches source text and considers seed-folder neighbours; rerunning this already inspected sample is exploratory evidence, not a fresh held-out evaluation of the combined ranking.

## Sampling and leakage controls

Use exactly the first 20 repositories in `showcase/repos.json`, with their declared language, scanned subroot and exclusion flags. Supplemental Go repositories are outside this experiment. Collect the first 500 closed PRs returned by GitHub's updated-descending endpoint; retain merged PRs and sort them by merge time, then number, descending. Take the first eligible PR for each repository and a second eligible PR for the first ten manifest entries. This is a deterministic convenience sample of recent eligible changes, not a random sample of all PRs.

Eligibility requires at least two modified or renamed source files that exist in the PR's **base** graph as production files. Added/deleted files, files outside the manifest root, excluded tests/generated/support files, pure barrels and literal `index.ts` files cannot be seeds or the holdout. Record all changed paths and candidate rejections, including scan failures. For renames use the old, base-existing path. Never replace a case after seeing its retrieval result.

Scan each selected PR's recorded base SHA in a separate clean checkout. Require that base to be a Git ancestor of the PR merge commit; reject API bases that fail this historical-order check. Never use the showcase's later pinned graph, the PR head graph, diff contents or PR body to predict the change. The task is the PR title, unmodified. Sort eligible paths; choose one holdout by the integer SHA-256 of `owner/repository/PR-number` modulo their count. The remaining paths are seeds. There is one observation per PR, not one per file. The frozen case inventory records all choices, source commits, scan commands and graph hashes.

The initial, unscored 100-closed-PR pilot yielded only 29 cases: SQLAlchemy had no eligible multi-production-file change in its 11 merged PRs. Before running either ranker, the candidate window was expanded uniformly to 500 closed PRs per repository. The original pilot and infrastructure retries are archived; no held-out retrieval outcome informed this amendment.

## Fixed comparison

Run the context command with its default ranking, a 40,000-token budget, the title and seeds. Also record 10,000 and 20,000-token budgets without tuning ranking parameters between runs. Count a hit when the held-out path occurs in the emitted context. Export-name summary entries count as retrieval (they are not complete signatures), so this measures file discovery rather than full-body availability.

The concrete “grep + open the folder” comparator considers the same production, non-barrel, non-`index.ts` base-graph files. It selects seeds first, then files with a title keyword in their case-insensitive path or text, then other files in a seed's immediate folder. Keywords are unique alphanumeric/underscore words of at least three characters, excluding the fixed generic stop list in the runner. Within groups, sort by descending keyword count, minimum folder distance to a seed folder, then path. Greedily pack whole files, skipping oversized entries. Use the same estimator as context: `ceil((UTF-8 path bytes + UTF-8 reason bytes + body bytes + 160)/4)`, with fixed reasons `explicit seed`, `grep task keywords`, or `same seed folder`. Reserve the same task overhead, `ceil((UTF-8 title bytes + 640)/4)`, and charge the same mandatory constraint entries before packing. The baseline receives no held-out identity in its ranking logic. Its content search has more information than the context command's path-keyword seeding.

Report raw numerators and denominators, failed cases separately, and token use. Thirty PRs are too few for confident repository-level or language-level claims. Sampling conditions favor multi-file production changes and exclude new-file discovery. Import omissions, unresolved imports, broad exclusions, and title wording affect retrieval. Neither method can prove that an agent has enough information to edit correctly. Worktree and graph paths in the inventory refer to local reproduction artifacts; clone and rescan the recorded immutable base commits to reproduce elsewhere.

## Reproduce

Requires `gh` read access to public GitHub repositories, Git, the standard collectors, and a Boundrail binary. The runner only writes its own `/private/tmp/boundrail-context-validation` clones, worktrees and API cache. When available, existing cached repository clones provide read-only Git object alternates; they are never reset or checked out. Otherwise the runner makes its own blob-filtered clone. Missing blobs are fetched in batches into its own object store. Preparation failures and retries occur before freezing the sample and are recorded separately.

```sh
python3 -B benchmarks/context_pruning.py select --binary "$PWD/target/release/boundrail"
python3 -B benchmarks/context_pruning.py score --binary "$PWD/target/release/boundrail"
python3 -B -m unittest discover -s benchmarks -p test_context_pruning.py
```

Selection and scoring are separate commands so the committed `context-prs.json` can be inspected before retrieval outcomes exist. The cached GitHub responses make selection replay stable; deleting that cache initiates a new sample and must not be presented as the same experiment.

<!-- RESULTS -->
## Results

One held-out existing production file per PR; export-name summaries count as discovery, not full-body reading or complete function signatures.

| Token budget | Context discovery | Context full body | Grep + folder | Context only / baseline only | Mean tokens (context / baseline) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 8/30 (26.7%) | 4/30 (13.3%) | 8/30 (26.7%) | 4 / 4 | 9,964 / 9,937 |
| 20,000 | 10/30 (33.3%) | 8/30 (26.7%) | 7/30 (23.3%) | 5 / 2 | 19,968 / 19,949 |
| 40,000 | 13/30 (43.3%) | 10/30 (33.3%) | 11/30 (36.7%) | 5 / 3 | 39,918 / 39,950 |

At the primary 40k budget, context discovered 13 held-out files versus 11 for the baseline. Context supplied full bodies for 10; 3 discoveries supplied export names only. The paired comparison has 5 context-only hits and 3 baseline-only hits. This small convenience sample does not establish general superiority.

Greedy packing can be nonmonotonic: a larger budget admits an earlier large file, which can displace a later held-out file. Report every budget rather than choosing the most favorable one.

The 30 selected PRs changed 780 paths: 230 were eligible existing production files, 60 were added, 2 removed; all remaining paths failed source/root/default-exclusion/barrel eligibility. Full reasons and all considered PRs are in [the frozen inventory](context-prs.json). Base graphs include the surface index, with no hotspot or co-change attachments; historical ranking boosts were not measured.

Measured source and executed harness commit: `d88519583ea5c1aa12147708dcbae9944eba8082`. The executed harness hash belongs to that commit; subsequent report wording changes do not rerun or alter rankings. The [machine-readable results](results/context-pruning.json) retain every selected path, token count and raw report hash. Source graphs, API responses and unabridged context reports are archived under the local experiment directory, with immutable commit IDs and commands recorded for reproduction.

| Repository / PR | Base | Held-out file | Context at 40k | Grep + folder |
| --- | --- | --- | --- | --- |
| [nest #18178](https://github.com/nestjs/nest/pull/18178) | [`1a7a31fa`](https://github.com/nestjs/nest/commit/1a7a31fa46756875e1dc33de0185edadb06504d9) | `packages/platform-express/adapters/utils/get-media-type-version.util.ts` | body | body |
| [nest #18166](https://github.com/nestjs/nest/pull/18166) | [`94fcc937`](https://github.com/nestjs/nest/commit/94fcc937706978c9305b2486f38643cfdf37bab9) | `packages/testing/testing-injector.ts` | miss | body |
| [next.js #99706](https://github.com/vercel/next.js/pull/99706) | [`da8f4e92`](https://github.com/vercel/next.js/commit/da8f4e9229ef139f4307d6a3441b32c053fba3ad) | `src/client/components/segment-cache/scheduler.ts` | miss | miss |
| [next.js #100284](https://github.com/vercel/next.js/pull/100284) | [`da8f4e92`](https://github.com/vercel/next.js/commit/da8f4e9229ef139f4307d6a3441b32c053fba3ad) | `src/server/base-server.ts` | miss | miss |
| [excalidraw #12270](https://github.com/excalidraw/excalidraw/pull/12270) | [`4f310b79`](https://github.com/excalidraw/excalidraw/commit/4f310b799613d6b506a40e9334f40f941001f425) | `actions/actionClipboard.tsx` | body | miss |
| [excalidraw #12264](https://github.com/excalidraw/excalidraw/pull/12264) | [`2df993fb`](https://github.com/excalidraw/excalidraw/commit/2df993fb397dd6d64625908c9c10337727617f6f) | `components/HelpDialog.tsx` | miss | miss |
| [immich #32143](https://github.com/immich-app/immich/pull/32143) | [`2a8f8324`](https://github.com/immich-app/immich/commit/2a8f8324e43487f29fd475832107f279da1c39c3) | `src/emails/welcome.email.tsx` | body | body |
| [immich #32412](https://github.com/immich-app/immich/pull/32412) | [`d519140a`](https://github.com/immich-app/immich/commit/d519140a6c4d9bc8c392e08a29679ddf3326d917) | `src/services/shared-link.service.ts` | body | miss |
| [activepieces #16171](https://github.com/activepieces/activepieces/pull/16171) | [`03a136fc`](https://github.com/activepieces/activepieces/commit/03a136fcfbfaa5d460cc1cb54070de3aaa088697) | `src/app/ee/agent/prompt/agent-surface-notes.ts` | exports | body |
| [activepieces #16130](https://github.com/activepieces/activepieces/pull/16130) | [`41b59bba`](https://github.com/activepieces/activepieces/commit/41b59bba6cb1d476ee90b93c0d4fad1932a9e84f) | `src/app/database/database-connection.ts` | miss | miss |
| [payload #18755](https://github.com/payloadcms/payload/pull/18755) | [`629359d3`](https://github.com/payloadcms/payload/commit/629359d3033261fec2e2e47d5c3eb4d01843a151) | `src/uploads/getFileContentRequirement.ts` | miss | miss |
| [payload #18502](https://github.com/payloadcms/payload/pull/18502) | [`dca3520b`](https://github.com/payloadcms/payload/commit/dca3520bf2c1b371a1f46aeba5ebe9e86ceb01af) | `src/fields/hooks/beforeChange/promise.ts` | exports | miss |
| [typeorm #12812](https://github.com/typeorm/typeorm/pull/12812) | [`c64a1f05`](https://github.com/typeorm/typeorm/commit/c64a1f052fc39f6688b6b73b83d065d7147ba8bb) | `driver/postgres/PostgresDriver.ts` | miss | body |
| [typeorm #12423](https://github.com/typeorm/typeorm/pull/12423) | [`5fd98ad3`](https://github.com/typeorm/typeorm/commit/5fd98ad3bae24da394cd50d3c87b69c0f7064a70) | `driver/postgres/PostgresQueryRunner.ts` | miss | miss |
| [vite #23777](https://github.com/vitejs/vite/pull/23777) | [`bd325338`](https://github.com/vitejs/vite/commit/bd3253387fe1dc96831950f9894a456e9cbfe2f8) | `src/node/plugins/html.ts` | miss | miss |
| [vite #23746](https://github.com/vitejs/vite/pull/23746) | [`ce4293f1`](https://github.com/vitejs/vite/commit/ce4293f15cc5cd221c5ed60f5cd64e82ca5cbaf5) | `src/node/plugins/css.ts` | miss | miss |
| [outline #14125](https://github.com/outline/outline/pull/14125) | [`56476212`](https://github.com/outline/outline/commit/56476212241fd92a379dada3471642178f56d9ba) | `app/components/DocumentMeta.tsx` | body | miss |
| [outline #14124](https://github.com/outline/outline/pull/14124) | [`bd49f858`](https://github.com/outline/outline/commit/bd49f85819743af5df2d60a2d67a5be4750f94e2) | `server/routes/api/documents/documents.ts` | miss | miss |
| [strapi #27942](https://github.com/strapi/strapi/pull/27942) | [`5c8788f2`](https://github.com/strapi/strapi/commit/5c8788f24e851c632870fd96472cc356235a1bc6) | `services/document-service/components.ts` | exports | miss |
| [strapi #27671](https://github.com/strapi/strapi/pull/27671) | [`e8ae3dc8`](https://github.com/strapi/strapi/commit/e8ae3dc8ca0dbdc4ec9fa75c2afa318cdd25a188) | `loaders/plugins/get-enabled-plugins.ts` | miss | miss |
| [trpc #7659](https://github.com/trpc/trpc/pull/7659) | [`796700d8`](https://github.com/trpc/trpc/commit/796700d8653b3832409fff997e6f5789519b9ea7) | `src/unstable-core-do-not-import/stream/jsonl.ts` | body | body |
| [django #22132](https://github.com/django/django/pull/22132) | [`1a9fba7a`](https://github.com/django/django/commit/1a9fba7a6c2a0acead19bbb45c25ceb0fdcca7c8) | `django/forms/fields.py` | miss | miss |
| [fastapi #16476](https://github.com/fastapi/fastapi/pull/16476) | [`9269f175`](https://github.com/fastapi/fastapi/commit/9269f17524e7990c39a9a1708db532810ceffc06) | `fastapi/telemetry/_api.py` | body | body |
| [flask #6168](https://github.com/pallets/flask/pull/6168) | [`d73fa1cd`](https://github.com/pallets/flask/commit/d73fa1cdcbd8b1465c151db8924ba58b1dd14e35) | `src/flask/sessions.py` | miss | body |
| [pydantic #13997](https://github.com/pydantic/pydantic/pull/13997) | [`03de65fc`](https://github.com/pydantic/pydantic/commit/03de65fc2f44faa4b32e05ce0a5d499cdf38f89b) | `pydantic/type_adapter.py` | miss | miss |
| [httpx #3699](https://github.com/encode/httpx/pull/3699) | [`ca097c96`](https://github.com/encode/httpx/commit/ca097c96f97d8d2a5da09b8ca736c7e78a2467f6) | `httpx/__init__.py` | body | body |
| [scrapy #8330](https://github.com/scrapy/scrapy/pull/8330) | [`aaf2b31c`](https://github.com/scrapy/scrapy/commit/aaf2b31cabf2eca932dd30ca102a8cb450aa6348) | `scrapy/extensions/feedexport.py` | body | body |
| [celery #10837](https://github.com/celery/celery/pull/10837) | [`2fa4aa28`](https://github.com/celery/celery/commit/2fa4aa287865e0d7c79a783d82050a3f6744f589) | `celery/bin/celery.py` | body | body |
| [zulip #40117](https://github.com/zulip/zulip/pull/40117) | [`7eed14f6`](https://github.com/zulip/zulip/commit/7eed14f6166b53c10228900be047387346f95414) | `zilencer/views.py` | miss | miss |
| [sqlalchemy #13342](https://github.com/sqlalchemy/sqlalchemy/pull/13342) | [`4e97e0ab`](https://github.com/sqlalchemy/sqlalchemy/commit/4e97e0abaf5c980ff04558323974c72757baa46f) | `sqlalchemy/dialects/postgresql/base.py` | miss | miss |
