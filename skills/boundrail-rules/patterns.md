# Layout patterns

Templates for `layout.yaml`, one per architecture. Rename the folders to the repository's own; keep the shape. Python paths use the package folder (`src/<pkg>/` or `<pkg>/`), written here as `src/pkg/`.

## Layered (n-tier)

Fits a service with one business area and simple rules. Lower layers never import upper ones.

```yaml
version: 1
layers:
  - "src/models/**"          # Python: src/pkg/models.py
  - "src/repositories/**"    # Python: src/pkg/crud.py
  - "src/services/**"
  - "src/routes/**"          # Python: src/pkg/api/**
  - "src/main.ts"            # Python: src/pkg/main.py
```

## Hexagonal / clean (ports and adapters)

Fits one bounded context with real business rules. Dependencies point inward; driving and driven adapters do not import each other.

```yaml
version: 1
layers:
  - "src/domain/**"
  - "src/application/**"     # use cases and ports
  - "src/adapters/**"
  - "src/main.ts"            # the composition root
forbid:
  - { id: in-no-out, from: "src/adapters/in/**", to: "src/adapters/out/**", reason: "driving adapters do not import driven ones" }
  - { id: out-no-in, from: "src/adapters/out/**", to: "src/adapters/in/**", reason: "driven adapters do not import driving ones" }
```

Python (Cosmic Python naming): layers `src/pkg/domain/**`, `src/pkg/service_layer/**`, `src/pkg/{adapters,entrypoints}/**`, `src/pkg/bootstrap.py`, with the sibling forbids between `adapters` and `entrypoints`.

## Modular monolith

Fits several bounded contexts in one deployable. Each module exposes one public entry; everything else is private. Layers repeat inside each module.

```yaml
version: 1
modules:
  - { id: "modules/<name>", path: "src/modules/<name>/**" }
layers:
  - "src/shared/**"
  - "src/modules/*/domain/**"
  - "src/modules/*/application/**"
  - "src/modules/*/infrastructure/**"
  - "src/modules/*/interfaces/**"
  - "src/main.ts"
public:
  - { module: "src/modules/<name>/**", entries: ["src/modules/<name>/index.ts"] }
forbid:
  - { id: shared-no-module, from: "src/shared/**", to: "src/modules/**", reason: "the shared kernel imports no module" }
```

Python: entries are `src/pkg/modules/<name>/__init__.py`. A module without its entry file is still private: imports of its files from outside are forbidden.

When the team's contract is a whitelist (an import is allowed only when a rule names it), write the allowed edges as `allow` rules, keep the inward rules as `forbid`, and add `default: forbidden`. Bind `<name>` so "anything within its own module" is one rule: `{ from: "src/modules/<name>/**", to: "src/modules/<name>/**" }`.

## Frontend domains (FDA, Next.js App Router)

Fits a frontend grouped by business domain with explicit browser and server boundaries.

```yaml
version: 1
modules:
  - { id: "domains/<name>", path: "src/domains/<name>/**" }
layers:
  - "src/shared/**"
  - "src/domains/*/{types,constants}.ts"
  - "src/domains/*/repo.server.ts"
  - "src/domains/*/model.server.ts"
  - "src/domains/*/client.ts"
  - "src/domains/*/controller.ts"
  - "src/domains/*/ui/**"
  - "app/**"
public:
  - { module: "src/domains/<name>/**", entries: ["src/domains/<name>/index.ts", "src/domains/<name>/server.ts"] }
forbid:
  - { id: shared-no-domain, from: "src/shared/**", to: "src/domains/**", reason: "shared code imports no domain" }
  - { id: browser-no-server, from: "src/domains/*/{client,controller,index}.ts", to: "**/*.server.ts", reachable: true, reason: "the browser graph never reaches server code" }
```

Calibrate with: a client importing a `model.server.ts`, an `index.ts` re-exporting server code, a cross-domain internal import, and shared importing a domain (all forbidden); a cross-domain import of `index.ts` (allowed).

## Feature folders

Fits a frontend where features must not reach into each other.

```yaml
version: 1
modules:
  - { id: "features/<name>", path: "src/features/<name>/**" }
public:
  - { module: "src/features/<name>/**", entries: ["src/features/<name>/index.ts"] }
forbid:
  - { id: shared-no-feature, from: "src/shared/**", to: "src/features/**", reason: "shared code imports no feature" }
```
