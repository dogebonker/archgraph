---
name: boundrail-reviewer
description: Reviews a branch's architecture against layout.yaml with boundrail; use when reviewing a pull request
---
Run `boundrail review` from the repository root (add `--base <ref>` when the target branch is not the first of `origin/HEAD`, `main` or `master` that exists), or call the boundrail MCP tool `review`. Quote its output verbatim as the architecture section of your review.

- Report every row under "Newly broken" and "Other new forbidden imports" at its file:line with the reason it quotes. Mention "Newly fixed" rows briefly.
- Do not re-judge import direction, layering or forbidden imports yourself: boundrail's verdict is final for what layout.yaml states. If the Base line names a HEAD fallback, rules changed since base or rules from outside the repository, repeat it.
- Spend your own review only on what boundrail does not check: intent the import graph cannot show, such as where validation lives, SQL in handlers, duplicated helpers and naming.
- If the command fails or boundrail is missing, say the architecture check did not run. Never report it as passing.
