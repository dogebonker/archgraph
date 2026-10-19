# Predeclared public synthetic cases

These four tasks were selected before any model runs. Flask and tRPC are showcase
repos; HTTPX and Pydantic are public substitutes because Perf's repositories and
tasks were unavailable. Pins and public URLs come from `showcase/repos.json`.
These small utility changes test feature completion and benchmark plumbing; they
are not designed to force an architecture violation or demonstrate efficacy.

| Repo | Requested feature | Expected implementation area |
| --- | --- | --- |
| Flask | Strip whitespace before interpreting the dotenv environment flag | `src/flask/helpers.py` |
| HTTPX | Convert bytearray to immutable bytes in `to_bytes` | `httpx/_utils.py` |
| Pydantic | Optional first-value retention in `unique_list` | `pydantic/_internal/_utils.py` |
| tRPC | Require a callable async iterator member | `packages/server/src/unstable-core-do-not-import/utils.ts` |

Prepare four private baselines and their `NAME-tasks.json` files:

```sh
python3 -B bench/cases/prepare.py --sources /path/to/pinned/clones --output /private/tmp/archgraph-cases --python /path/to/python3.13 --node /path/to/node24
```

Preparation checks each source HEAD against its declared pin, clones with
`--no-hardlinks`, checks out that exact committed tree, and refuses existing
output repos. Local uncommitted upstream changes are not copied. Each baseline
gets a deterministic local setup commit containing a benchmark-authored
utility-to-outer-API import boundary, a provenance record and an oracle copy.
This policy is not a claim about upstream architectural intent. The manifest
records the upstream and baseline hashes and the policy digest. Source repos
and indexes remain unchanged.

The generated task invokes the canonical `check.py` outside the agent checkout;
the committed copy is provenance only. Freeze the external oracle digest before
and after a report. Python cases compile the selected function AST with deferred
annotations and a few standard-library globals, avoiding package dependencies.
They check isolated utility behavior, not imports, integration or static typing.
The tRPC check imports the complete utility module using Node 24 type stripping.
These checks are deliberately labeled isolated feature oracles, not upstream
suites. Tasks preserve existing behavior through independent positive and
negative examples; no expected implementation text is embedded in the prompts.

Before model runs, each oracle was observed to fail its pristine baseline for
the missing behavior and pass a minimal implementation in a discarded clone.
The prepared baselines remained clean. The preparation self-check is:

```sh
python3 -B -m unittest discover -s bench/cases -p 'test_*.py'
```

Use each generated task file with `bench/run.py --repo OUTPUT/NAME`; retain all
three repetitions in both arms, including failures and zero-benefit results.
