# Labelled service flows

This benchmark measures exact, independently labelled source callsites. It does not measure
whole-repository precision, runtime traffic, or whether two configured services actually share
a deployment. `service-flows.json` is the source of truth for pins, locations, file hashes,
expected endpoints, configuration assumptions, and unresolved-pattern explanations.

The corpus separates applications from public reference programs:

| Local directory under `--repos` | Source | Commit | Cohort |
| --- | --- | --- | --- |
| `immich` | `immich-app/immich` | `3ae71117413cc0a79e4a0555df015e3ffde11786` | Application, two HTTP flows |
| `activepieces` | `activepieces/activepieces` | `55134042a88da01be2229e06302a78c80f209ea0` | Application, six HTTP flows |
| `outline` | `outline/outline` | `095b096c2197f35e57057e6cc6ae9d7ef562330a` | Same-service queue controls |
| `new-corpus/nats.go` | `nats-io/nats.go` | `6068694d76fcf1f12bdf1e7383876aa5a3928421` | Official reference, two queue flows |
| `new-corpus/nats-pubsub` | `rafian-git/Nats-PubSub` | `e642c0c36ffcce1274e8fc56fa8213eac2ee4760` | Community reference, one queue flow |

The official NATS cohort scans `examples/docs`. A generated manifest selects its four labelled
program directories and declares **zero edges**. Other examples are excluded from this cohort,
so common subjects in unrelated example programs do not create unreviewed cross-example links.
No manifest supplies an expected call.

Prepare local source checkouts at those commits, or extract their pinned source archives into
the listed directories. The runner verifies Git HEAD when a `.git` entry is present, plus every annotated file hash
and source witness. It records `git_head_and_annotated_files`, without claiming whole-tree
cleanliness. For archives it records `annotated_files_only`. Neither mode claims to have
revalidated unannotated files. The runner avoids Git status and its configurable filters/hooks. Source repositories remain read-only, and their programs are not executed.

```sh
cargo build --release
python3 -B -m unittest discover -s benchmarks -p test_service_flows.py
python3 -B benchmarks/service_flows.py \
  --archgraph target/release/archgraph \
  --repos /private/tmp/archgraph-app-model-validation \
  --output /private/tmp/archgraph-app-model-validation/flow-candidate.json
```

The runner freezes a copy of the binary before scanning. The output records the binary and truth
hashes, source verification level, exact commands, per-repository results, missed labels, and
raw prediction reports. Artifact paths must be outside the checkout and source roots, including
when existing paths are symlinks. Observed scan seconds are informational single-run wall times,
not a comparative performance benchmark.

## Reading the results

- A true positive matches repository, caller file and line, source service, destination service,
  protocol, and operation. It is counted once even if repeated in the report.
- An incorrect prediction at a labelled positive callsite is a false positive. A prediction that
  violates a labelled negative control is also a false positive.
- Other predictions require manual review. They are reported separately and are never counted as
  verified true positives or whole-repository false positives.
- Precision applies only to labelled predictions. With no labelled prediction it is `null`, not
  a claim of perfect accuracy. Recall includes every labelled positive flow, including misses.
- `all_labelled_flows_recovered` describes coverage only. False positives, violated controls and
  unreviewed predictions remain separate, and can still exist when coverage is complete.
  Check the per-repository cohorts before using the aggregate: successful reference programs do
  not prove application HTTP coverage.

Activepieces' API owns its BullMQ workers. Its separate worker process polls through Socket.IO
RPC. That unsupported transport is documented as a control against inventing an API-to-worker
queue edge. Outline's worker is a role within the `server` service. Neither is a positive
cross-service queue sample.

The unit tests seed missing edges, wrong destinations with unchanged totals, duplicates,
external-call misattribution, unreviewed predictions, stale/missing evidence, declared-edge
injection, and artifact-path escapes. A runner test changes the original binary during the run
and proves both repositories still use the same frozen version.

## A1 measured result

The source-labelled cohort contains eight application HTTP flows, two official NATS reference
flows, and one community NATS reference flow. The candidate adds immutable local Go topic aliases.

| Cohort | Baseline | Candidate |
| --- | ---: | ---: |
| Application HTTP | 0 / 8 | 0 / 8 |
| Official NATS references | 2 / 2 | 2 / 2 |
| Community NATS reference | 0 / 1 | 1 / 1 |
| Total labelled flows | 2 / 11 | 3 / 11 |

Both runs have zero false positives on labelled sites, zero unreviewed predictions, and six
controls with no violations. The eight application misses remain in the output with their
configuration and wrapper explanations. This is a coverage gap, not proof that the applications
have no service calls. No runtime traffic was observed.

Baseline source is commit `a086e2c`. Both final runs use frozen binaries and the same truth hash
`44d497807e36de0043fcafae5d367ede5d790bcfec51f74caf341c67813309e3`.
Baseline binary SHA-256 is `e620961cc915976b1ee72e4b3d7afa5fcb076268b770df41faf7addabe95cc33`.
Candidate binary SHA-256 is `9020769125576ed6e9b5cb010612c23f743fc9725e73ea9db1c10f0edae5753b`.
The frozen reports and binaries are under `/private/tmp/archgraph-app-model-validation/` in
`a1-baseline-final*` and `a1-candidate-final*`. Those scratch artifacts are not required by CI.
