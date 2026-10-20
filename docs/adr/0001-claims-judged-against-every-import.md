# 1. A claim is every forbid sharing an id, judged against every import

Status: accepted (2026-10-20)

## Context

An import's verdict comes from the first rule that matches it (`Compiled::verdict` in
`src/rules.rs`). A target architecture states claims such as "dependencies point inward", each
written as one or more `forbid` rules. If a claim were read off the rule that decided each
import, an import caught by an earlier rule would leave a later claim reading `holds` while the
import it forbids is in the code.

## Decision

A claim is every `forbid` with the same `id`, where at least one carries a `reason`. It is
evaluated against every import on its own, ignoring the first-match order. `verdict` and the
claim evaluation share the same forbid step, so they cannot disagree on whether a rule matches.

## Consequences

One import can have two views: its verdict names the first rule, and every claim it breaks
lists it. The guide explains this. Reversing the decision would change every reported verdict.
