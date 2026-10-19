# Controlled usefulness test

agent exit 0; business oracle passes; expected area touched; policy unchanged; verified arm activation/absence; measured zero final violations; no harness errors.

Every planned run stays in the denominator; failed, missing and unverified runs are non-successes.

| Task/group | Bare success | Guarded success | Bare business pass | Guarded business pass | Guarded rule-fire records | Guarded UNAVAILABLE observed / runs unknown |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| boundary | 9/9 | 0/9 | 9/9 | 0/9 | 0 | 0 / 9 |
| control | 3/3 | 0/3 | 3/3 | 0/3 | 0 | 0 / 3 |
| tax | 3/3 | 0/3 | 3/3 | 0/3 | 0 | 0 / 3 |
| inventory | 3/3 | 0/3 | 3/3 | 0/3 | 0 | 0 / 3 |
| receipt | 3/3 | 0/3 | 3/3 | 0/3 | 0 | 0 / 3 |
| discount | 3/3 | 0/3 | 3/3 | 0/3 | 0 | 0 / 3 |

All-run costs (mean ± sample SD; unknown values are not imputed as zero):

| Group | Arm | Agent seconds | Tokens |
| --- | --- | ---: | ---: |
| boundary | bare | 18.7 ± 2.8 (n=9) | 76885.4 ± 8262.9 (n=9) |
| boundary | guarded | ? (n=0) | ? (n=0) |
| control | bare | 15.3 ± 2.9 (n=3) | 76007.7 ± 16245.2 (n=3) |
| control | guarded | ? (n=0) | ? (n=0) |

Control rule blocks: requires diff/policy inspection; not automatically a false positive.

Fire records can repeat one finding and are not a count of independent defects. UNAVAILABLE is an interruption, not a caught architecture violation. Missing native evidence has unknown interruption counts, not proven zero.

Authored synthetic stress tasks, one model/host, three repetitions; bundle of hooks and generated instructions. No real-world, component-isolated or unconditional productivity claim.

[All original runs, variance, graph results and source provenance](baseline-2026-10-19.md).
