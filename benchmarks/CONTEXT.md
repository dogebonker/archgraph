# Context pruning: held-out PR files

This is a pre-registered retrieval experiment, not a measure of agent correctness or developer productivity. The experiment freezes its PRs and holdouts before running either ranker. Results will be added only after all 30 observations run.

## Sampling and leakage controls

Use exactly the first 20 repositories in `showcase/repos.json`, with their declared language, scanned subroot and exclusion flags. Supplemental Go repositories are outside this experiment. Collect the first 500 closed PRs returned by GitHub's updated-descending endpoint; retain merged PRs and sort them by merge time, then number, descending. Take the first eligible PR for each repository and a second eligible PR for the first ten manifest entries. This is a deterministic convenience sample of recent eligible changes, not a random sample of all PRs.

Eligibility requires at least two modified or renamed source files that exist in the PR's **base** graph as production files. Added/deleted files, files outside the manifest root, excluded tests/generated/support files, pure barrels and literal `index.ts` files cannot be seeds or the holdout. Record all changed paths and candidate rejections, including scan failures. For renames use the old, base-existing path. Never replace a case after seeing its retrieval result.

Scan each selected PR's recorded base SHA in a separate clean checkout. Require that base to be a Git ancestor of the PR merge commit; reject API bases that fail this historical-order check. Never use the showcase's later pinned graph, the PR head graph, diff contents or PR body to predict the change. The task is the PR title, unmodified. Sort eligible paths; choose one holdout by the integer SHA-256 of `owner/repository/PR-number` modulo their count. The remaining paths are seeds. There is one observation per PR, not one per file. The frozen case inventory records all choices, source commits, scan commands and graph hashes.

The initial, unscored 100-closed-PR pilot yielded only 29 cases: SQLAlchemy had no eligible multi-production-file change in its 11 merged PRs. Before running either ranker, the candidate window was expanded uniformly to 500 closed PRs per repository. The original pilot and infrastructure retries are archived; no held-out retrieval outcome informed this amendment.

## Fixed comparison

Run the context command with its default ranking, a 40,000-token budget, the title and seeds. Also record 10,000 and 20,000-token budgets without tuning ranking parameters between runs. Count a hit when the held-out path occurs in the emitted context. Signature-only entries count as retrieval, so this measures file discovery rather than full-body availability.

The concrete “grep + open the folder” comparator considers the same production, non-barrel, non-`index.ts` base-graph files. It selects seeds first, then files with a title keyword in their case-insensitive path or text, then other files in a seed's immediate folder. Keywords are unique alphanumeric/underscore words of at least three characters, excluding the fixed generic stop list in the runner. Within groups, sort by descending keyword count, minimum folder distance to a seed folder, then path. Greedily pack whole files, skipping oversized entries. Use the same estimator as context: `ceil((UTF-8 path bytes + UTF-8 reason bytes + body bytes + 160)/4)`, with fixed reasons `explicit seed`, `grep task keywords`, or `same seed folder`. Reserve the same task overhead, `ceil((UTF-8 title bytes + 640)/4)`, and charge the same mandatory constraint entries before packing. The baseline receives no held-out identity in its ranking logic. Its content search has more information than the context command's path-keyword seeding.

Report raw numerators and denominators, failed cases separately, and token use. Thirty PRs are too few for confident repository-level or language-level claims. Sampling conditions favor multi-file production changes and exclude new-file discovery. Import omissions, unresolved imports, broad exclusions, and title wording affect retrieval. Neither method can prove that an agent has enough information to edit correctly. Worktree and graph paths in the inventory refer to local reproduction artifacts; clone and rescan the recorded immutable base commits to reproduce elsewhere.

## Reproduce

Requires `gh` read access to public GitHub repositories, Git, the standard collectors, and an archgraph binary. The runner only writes its own `/private/tmp/archgraph-context-validation` clones, worktrees and API cache. When available, existing cached repository clones provide read-only Git object alternates; they are never reset or checked out. Otherwise the runner makes its own blob-filtered clone. Missing blobs are fetched in batches into its own object store. Preparation failures and retries occur before freezing the sample and are recorded separately.

```sh
python3 -B benchmarks/context_pruning.py select --binary "$PWD/target/release/archgraph"
python3 -B benchmarks/context_pruning.py score --binary "$PWD/target/release/archgraph"
python3 -B -m unittest discover -s benchmarks -p test_context_pruning.py
```

Selection and scoring are separate commands so the committed `context-prs.json` can be inspected before retrieval outcomes exist. The cached GitHub responses make selection replay stable; deleting that cache initiates a new sample and must not be presented as the same experiment.
