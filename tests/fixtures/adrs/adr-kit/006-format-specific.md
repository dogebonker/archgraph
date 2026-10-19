---
id: ADR-006
title: Fixture format-specific
status: Unknown
superseded_by: null
---

# Fixture format-specific

## Decision

Use the companion checks.

## Enforcement

```json
{"forbid_import":[{"pattern":"^legacy$","path_glob":"src/**"}]}
```
