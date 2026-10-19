# Controlled usefulness test

agent exit 0; business oracle passes; expected area touched; policy unchanged; verified arm activation/absence; measured zero final violations; no harness errors.

Every planned run stays in the denominator; failed, missing and unverified runs are non-successes.

| Task/group | Bare success | Guarded success | Bare business pass | Guarded business pass | Guarded rule-fire records | Guarded UNAVAILABLE observed / runs unknown |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| boundary | 9/9 | 9/9 | 9/9 | 9/9 | 0 | 0 / 0 |
| control | 3/3 | 3/3 | 3/3 | 3/3 | 0 | 0 / 0 |
| tax | 3/3 | 3/3 | 3/3 | 3/3 | 0 | 0 / 0 |
| inventory | 3/3 | 3/3 | 3/3 | 3/3 | 0 | 0 / 0 |
| receipt | 3/3 | 3/3 | 3/3 | 3/3 | 0 | 0 / 0 |
| discount | 3/3 | 3/3 | 3/3 | 3/3 | 0 | 0 / 0 |

All-run costs (mean ± sample SD; unknown values are not imputed as zero):

| Group | Arm | Agent seconds | Tokens |
| --- | --- | ---: | ---: |
| boundary | bare | 19.5 ± 4.2 (n=9) | 75335.7 ± 5638.0 (n=9) |
| boundary | guarded | 35.1 ± 5.1 (n=9) | 139865.7 ± 21228.8 (n=9) |
| control | bare | 15.3 ± 0.9 (n=3) | 60201.7 ± 312.7 (n=3) |
| control | guarded | 34.8 ± 2.4 (n=3) | 127090.3 ± 11206.3 (n=3) |

Control rule blocks: none observed.

Fire records can repeat one finding and are not a count of independent defects. UNAVAILABLE is an interruption, not a caught architecture violation. Missing native evidence has unknown interruption counts, not proven zero.

Authored synthetic stress tasks, one model/host, three repetitions; bundle of hooks and generated instructions. No real-world, component-isolated or unconditional productivity claim.

[All original runs, variance, graph results and source provenance](baseline-2026-10-19.md).
