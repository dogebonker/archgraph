# Catching architecture on pull requests: how other tools do it

Researched 2026-10-21 from primary sources (vendor docs, source repositories, authors' own pages).
"Docs do not state" means the cited page was read and says nothing, not that the feature is absent.
Pages were fetched through a summarizing reader: check exact wording at the source before quoting.

## Summary

Three families catch architecture on a pull request, and each sees something the others miss.

| Family | Representation | Rules are | Catches | Misses |
| --- | --- | --- | --- | --- |
| AI reviewers (Qodo, CodeRabbit, Greptile, Graphite, Copilot, Amp, Bito, Ellipsis) | the diff plus retrieval: embeddings, symbol references, a call/import graph (Greptile), co-change (CodeRabbit) | natural language scoped by glob | intent the code does not encode ("controllers must not query the DB directly", "validation belongs on the DTO") | no determinism guarantee from any vendor; no baseline or ratchet; no drift trend; transitive effects only if retrieval finds them |
| Rule and graph tools (ArchUnit, Konsist, NDepend, jQAssistant, CodeQL, Semgrep, dependency-cruiser, Nx, import-linter, Tach, Packwerk, Deptrac, go-arch-lint, Spring Modulith, Sonar) | import graphs (most), class graphs with call/field/constructor edges (ArchUnit), queryable databases (CodeQL, jQAssistant, NDepend), AST patterns (Semgrep) | code, config or queries | dependency direction, layers, cycles, public surfaces, exactly and repeatably | anything not visible as a dependency; most see imports, not calls; one language each, except CodeQL and Semgrep |
| Models, decisions, history (Structurizr, ADR tools, Archgate, fitness functions, CodeScene, Sonargraph) | intent models in text, ADR files, git history | DSLs, prose plus companion checks, gates on decline | logical coupling the import graph cannot see (files that change together across a boundary); decay over time | the model is rarely checked against code (Structurizr checks only the model itself); ADR prose enforces nothing without a second artifact |

So the quote's ideal ("it knows a service is calling a controller and flags it on every PR") is
today delivered by a language model judging prose rules over retrieved context. That catches
fuzzy intent, but no vendor promises it catches the same thing twice. The deterministic tools
catch dependency direction exactly but cannot read intent. Nobody in this set combines the two:
a rule the team writes once in plain language, checked exactly where it can be and judged by a
model only where it cannot, with a baseline, a trend and a verdict that does not change between
runs.

## What archgraph could learn

Codes are stable for discussion.

- **L1. Be the deterministic layer an AI reviewer calls, not a competitor to it.** Every AI
  reviewer above treats architecture as prose for the model. Archgraph already gives claim
  verdicts with evidence over MCP. A reviewer agent (Amp checks, Ellipsis reviewers, Claude Code)
  could call `target`/`check_diff` for the parts that are dependency questions and spend the
  model only on what is left. The skill step for drafting rules from a proposal is the bridge:
  a sentence in AGENTS.md or a deck becomes a claim archgraph can check.
- **L2. Inline pull request annotations and SARIF.** CodeQL and Semgrep get line comments on
  changed code through SARIF and code scanning (docs.github.com triaging page, semgrep-ci
  overview). Archgraph posts a job summary and one `::error` line only. A `--format sarif` for
  `diff` (new forbidden imports with file and line, claim reason as the message) would put
  "this import breaks *Dependencies point inward (slide 8)*" on the exact line, the main thing
  AI reviewers do well.
- **L3. A committed baseline that shrinks.** ArchUnit `FreezingArchRule`, dependency-cruiser
  `--baseline` with `shrink-only`, Packwerk `package_todo.yml`, jQAssistant baselines and Deptrac
  `skip_violations` all keep known violations in a reviewed file and fail only on new ones, with
  fixed entries dropping out. Archgraph's ratchet needs a base checkout and makes pushes without
  a base red whenever any forbidden import exists (guide, action section). A committed
  `archgraph.baseline` (or known violations per claim) would work on pushes, in hooks and in
  `check`, and show debt going down.
- **L4. Calls, not only imports, where the collector knows them.** ArchUnit is the only rule tool
  here whose edges are method calls, field accesses and constructor calls; import-graph tools
  cannot tell a call from an unused import. Archgraph's Go collector already records references
  to the declaring file of each function, type, field and method, which is closer to ArchUnit
  than to the import tools; the guide could say so and TypeScript/Python could follow where the
  native engine resolves symbols. That is what "a service calls a controller" needs.
  Covered at file level: the guide's "What an edge means" lists what an edge is per language and
  engine, and a `forbid` on files already catches calls and field uses in Go, Java, Rust and
  Elixir. Java no longer counts an unused import. The remaining gap is the kind of use: no edge
  says call, so a claim cannot forbid calls while allowing type uses, and TypeScript and Python
  edges stay imports.
- **L5. Fail on rules that match nothing, and on uncovered code, as options.** ArchUnit fails a
  rule that matches zero classes by default; Deptrac `--fail-on-uncovered` and Sonargraph
  "unassigned" make coverage gaps visible. Archgraph reports `vacuous` claims and unplanned
  imports but never fails on them. A `fail-on: vacuous` in the action would stop a target from
  silently protecting nothing after a rename.
- **L6. History as a second signal, scoped to claims.** CodeScene and CodeRabbit use co-change;
  Tornhill's point is to look for change coupling "that spans architectural boundaries".
  Archgraph has `--cochange` and hidden coupling findings. Reporting files that changed together
  across a claim's two sides, in the pull request summary, would catch coupling through DI,
  events or the database that no import shows.
- **L7. Mine candidate rules from history, then verify them.** Qodo's Rule Miner and Greptile's
  suggested rules propose rules from past pull requests for a human to accept. Archgraph's
  `infer-layout` proposes forbids from the graph. Proposing claims from the target's own
  pattern (a direction that holds in most slices, like `layered` in 16 of 18) and showing where
  it breaks would give teams a starting target without a deck.
- **L8. Feedback is a reason, not a vote.** AI reviewers learn from thumbs and dismissals, which
  tunes tone but never makes a rule exact. Archgraph's equivalent is an `allow` with a `reason`
  in the layout, reviewed in the pull request like code. Keep that: an exception should be a
  reviewed rule change, not a learned preference.
- **L9. Keep architecture as code.** Sonar is moving its intended architecture into the server UI
  and deprecating the file form; Structurizr keeps the model in a diffable DSL. Archgraph's
  `layout.yaml` is reviewable in the same pull request that changes it, which the UI route loses.

## AI reviewers

### Qodo (Qodo Merge / PR-Agent)
- Representation: reviews "the full repository" but the review page does not explain indexing.
  The older Context Engine is an optional RAG (`enable_rag=true`, Enterprise), with "no guarantee
  that RAG results will be useful"; v1 docs are being sunset.
- Rules: `best_practices.md` (guideline plus before/after code, under about 800 lines); the newer
  Rules system takes rules from the portal, AGENTS.md or CLAUDE.md, or a "Rule Miner" over pull
  request history. Scope syntax is not documented on that page.
- Enforcement: model-judged; violations become suggestions labelled "Organization best practice";
  a judge agent drops low-confidence findings. No guarantee stated.
- Reporting: findings ranked by severity with Apply Suggestion; merge blocking not stated.
- False positives: dismissal and fixed-versus-ignored history; rarely fixed rules auto-deactivate.

Sources: https://docs.qodo.ai/code-review ; https://docs.qodo.ai/code-review/get-started/rule-enforcement ; https://docs.qodo.ai/governance/rule-enforcement/without-rule-system/best-practices.md ; https://docs.qodo.ai/v1/features/qodo-aware-rag.md

### CodeRabbit
- Representation: the diff plus a code graph of symbol definitions and references, plus
  commit-history co-change; the graph is regenerated per review.
- Rules: `reviews.path_instructions` (glob plus prose, for example `src/controllers/**`: "Flag any
  direct database queries that bypass the ORM layer"); ast-grep YAML rules; natural-language
  pre-merge checks that run agentically (ast-grep, ripgrep, sandboxed shell) and return Passed,
  Failed or Inconclusive.
- Enforcement: ast-grep matching is structural, single-file; how matches become comments is not
  documented. Path instructions only steer the model; the docs warn vague instructions are unreliable.
- Reporting: check results in the summary table only; `error` mode blocks merge only with the
  Request Changes Workflow, and Inconclusive does not block. Instructions are capped at 1,000 characters.
- False positives: learnings from chat feedback; warning mode by default for rollout.

Sources: https://docs.coderabbit.ai/integrations/code-graph-analysis ; https://docs.coderabbit.ai/guides/review-instructions ; https://docs.coderabbit.ai/configuration/ast-grep-instructions ; https://docs.coderabbit.ai/pr-reviews/custom-checks.md ; https://docs.coderabbit.ai/pr-reviews/pre-merge-checks.md ; https://docs.coderabbit.ai/knowledge-base

### Greptile
- Representation: a pre-built repository graph (directories, files, functions, classes,
  variables; calls, imports, dependencies, variable usage), queried for callers, callees and
  similar functions; built at sign-up, update timing not stated; rules need the repo "Indexed".
- Rules: dashboard, `.greptile/config.json` (`rule`, `scope` globs, `severity`, `id`),
  `.greptile/rules.md`, `.greptile/files.json` pointing at architecture docs. Docs ask for
  measurable rules.
- Enforcement: model over rules plus graph context; no determinism statement.
- Reporting: review comments; status checks not mentioned; per-rule usage and "Addressed Rate".
- False positives: rules suggested after about 10 pull requests for approval.

Sources: https://www.greptile.com/docs/how-greptile-works/graph-based-codebase-context.md ; https://www.greptile.com/docs/code-review/custom-standards ; https://www.greptile.com/docs/how-greptile-works/custom-rules.md

### Graphite Agent (formerly Diamond)
- Representation: optional code indexing of files per repo; the kind of index is not documented.
- Rules: prompts in the UI or repo files by glob (rule, bad example, good example, reasoning);
  large files are truncated and many files lower quality.
- Enforcement: model-judged; no architecture-specific guidance.
- Reporting: pull request comments; severity and merge blocking not described on that page.
- False positives: exclusions, per-rule acceptance and vote rates.

Sources: https://graphite.com/docs/ai-review-customization ; https://graphite.com/docs/code-indexing

### GitHub Copilot code review
- Representation: the diff plus "full project context gathering" on Actions; no graph or index named.
- Rules: `.github/copilot-instructions.md`, path-scoped `.github/instructions/*.instructions.md`
  (`applyTo:` globs), `AGENTS.md`; read from the head branch.
- Enforcement: model-judged; GitHub says Copilot "may miss problems or make mistakes".
- Reporting: review comments that do not count toward required approvals by default; merge
  gating comes from separate products.

Sources: https://docs.github.com/en/copilot/concepts/agents/code-review ; https://docs.github.com/en/copilot/customizing-copilot/adding-repository-custom-instructions-for-github-copilot

### Amp (from Sourcegraph)
- Representation: an agent exploring the checkout with tools; no pre-built index named.
- Rules: markdown "checks" in `.agents/checks/`, scoped to the directory holding `.agents/`.
- Enforcement: one agent per check, "a stronger guarantee that each check will actually be
  checked" than rules in AGENTS.md; that the check runs, not that its verdict is deterministic.
- Reporting: `amp review` CLI or a thread request; no pull request status check documented.

Sources: https://ampcode.com/news/liberating-code-review

### Bito
- Representation: "Symbol Indexing, Abstract Syntax Trees (AST), and Embeddings"; details not documented.
- Rules: AGENTS.md, CLAUDE.md, GEMINI.md, Cursor and Windsurf rule files, scoped by
  subdirectory; the docs' own example is an architecture guide.
- Enforcement: model-judged. Reporting: comments, summary, changelist table.
- False positives: three negative votes enable a learned rule.

Sources: https://docs.bito.ai/ai-code-review-agent/key-features ; https://docs.bito.ai/ai-code-review-agent/implementing-custom-code-review-rules

### Ellipsis
- Representation: agent sessions (`claude_code` or `codex` harness) that can check out named
  repositories; no index described.
- Rules: `.ellipsis/code_review.yaml` with 1 to 64 reviewers (prompt, model, `paths` globs);
  older `ellipsis.yaml` had prose rules and `confidence_threshold`.
- Reporting: comments; an `output` block turns verdicts into approve or request-changes.
- False positives: an optional gatekeeper verifies findings before posting.

Sources: https://www.ellipsis.dev/docs/code-review/custom-reviewers ; https://docs.ellipsis.dev/config

### Patterns across AI reviewers
- Rules are natural language scoped by glob; none documents a layer or dependency-direction language.
- Enforcement is model-judged everywhere; the one structural exception, CodeRabbit's ast-grep,
  is single-file and cannot express transitive or import-graph rules. No vendor documents a
  determinism guarantee.
- Context is retrieval for the model, not a verified architecture model.
- Only CodeRabbit (`error` mode) and Ellipsis (`output`) document requesting changes.
- No baseline, ratchet or drift trend is documented; usefulness metrics measure comments, not violations.
- Feedback learning tunes preferences but cannot make a rule exact; rule size and count are capped.

## Rule and graph tools

### ArchUnit (Java)
Bytecode imported into a graph with `JavaMethodCall`, `JavaFieldAccess` and `JavaConstructorCall`
edges, so it sees calls. Rules are JUnit tests:
`classes().that().resideInAPackage("..service..").should().onlyBeAccessed().byAnyPackage("..controller..", "..service..")`;
`layeredArchitecture()` and `onionArchitecture()` are built in. A rule that matches no class fails
by default. `FreezingArchRule.freeze(rule)` stores current violations, reports only new ones, and
shrinks the store as they are fixed; `allowStoreCreation=false` stops CI from creating it silently.

Sources: https://www.archunit.org/userguide/html/000_Index.html

### Konsist (Kotlin)
Kotlin declarations and imports; rules are Kotlin tests with `assertArchitecture { … dependsOn … }`.
Import-level only: a fully qualified reference without an import is missed. No baseline yet
("will be added in the future").

Sources: https://docs.konsist.lemonappdev.com/writing-tests/architecture-assert.md ; https://docs.konsist.lemonappdev.com/features/add-konsist-existing-project.md

### NDepend (.NET)
A code model queried with CQLinq (`warnif count > 0 from m in Application.Methods where …`);
quality gates with `failif`. Baselines compare against an earlier analysis ("New Debt since
Baseline"), a ratchet on new debt. `NDepend.Console` exits non-zero on a failed gate.

Sources: https://www.ndepend.com/docs/cqlinq-syntax ; https://ndepend.com/docs/reporting-code-diff ; https://www.ndepend.com/docs/devops-quality-gate-strategy

### jQAssistant
Code and build artifacts scanned into Neo4j; rules are Cypher concepts and constraints with
severities. `jqassistant-baseline.xml` suppresses known violations and drops fixed ones.
`fail-on-severity` stops the build.

Sources: https://jqassistant.github.io/jqassistant/current/

### CodeQL
A relational database per language with data flow; QL queries. Not presented as an architecture
tool, but custom queries can express boundaries. Pull requests show "new alerts on lines of code
changed in the pull request"; results upload as SARIF.

Sources: https://codeql.github.com/docs/codeql-overview/about-codeql/ ; https://docs.github.com/en/code-security/code-scanning/managing-code-scanning-alerts/triaging-code-scanning-alerts-in-pull-requests

### Semgrep
AST patterns with metavariables, optional taint (cross-file only in Pro). No project-wide layer
graph, so cycles and layers are not native. `SEMGREP_BASELINE_REF` / `--baseline-commit` report
only new findings; `semgrep ci` comments on pull requests; `--sarif`.

Sources: https://docs.semgrep.dev/writing-rules/pattern-syntax ; https://docs.semgrep.dev/semgrep-ci/overview ; https://docs.semgrep.dev/cli-reference

### dependency-cruiser (JS/TS)
Module graph with dependency types (type-only, dynamic-import); `forbidden`/`allowed`/`required`
rules with path regexes, capture groups, `circular`, `reachable`, `moreUnstable`. `--baseline`
writes known violations, `--ignore-known` lowers them, `--baseline-mode shrink-only` ratchets.
The `err` reporter exits with the count of errors.

Sources: https://github.com/sverweij/dependency-cruiser/blob/main/doc/rules-reference.md ; https://github.com/sverweij/dependency-cruiser/blob/main/doc/cli.md

### Nx module boundaries
Project graph checked by ESLint `@nx/enforce-module-boundaries` with tags and `depConstraints`
(`onlyDependOnLibsWithTags`, `bannedExternalImports`, `banTransitiveDependencies`). Project
granularity; no baseline documented.

Sources: https://nx.dev/features/enforce-module-boundaries

### import-linter (Python)
Import graph including indirect chains; INI/TOML contracts (`forbidden`, `layers`, `independence`,
`protected`, `acyclic_siblings`) with `ignore_imports`. No baseline documented.

Sources: https://import-linter.readthedocs.io/en/stable/contract_types/

### Tach (Python)
Module import checks in Rust; `tach.toml` with `depends_on`, ordered layers, interfaces
(`expose`), `exact`, deprecated dependencies. No baseline file; incremental adoption via
unchecked modules.

Sources: https://github.com/gauge-sh/tach ; https://docs.gauge.sh/usage/configuration

### Packwerk (Ruby)
Static constant references (Zeitwerk), tuned to avoid false positives and accept false negatives;
method calls ignored. `package.yml` dependencies; privacy, layers and visibility via
packwerk-extensions. `package_todo.yml` records existing violations; `update-todo` removes fixed
ones; `strict` blocks new todo entries.

Sources: https://github.com/Shopify/packwerk/blob/main/USAGE.md ; https://github.com/rubyatscale/packwerk-extensions

### Deptrac (PHP)
Collectors assign classes to layers; YAML rulesets where everything not allowed is forbidden;
`--fail-on-uncovered`; `skip_violations` baseline. The 2.0.x repository is archived.

Sources: https://github.com/qossmic/deptrac/blob/2.0.x/docs/concepts.md

### go-arch-lint (Go)
Packages mapped to components by globs; `deps` with `mayDependOn`; also checks DI wiring in
composition code. Exit 1 on warnings; no baseline documented.

Sources: https://github.com/fe3dback/go-arch-lint

### Spring Modulith (Java/Kotlin)
Modules are sub-packages; `ApplicationModules.of(…).verify()` checks an acyclic module graph,
access through named interfaces and allowed dependencies; `type = OPEN` for migration.

Sources: https://docs.spring.io/spring-modulith/reference/verification.html

### Sonar architecture (with Structure101)
A current-architecture map per analysis; intended architecture defined in the server UI raises
"deviations" as issues. The file form (`architecture.json|yaml`) is deprecated "pending removal
in January 2026". Developer edition or higher.

Sources: https://docs.sonarsource.com/sonarqube-server/architecture.md ; https://docs.sonarsource.com/sonarqube-server/2025.6/design-and-architecture/configuring-the-architecture-analysis

### Patterns across rule tools
- Four representations: import graphs (most), class graphs with access edges (ArchUnit, Spring
  Modulith), typed code models and stored databases (NDepend, jQAssistant, CodeQL), AST patterns (Semgrep).
- "Service calls controller" is catchable only where call edges exist (ArchUnit; jQAssistant or
  CodeQL with custom queries). Import tools see only that a file imported something.
- Baselines come as a frozen violation store (ArchUnit, jQAssistant, dependency-cruiser, Deptrac,
  Packwerk), a diff against a baseline run or commit (NDepend, Semgrep, CodeQL), or not at all
  (Konsist, Nx). Some shrink automatically as violations are fixed.
- CI is mostly an exit code or a failing test; SARIF and native pull request annotations are
  documented only for CodeQL and Semgrep.
- Coverage is language-bound except CodeQL and Semgrep.

## Architecture models, decisions and behaviour

### Structurizr and the C4 model
A diffable text model (Structurizr DSL) apart from its views; ADRs attach with `!adrs`.
`inspect` checks the model's own completeness (missing descriptions, elements in no view); the
component finder reverse-engineers components from code, but no model-versus-code check is
documented. Baselining not described.

Sources: https://docs.structurizr.com/workspaces/inspections ; https://docs.structurizr.com/java/component/introduction ; https://docs.structurizr.com/as-code

### ADR tooling (adr-tools, MADR, Log4brains)
Markdown decisions in git; none enforces anything. MADR 4's optional "Confirmation" section
states how compliance is verified, in prose.

Sources: https://github.com/npryce/adr-tools ; https://adr.github.io/madr/ ; https://github.com/thomvaill/log4brains

### ADRs linked to checks (Archgate) and archgraph's ADR import
Archgate: ADR frontmatter plus a companion `.rules.ts` (`ctx.glob`, `ctx.grep`,
`ctx.report.violation`); `archgate check` exits 1 on a violation; no baseline stated. Archgraph's
`adr import` maps one sentence form and measured 1 of 35 Archgate and 0 of 51 ADR-kit documents
against its 50% target ([interop.md](interop.md)).

Sources: https://cli.archgate.dev/ ; https://github.com/archgate/cli

### Fitness functions (Ford, Parsons, Kua)
"Objective integrity assessment" of an architectural characteristic; atomic or holistic,
triggered, continual or temporal, static or dynamic; run in delivery pipelines "as gatekeepers".
No separate model and no baselining recipe in the pages read.

Sources: https://www.oreilly.com/library/view/building-evolutionary-architectures/9781492097532/ch02.html ; https://www.thoughtworks.com/insights/articles/fitness-function-driven-development

### CodeScene
Git history plus Code Health: hotspots (high change frequency, low health), change coupling
(same commit, same author in a window, same ticket; across repositories), architectural
components by glob. Gates are relative (health decline, new debt). Change coupling is "neither
good nor bad"; Tornhill: look for change dependencies "that span architectural boundaries".

Sources: https://docs.enterprise.codescene.io/versions/6.0.0/guides/technical/change-coupling.html ; https://docs.enterprise.codescene.io/latest/guides/architectural/architectural-analyses.html ; https://codescene.io/docs/guides/pr-integration/integrate-into-ci-cd.html ; https://adamtornhill.com/articles/aspnetclones/killtheclones.html

### Sonargraph and Lattix
Sonargraph: an architecture DSL over a parsed system model; unassigned elements listed;
Sonargraph-Build fails on violations; baselines as XML reports with gates on current state or on
the diff ("no additional error issues"), commercial only. Lattix: dependency structure matrix
with rules; its CI and rule pages could not be read.

Sources: https://eclipse.hello2morrow.com/doc/standalone/content/architecture_definition.html ; https://eclipse.hello2morrow.com/doc/standalone/content/defining_qualitygates.html ; https://docs.lattix.com/

### Patterns
- Intent lives in versioned text (Structurizr, Sonargraph, Archgate), but only Sonargraph and
  Archgate run it against code.
- ADR prose is not enforceable without a second artifact.
- History finds coupling the import graph misses, as an advisory signal.
- Legacy code is handled with relative gates (decline, new debt) rather than absolute thresholds.
- The grouping unit is a glob everywhere; unassigned code is a visible coverage gap.
