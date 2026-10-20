# 5. Go bound pairs are the single binding evidence

Status: accepted (2026-10-20)

## Context

Splitting a Go package needs every unexported name used across the new boundary exported, or the
files moved together. The Go collector saw these names while type-checking and kept only the list
of files that could not leave their package (`bound_files`).

## Decision

The reference Go collector records each bound file pair with the qualified unexported names
behind it (`Type.field` for selectors and composite-literal keys) and a `together` flag for pairs
that must move as one. `bound_files` stays only where no pair evidence exists: native Go, and
go-types scans saved before pairs carried names, where names to export are reported as unknown.
Readers go through `Collector::bound_paths()`.

## Consequences

A saved scan can price a new draft without rescanning. Scan documents grow: GoldenGate went from
1.61 MB to 1.79 MB (638 pairs, 2,506 names). Type aliases keep bare names while the helper
supports Go 1.21.
