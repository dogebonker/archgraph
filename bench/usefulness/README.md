# Controlled usefulness benchmark

Four predeclared, standard-library Python mini-shop tasks: live regional tax,
reservation-aware inventory, canonical receipt statuses, and neutral discount/rounding.
The first three expose adapter, facade, and cycle boundaries; the neutral task needs
no boundary change. `fixture/layout.yaml` is deliberately authored synthetic policy,
not upstream truth. This is a controlled challenge, not evidence of real-world efficacy.

Preparation requires a fresh destination outside this repository:

```sh
python3 -B bench/usefulness/prepare.py --output /tmp/archgraph-usefulness
python3 -B -m unittest discover -s bench/usefulness -p 'test_usefulness.py' -v
```

Preparation copies only `fixture/` into a private Git baseline, fixes author/committer
identity and timestamp, and emits `tasks.json` plus `manifest.json` with the baseline
commit, Python path, canonical external oracle path, and source/task SHA-256 hashes.
The oracle and discarded reference implementations remain outside model workspaces.
Preparation refuses an existing destination. The self-check prepares twice to verify
deterministic commits, imports complete packages through the external oracle, and
uses the existing `target/debug/archgraph` (or `ARCHGRAPH_TEST_BINARY`) native engine with symbols and package
`shop`. It checks a clean baseline; each baseline fails its requested feature;
all four correct patches pass behavior and `check_diff`; three independent tempting
patches pass behavior but produce forbidden/public/cycle findings. Disposable patches
and Git commits live only in temporary directories. No models or APIs are called.

After prompt review, use **unchanged** `bench/run.py`:

```sh
python3 -B bench/run.py --repo /tmp/archgraph-usefulness/baseline \
  --tasks /tmp/archgraph-usefulness/tasks.json --runner /absolute/path/to/runner.json \
  --archgraph "$PWD/target/release/archgraph" --package shop --repeats 3 \
  --output /absolute/path/to/results
```

The existing harness independently clones the same baseline for every task/run/arm,
alternates arm order, and uses identical prompts and layout in both arms. Three repeats
per task per arm means 24 declared runs; preserve failures and exclusions in the
denominator. Review joint behavior-plus-architecture outcomes, neutral-task overhead,
usage, and run validity. The self-check proves challenge/oracle independence only;
it is not an observed agent outcome. Small synthetic tasks, visible task checks,
instruction-only restrictions on evaluator inspection, one local environment, and
three repeats limit inference. Architecture findings describe static imports, not
runtime correctness. There is no new parser, dependency, external configuration,
or secret in the mini-shop.

Primary outcome, declared before model runs: for each task and arm, use all three
planned runs. A usable completion requires agent exit 0, the external business oracle
passing, the expected area touched, unchanged policy, verified hook activation
(or verified absence in the bare arm), measured zero final violations, and no harness
errors. Failed, missing and unknown runs are non-successes. Pool the three boundary
tasks (nine runs per arm); report the neutral control separately (three per arm).
Show all-run time and token means with sample SD, including failures, and distinguish
UNAVAILABLE interruptions from rule-fire records. A control block requires inspecting
the actual edit against the frozen policy; it is not automatically a false positive.
This compares the hooks plus generated instructions bundle, not either component alone.
Any completion benefit must be presented with its measured cost, without a general
productivity or statistical-significance claim from three repetitions.

Produce the fixed-denominator summary after all runs:

```sh
python3 -B bench/usefulness/report.py RESULTS/baseline-DATE.json --output RESULTS/usefulness-summary
```
