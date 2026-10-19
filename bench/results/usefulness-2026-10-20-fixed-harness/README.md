# Controlled usefulness result

**No observed improvement in agent completion on these tasks, with higher time and token volume.**

The valid trial used four frozen Python mini-shop tasks, three repetitions per arm,
Codex 0.163.0 and `gpt-6.1-sol`. Three tasks exercised dependency direction, inventory
facade access and receipt cycles; a discount task was the neutral control. Both arms
received the same business requests and visible architecture policy. The guarded arm
added native edit hooks and generated rules. No warm daemon was started.

| Measure | Bare | Guarded |
| --- | ---: | ---: |
| Boundary tasks: usable completions | 9/9 | 9/9 |
| Neutral control: usable completions | 3/3 | 3/3 |
| Final architecture violations | 0 | 0 |
| Rule-fire records during agent work | 0 | 0 |
| Boundary agent time, mean ± sample SD | 19.5 ± 4.2 s | 35.1 ± 5.1 s |
| Boundary token volume, mean ± sample SD | 75,336 ± 5,638 | 139,866 ± 21,229 |
| Control agent time, mean ± sample SD | 15.3 ± 0.9 s | 34.8 ± 2.4 s |
| Control token volume, mean ± sample SD | 60,202 ± 313 | 127,090 ± 11,206 |

Boundary runs took about **80% more agent time and 86% more tokens** with the bundle.
All twelve guarded runs had verified native activation; all twelve bare runs had
verified absence. No unavailable-hook events or control rule blocks were observed.
These costs include model activity, guidance and hook work together; the trial does
not isolate each component's contribution. Tokens include cached input and are not
a dollar-cost estimate. Raw usage retains the cached-input subsets.

The independent preflight confirmed all four missing features initially failed;
four compliant reference implementations passed full-package business checks and
architecture checks. Three behavior-correct shortcuts triggered the intended forbid,
facade-bypass or cycle finding. That proves the detectors work on those authored
examples. The actual agent made none of those mistakes in either arm, so enforcement
had no opportunity to improve these final outcomes. This is not evidence that the
guard is useless in larger or less explicit repositories.

Usable completion was declared before outcomes: exit 0, passing external business
oracle, expected area touched, unchanged policy, verified arm activation/absence,
measured zero final violations and no harness errors. Every planned attempt remains
in its trial's denominator. No failed or unknown attempt was replaced inside a trial.

An [initial 24-attempt pass](../usefulness-2026-10-20/usefulness-summary.md) is retained
separately. Its twelve guarded attempts failed during benchmark setup, before any
agent turn: Git staging rejected the ignored `.archgraph/` pathspec. The harness's
setup and private-index measurement now stage normally and restore only tool-state
index entries to the baseline. Regression tests cover ignored/unignored state and
unchanged source indexes. All 24 attempts were then repeated as this separate trial;
tasks, oracle, policy, model, binary and scoring stayed frozen. Thus 48 planned
attempts are recorded in total, involving 36 actual agent turns.

[Fixed-denominator summary](usefulness-summary.md) ·
[Every valid-trial run and SVG chart](baseline-2026-10-19.md) ·
[Frozen provenance and verified hashes](provenance.json) ·
[Preparation, prompts, oracle and scoring](../../usefulness/README.md)

Small authored tasks, one repository, one model/host and three repetitions per task
limit inference. Both arms were explicitly told to follow the visible layout, so
this tests additional guard/guidance value against an already informed agent. These
results establish neither real-world effectiveness nor unconditional productivity.
Real feature PRs in a larger repository remain untested by this experiment.
