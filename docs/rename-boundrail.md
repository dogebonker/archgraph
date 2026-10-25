# Upgrade from Archgraph to Boundrail

The product and executable are now **Boundrail** and `boundrail`. The source repository remains
[`dogebonker/archgraph`](https://github.com/dogebonker/archgraph). The rename does not publish a new
registry package or rename the GitHub repository.

## Build the current executable

From a current source checkout:

```sh
cargo +stable install --path . --locked --bin boundrail
boundrail --version
```

Use `boundrail` in new scripts and commands. The Rust package and crate are also named `boundrail`.
Rust library consumers can update their imports, or keep a local dependency alias with
`archgraph = { package = "boundrail", git = "https://github.com/dogebonker/archgraph" }`.
The npm package definition is `boundrail-cli`. Registry publication is a separate release step.
The installer and npm wrapper accept both new and legacy release archive layouts. An older binary
keeps the behavior and version of that older release, even when installed under the new filename.
Build from current source for the features described in the README.

## Keep existing reports and project state

These identities remain stable:

| Existing identity | Compatibility behavior |
| --- | --- |
| `.archgraph/` and `.archgraphignore` | Existing project state, snapshots, exclusions, and receipts keep their paths. |
| Default `archgraph.json`, `archgraph.html`, and `archgraph.sarif` filenames | Current command and integration defaults still agree. |
| `archgraph/v1`, `archgraph/v1.1`, `archgraph/v1.2`, and other versioned protocol IDs | Existing reports and validators retain their wire contracts. |
| Serialized scan `tool.name: archgraph` | This is a compatibility identity, independent of the visible product name. |
| SARIF `driver.name: archgraph` | Code-scanning identity stays stable. `driver.fullName` supplies the Boundrail display name. |
| `ag-` finding IDs | Saved finding references and verification baselines remain recognizable. |
| `archgraph:*` owned fences and managed-hook markers | Existing managed content can be updated without creating a second owned block. |
| `ARCHGRAPH_*` environment overrides | Existing binary, collector, context, guard, and installer settings remain accepted. |

Historical screenshots, benchmark results, and transcripts retain the names and commands recorded
at capture time. They are evidence, rather than templates for new commands.

## Update agents and the daemon

Stop a daemon started by the old executable before starting a new one for the same repository:

```sh
boundrail serve . --engine native --symbols
```

Reinstall managed hooks with the new executable:

```sh
boundrail hook install --agent codex
```

Use `--agent claude` for Claude Code. The existing ownership markers let installation replace
managed handlers in place. The OpenCode managed plugin retains its `archgraph.ts` filename so
the rename does not load duplicate plugins. Hand-written content remains protected.

Replace the executable command in an existing MCP configuration with `boundrail`. A display-key
rename is optional. Replace the existing configuration rather than adding a duplicate server.
The checked-in [MCP examples](mcp/) use the new name.

The bundled skills are now `boundrail`, `boundrail-context`, `boundrail-guard`, `boundrail-fix`, and
`boundrail-rules`. Preserve local customizations when installing updates. The generated
`project-architecture` skill keeps its name and draft/adopt lifecycle. Regenerate and review a
draft before adopting its new command text.

The rename does not delete old manually installed skill directories or rewrite arbitrary unmanaged
hooks and commands. Review those installations before removing or replacing them.
