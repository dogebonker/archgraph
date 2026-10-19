---
id: ARCH-004
title: Fixture alternatives
domain: architecture
rules: false
files: ["src/**/*.ts"]
---

# Fixture alternatives

## Considered Options

`src/domain/**` must not import `src/adapters/**`.

## Decision

Keep the current boundary.
