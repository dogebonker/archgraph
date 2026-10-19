---
name: archgraph-rules
description: Writes and calibrates an archgraph layout.yaml from an architecture the user approves (layered, hexagonal or clean, modular monolith, frontend domains, feature folders) and wires the CI gate. Use when asked to define, write, tighten or enforce architecture rules, layers or module boundaries for a repository.
license: MIT OR Apache-2.0
metadata:
  compatibility: Needs git and the archgraph binary (setup in the archgraph skill).
---

# archgraph-rules

## Input-quality interpretation

Check the saved source root, grouping depth and collector limitations before comparing SCC counts. Native Rust excludes definite test-only references and keeps containment structural; unknown cfg, macro expansion and type resolution remain incomplete. Its cycles measure source coupling, not initialization failure. A shared state container is a hub; do not attribute an entire component to it without a counterfactual. Rust/Java/Elixir semantic cuts stay non-mechanical. For Python, empty initializers do not establish a facade, and unresolved installed dependencies need actual package metadata. Explicit layout policies remain authoritative. Restart a running daemon and rescan after a collector upgrade.

## MCP tools

Prefer MCP `rules({paths})` and `explain({path})`/`explain({module})` when available to inspect existing constraints and dependency evidence. Use `check_diff` for newly introduced violations under the current policy; it does not prove a rule existed historically or represents the intended architecture. Inspect truncation markers and use CLI scans/checks for complete evidence. Drafting, calibrating planted violations and changing `layout.yaml` still follow the workflow below; MCP does not write policy. Fall back to the CLI when MCP is unavailable.

A rules file is done when it is **calibrated**: every rule has caught a planted violation, every intended import still passes, and the user has approved the layout it encodes. An uncalibrated rule may silently allow everything.

## Steps

To start from observed dependencies, run `archgraph infer-layout <tmp>/graph.json` after scanning. It prints a draft with SCC-aware layers, unresolved cycle groups and sparse-boundary proposals with existing violations. Layer evidence is top-down; executable YAML remains foundation-first. All import kinds participate, matching `check`. Use this as evidence for step 1, then review intent and calibrate the approved rules; absence of imports is not proof of an intended boundary. `--apply <new-path>` creates a draft without overwriting an existing file.

1. **Agree on the architecture.** Scan without rules (Setup in [the archgraph skill](../archgraph/SKILL.md)) and read the folders, modules and module imports as they are. Propose the pattern the code is closest to, and one alternative, from [patterns.md](patterns.md); keep an existing convention over a new one. Done when the user has chosen a pattern and confirmed which folders are its layers, modules and public entries.
2. **Write `layout.yaml`** with the pattern's template, adapted to the real folder names (syntax below). Give every `forbid` an `id` and a `reason` in the team's words. Done when `archgraph scan <root> --rules layout.yaml -o <tmp>/graph.json` runs.
3. **Read the verdicts.** Run `archgraph check <tmp>/graph.json`. Walk the user through every forbidden and unplanned import, grouped by rule (your `id`s, or a built-in one: `layers`, `public`, `same-module`, `none` for unplanned, `default`): either the rule misstates the layout (fix the rule) or the import is a real violation (keep it on the list). Done when each group is classified and the rules are corrected.
4. **Calibrate.** In a temporary worktree (`git -c core.hooksPath=/dev/null worktree add --detach <tmp>/plant HEAD`), copy `layout.yaml` in, link the repository's `node_modules` into it when it has one (or pass `--depcruise <path>`), and add one small planted file per rule that breaks exactly that rule and no other (a `forbid` between two files of the same layer, a layer pointing upward, an internal file imported from outside its module, a reachable chain for `reachable: true`), plus one planted import that each rule must allow (the public entry, a downward layer import). Scan the worktree with the rules and run `archgraph diff <step-3 graph> <plant graph> --fail-on new-forbidden,new-unplanned --format markdown`. Done when the diff lists exactly the violating plants, each forbidden by its own rule id, and nothing else (the allowed plants are absent from it), and `git worktree remove --force <tmp>/plant` has removed the worktree.
5. **Baseline and gate.** Real violations from step 3 either get fixed (one per change, with the archgraph-fix skill, after the user has committed the approved `layout.yaml`, since that skill starts from a clean tree) or stay as a recorded baseline: the CI gate fails only on new ones, so existing violations do not block work. Add the workflow below as `.github/workflows/architecture.yml`. On pull requests the action compares HEAD with the base commit under HEAD's `layout.yaml` and fails only on new forbidden imports and new cycles. On a push it fails when any import is forbidden, so add `push` to the triggers only once the baseline is empty. It installs archgraph and the collector itself (dependency-cruiser and typescript, grimp, or Go when the path has `go.mod`), so the repository needs neither. Set `path` when the project is in a subfolder, `language: python` when the Python root has no `pyproject.toml`, `setup.py` or `setup.cfg`, and `fail-on: new-forbidden,new-cycles,new-unplanned` to fail on unplanned imports too. Use `default: forbidden` only when the user wants every import the rules do not name to fail. Done when the user has approved `layout.yaml` and the CI change, and you have listed the baseline violations.

   ```yaml
   name: architecture
   on: [pull_request]   # add push once there are no baseline violations; never pull_request_target
   permissions:
     contents: read
   jobs:
     archgraph:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
           with:
             fetch-depth: 0
             persist-credentials: false
         # TypeScript/JavaScript only: dependency-cruiser 18.4.0 needs Node 22
         - uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0
           with:
             node-version: 22
         # Python only: CPython 3.10 to 3.14, e.g. actions/setup-python
         - uses: dogebonker/archgraph@v0.1.0
   ```

## Syntax

```yaml
version: 1
modules:                      # only when folders do not match the intended modules
  - { id: "domains/<name>", path: "src/domains/<name>/**" }
layers:                       # lowest first; a file may import its own or a lower layer
  - "src/shared/**"
  - "src/domains/*/model.server.ts"
  - "src/domains/*/ui/**"
public:                       # module pattern and its public entries
  - { module: "src/domains/<name>/**", entries: ["src/domains/<name>/index.ts"] }
forbid:
  - { id: shared-no-domain, from: "src/shared/**", to: "src/domains/**", reason: "shared imports no domain" }
  - { id: browser-no-server, from: "src/domains/*/client.ts", to: "**/*.server.ts", reachable: true }
allow:
  - { from: "app/**", to: "**", reason: "composition" }
default: forbidden            # optional: every import no rule names is forbidden
```

- `*` matches one path segment, `**` any depth, `{a,b}` alternatives. Paths are relative to the scanned root.
- Imports within one layer, and within one module, are allowed unless a `forbid` names them: siblings in one layer (driving and driven adapters) need their own `forbid` rules.
- `<name>` matches one segment and binds it within a rule: in `modules` path to id, in `public` module to entries, and in `forbid`/`allow` importer to target, so `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }` means "within one module". `layers` take no capture: write `*`.
- `reachable: true` on a `forbid` also matches when any file matching `to` is reachable from the imported file.
- Verdict order, first match wins: forbid, allow, public (an internal file imported from outside its module), layers, same module, public entry, otherwise unplanned (rule `none`), or forbidden (rule `default`) with `default: forbidden`.
- Strings are single plain lines; unknown keys are an error. An older archgraph ignores `default`, so pin the version wherever it gates CI.
- Type-only imports count in verdicts. With a `src/` directory only `src/` is scanned, so rules about root folders such as `app/**` see only the files `src/` imports.

For a forbidden-edge or deep-import finding, obtain the current recipe with MCP `propose` (CLI `archgraph propose <finding_id>` otherwise) before editing. Interface locations and reexports are suggestions under the current rules, not permission to rewrite policy. Preserve the structured verification baseline, treat null deltas as unknown and use [archgraph-fix](../archgraph-fix/SKILL.md) for dry-run/apply/verify. Findings lacking a concrete edit remain advisory diagnostics; changing ranking with `--rank legacy` does not change rule enforcement.
