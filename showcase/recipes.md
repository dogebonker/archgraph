# Pinned showcase fix recipes

Actual CLI `propose` and `apply --dry-run` results for the original 20 TypeScript/Python showcase repositories. Findings are recomputed and ranked by the current engine from the saved edge-quality graphs; the source collector is not rerun. No recipes were applied.

Binary SHA-256: `d8904d3947dbe1f474d9575c00b37171a8ec6b04fbd32f530ffa60813c80675d`. Manifest SHA-256: `1a91d643c65defb2098bff2edf1e48864d65cccfc5603202dd72ea4209425533`. Full recipes, exact commands, input graph hashes and source/index checks are in [`recipes-results.json`](recipes-results.json).

The executables are frozen private copies for the whole audit, even if Cargo rebuilds concurrently. The JSON records the working revision and source-input hashes; binary hashes identify the measured engine, independently of uncommitted or concurrent source edits.

“Ready” means the source-aware mechanical planner accepted the dry-run, not that runtime behavior or compilation has been proved. “Blocked” records the planner’s real reason; an agent must perform those semantic changes. Unknown predicted deltas are shown as `?`.

| Repository | Pin | Top finding / recipe | Confidence | Predicted pairs removed | Actual dry-run |
| --- | --- | --- | --- | ---: | --- |
| nest | `1c2f68951841` | cycle: cut_import packages/microservices/context/rpc-context-creator.ts → packages/microservices/constants.ts | medium | 1 | Blocked (exit 2): packages/microservices/context/rpc-context-creator.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| next.js | `00c4ba8b6e26` | cycle: cut_import src/client/page-bootstrap.ts → src/server/dev/hot-reloader-types.ts | low | 1 | Blocked (exit 2): src/client/page-bootstrap.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| excalidraw | `4c37956ba5f0` | cycle: cut_import actions/actionLinearEditor.tsx → components/CommandPalette/CommandPalette.tsx | medium | 1 | Blocked (exit 2): actions/actionLinearEditor.tsx: import has live or ambiguous bindings; remove references manually before cutting it |
| immich | `3ae71117413c` | cycle: cut_import src/decorators.ts → src/schema/functions.ts | medium | 1 | Blocked (exit 2): src/decorators.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| activepieces | `55134042a88d` | cycle: cut_import src/app/authentication/otp/otp-service.ts → src/app/ee/helper/email/email-service.ts | medium | 1 | Blocked (exit 2): src/app/authentication/otp/otp-service.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| payload | `f177a2621999` | cycle: cut_import src/admin/initAdminContext.ts → src/index.ts | medium | 1 | Blocked (exit 2): src/admin/initAdminContext.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| typeorm | `fe9239428751` | cycle: move_file driver/DriverFactory.ts → driver/driver-factory/DriverFactory.ts | high | 10 | Ready (exit 0) |
| vite | `928e721b3669` | cycle: cut_import src/module-runner/evaluatedModules.ts → src/module-runner/sourcemap/decoder.ts; cut_import src/module-runner/runner.ts → src/module-runner/sourcemap/index.ts | low | 1 | Blocked (exit 2): src/module-runner/evaluatedModules.ts: import has live or ambiguous bindings; remove references manually before cutting it; src/module-runner/runner.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| outline | `095b096c2197` | cycle: cut_import app/components/Lightbox.tsx → app/editor/components/EditorContext.tsx | medium | 1 | Blocked (exit 2): app/components/Lightbox.tsx: app/components/Lightbox.tsx: syntax errors; repair syntax before applying a recipe |
| strapi | `d764743be322` | cycle: cut_import domain/module/index.ts → registries/namespace.ts | low | 1 | Blocked (exit 2): domain/module/index.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| trpc | `10e2ef98f784` | cycle: cut_import src/unstable-core-do-not-import/error/TRPCError.ts → src/unstable-core-do-not-import/utils.ts | medium | 1 | Blocked (exit 2): src/unstable-core-do-not-import/error/TRPCError.ts: import has live or ambiguous bindings; remove references manually before cutting it |
| django | `d430084791a6` | cycle: cut_import django/contrib/admin/models.py → django/contrib/contenttypes/models.py | medium | 1 | Blocked (exit 2): django/contrib/admin/models.py: import has live or ambiguous bindings; remove references manually before cutting it |
| fastapi | `0b919816378e` | cycle-via-grabbag: split_module fastapi/openapi/constants.py →  | low | ? | Blocked (exit 2): fastapi/openapi/constants.py: SplitModule: non-mechanical step; follow its instructions and run verify |
| flask | `d086db856be1` | — | — | ? | No actionable finding |
| pydantic | `5adfeaaaea94` | cycle: cut_import pydantic/_internal/_dataclasses.py → pydantic/fields.py | medium | 1 | Blocked (exit 2): pydantic/_internal/_dataclasses.py: import has live or ambiguous bindings; remove references manually before cutting it |
| httpx | `b5addb64f016` | — | — | ? | No actionable finding |
| scrapy | `fbfc89ae7b72` | cycle: cut_import scrapy/http/request/__init__.py → scrapy/__init__.py | medium | 1 | Blocked (exit 2): scrapy/http/request/__init__.py: scrapy/http/request/__init__.py: import has no unique safe source span; rescan or edit manually |
| celery | `ef37e616ef67` | cycle: cut_import celery/app/trace.py → celery/worker/state.py | medium | 1 | Blocked (exit 2): celery/app/trace.py: import has live or ambiguous bindings; remove references manually before cutting it |
| zulip | `fb4f7abc378d` | cycle: cut_import analytics/lib/counts.py → zilencer/models.py | low | 1 | Blocked (exit 2): analytics/lib/counts.py: import has live or ambiguous bindings; remove references manually before cutting it |
| sqlalchemy | `262c0f0ef49a` | cycle: cut_import sqlalchemy/sql/sqltypes.py → sqlalchemy/engine/processors.py | medium | 1 | Blocked (exit 2): sqlalchemy/sql/sqltypes.py: import has live or ambiguous bindings; remove references manually before cutting it |

Measured: 20 repositories; 1 mechanically ready dry-runs; 17 blocked; 2 with no actionable finding; 0 audit errors. Source bytes/modes and the real Git index were unchanged around each audit.

The unchanged-source check hashes tracked and nonignored untracked files from `git ls-files`, including symlink targets and file modes. `.archgraph/` metadata is explicitly allowed; ignored build/install artifacts and submodule contents are outside this check. Each graph’s recorded commit and the checkout HEAD must both equal the manifest pin. Tracked-clean status is recorded for each checkout; any existing dirtiness is preserved rather than discarded.
