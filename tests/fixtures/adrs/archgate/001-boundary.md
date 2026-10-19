---
id: ARCH-001
title: Fixture boundary
domain: architecture
rules: false
files: ["src/**/*.ts"]
---

# Fixture boundary

## Decision

`src/domain/**` must not import `src/adapters/**`.
