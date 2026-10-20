# 3. Target paths live in the rules, not in a mapping

Status: accepted (2026-10-20)

## Context

During a migration some files sit at today's paths and some at the target's. A rule written over
today's paths stops protecting a slice once its files move, and a rule over target paths protects
nothing until they do. A moved handler importing an unmoved repository matches neither.

## Decision

One rule covers both locations through brace alternation, for example
`{internal/<name>/handler*.go,internal/app/<name>/handler/*.go}`. Globset with
`literal_separator` accepts this and a repeated `<name>` binds once. `moves:` feeds only move
cost and progress, never verdicts, and there is no post-move verdict.

## Consequences

One rule semantics covers today, the target and every state in between, with no code. Globs get
longer, and a file directly in the target root (such as `internal/app/handler.go`) binds a slice
named after the root, so the guide warns to keep that folder free of files. Mapping target paths
back through `moves` would make verdicts depend on a second section and is hard to undo once
teams write layouts either way.
