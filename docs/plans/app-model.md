# App model: infer services, infrastructure and calls from code

Branch `feat/app-model` from main 505027f. Approved 2026-10-22, revised 2026-10-25 after independent
critic review and the user's Q1-Q3 answers: infer the application model
(services, inbound routes, database and cache clients, queue publishers and consumers, outbound
HTTP) from source. It is the free, local, framework-free counterpart to Encore's service catalog
and flow diagram. Encore derives these because apps declare infrastructure with its SDK. archgraph
must infer them from arbitrary repositories. Today `archgraph services` reads a hand-written
manifest whose call evidence is "not read or verified" (`docs/guide.md:412-464`,
`src/services.rs:257-408`). The inferred model feeds the same `analyze`, report and forbid rules.
Not in scope: provisioning or running infrastructure, deploying or executing code, connecting to
databases or reading data, resource sizing (it needs load data, not code), table ownership per
service (a later plan), gRPC inference, the HTML report page and MCP (no tool in
`src/mcp_tools.rs` covers services).

## Decisions

- D1. One analyzer, two inputs. Inference builds the existing `Manifest` (`src/services.rs:13-19`)
  and hands it to `analyze` (`:258`), so cycles, hubs, instability, forbid matching and markdown are
  reused unchanged. In `archgraph services <path>` a file is a manifest as today, with identical
  output, and a directory is inferred. With no argument `./services.yaml` is read when it exists
  (today's default, `src/cli/services.rs:5-7`), else `.` is inferred. The manifest stays because it
  covers what inference cannot see: gRPC, calls across repositories, URLs known only from
  configuration. It is a second input to one analyzer, not a second implementation.
- D2. `--manifest <file>` beside a directory makes the manifest an override. If any of its `services` name directories under the root, those directories replace the entire
  automatic boundary set. Unlisted directories are then unattributed. A manifest with only
  declared-only ids preserves automatic boundaries. Ids naming no directory stay
  declared-only services. Its `edges` are appended as declared evidence and its `forbid` rules judge
  every edge. A `services:` key in `layout.yaml` was rejected: layout keys are strict, its `forbid`
  judges files with another meaning, and `services.yaml` already holds service ids and rules.
- D3. A service is a directory with a deployable marker: a file named exactly `Dockerfile`, a
  `package.json` with `scripts.start`, or a Go `package main` directory
  (`source.collector.main_packages`, `src/model.rs:163-166`, `src/collect/native/go.rs:344-353`). A
  file belongs to its deepest marked ancestor. `main` in package.json is no marker, since libraries
  set it (activepieces `packages/server/worker`). `Dockerfile.dev`, `Dockerfile.base` and
  `Dockerfile.worker` are variants, not markers. Hidden directories, symlinks, dependencies, generated files, tests, fixtures and examples
  are skipped using the existing source policy. Finer automatic boundaries use a package/project
  root containing a bound runtime startup (server listen, NestFactory bootstrap, or a top-level
  queue worker), even when it lacks scripts.start. Without a nested package root, use the startup file's top-level subtree. A lone src directory
  remains part of the containing root. Go requires package main and func main. A route, dependency, main field or
  class alone never marks a service. Root Dockerfiles are fallback boundaries. When nested applications exist, a root image
  without its own startup or application facts outside child packages is dropped. Serve scripts require a known server launcher or a runtime executing source under src.
  Build-only serve scripts are excluded. Explicit turbo, nx or concurrently orchestration does
  not keep an aggregate root alive when it has no separate root startup. Single-command root startup
  scripts that target build/server or dist/server map to an existing source server directory. Root-name/directory-id collisions fail clearly. The id is the relative directory, and the root's own folder name for the root (ids reject
  `.`, `src/services.rs:178`). A file under no marker is unattributed. Its facts are counted in a
  caveat and never guessed into a service. Composition roots from `wiring` (`src/graph.rs:337-388`)
  are no markers: a root is any file nobody imports that imports `ROOT_MIN_MODULES` (3) modules
  (`:17`), which includes scripts and CLIs.
- D4. Collect eligible files across the whole repository, regardless of a root `src/` directory or
  the dominant language. Use the native TypeScript collector with source directory `.` and the
  existing policy for import identities and runtime file edges. Go facts reuse its private syntax
  parser without requiring go list or installing dependencies. A services-only second parse returns
  shared `AppFacts` and typed facts (database, cache, queue, HTTP, route, publish, subscribe), source
  lines, optional literal operation/method, and template uncertainty. The collector, its cache and
  scan JSON stay unchanged. Bound import aliases exclude type-only imports and conservatively
  reject shadowing. Track same-file constructed router/client receivers and Kafka producer/consumer
  chains. Unrelated ident.get calls are not routes. No cross-file receiver or injection tracking.
- D5. A fixed table maps `(package, exported name, shape)` to a resource kind. `database`: `pg`
  `new Pool`/`new Client`, `postgres()`, `mysql2` `createPool`, `mongodb` `new MongoClient`,
  `@prisma/client` `new PrismaClient`, `typeorm` `new DataSource`, `sequelize` `new Sequelize`,
  `kysely` `new Kysely`, `knex()`. `cache`: `ioredis` `new Redis`, `redis` `createClient`. `queue`:
  `kafkajs` `new Kafka`, `bullmq` `new Queue`/`new Worker`, `amqplib` `connect`,
  `@aws-sdk/client-sqs` `new SQSClient`. `http`: `axios` verbs, unbound `fetch`. Routes:
  `@nestjs/common` `@Controller(p)` with `@Get|Post|Put|Patch|Delete(p)`, and, in a file importing
  `express`, `fastify`, `koa-router`, `@koa/router` or `hono`, `<ident>.<verb>(<literal starting with
  />, handler)`. Topics: the first argument of `new Queue`/`new Worker`, the `topic` of kafkajs
  `send`/`subscribe`. A URL, route or topic is read only from a string literal (the path of an
  absolute URL), or from the static tail of a template literal after one leading substitution
  (`${base}/users`). Anything else keeps the fact without an operation. CommonJS destructuring
  (`const { Pool } = require('pg')`) is not bound, since `parse` records no names for `require`
  (`:496-520`).
- D6. Report additions are optional and skipped when empty, so manifest reports keep their bytes:
  `resources: [{service, kind, package, operation?, confidence, source}]`, `summary.resources`, and
  `source.collector` as `declarative`, `inferred` or `inferred+declarative`. Caveats depend on the
  mode. Inferred file and line are read from code, so "supplied by the manifest and are not
  verified" (`src/services.rs:403-407`) applies to declared edges only. Markdown is titled `Inferred
  service architecture` and gains `## Infrastructure`. Resource rules explicitly use `to: pkg:<package glob>`, for example `pkg:pg`. Only these
  targets judge resources. Ordinary service globs continue to judge only calls. Resource verdicts
  and optional rule ids, `forbidden_resources` indices and `summary.forbidden_resources` stay
  separate from call metrics. Resource rules with `kinds` match HTTP/queue resources only through their transport.
  Omit `kinds` for database/cache rules. Existing manifest validation stays compatible. All new empty fields are omitted.
- D7. Confidence. A resource constructed inside its service is `high`. One constructed in an
  unattributed file attaches as `medium` to each service with a file importing that file at runtime
  (`doc.edges`). A cross-service edge is `high` when method and literal path equal a route's, or the
  topic equals a consumer's. It is `medium` when they are equal only after parameter segments
  (`:id`, `{id}`, `${...}`, digits) become wildcards, or the URL came from a template. A path or
  topic matching two or more other services makes no edge and a caveat. An unmatched call stays an
  `http` resource, an external endpoint. Inferred edges are never `low`, the manifest default.
- D8. Injection tracking is dropped. Attribution needs only where a client is constructed (D3) and,
  for shared files, who imports them (D7). Which component inside a service receives a client
  changes no service edge. Table ownership can add it later if it needs it.
- D9. TypeScript, JavaScript and Go clients and calls are required before the real-repository gate.
  Go table: database/sql Open/OpenDB; pgx Connect and pgxpool New/Connect; go-redis NewClient and
  NewClusterClient; nats Connect with same-file Publish/Subscribe; net/http Get/Post/PostForm,
  Client methods, and NewRequest only when passed to Do. Routes use Handle/HandleFunc or a bound
  ServeMux, with method-prefixed patterns exact and methodless patterns ANY/medium. Imports use
  explicit aliases or known package names (including versioned paths), reject shadowing, dot and
  blank imports. Go service starts require package main plus func main. Python remains conditional P5.
- D10. HTTP matching requires a known method or an ANY route. Unknown methods never default to GET.
  Normalize parameter segments conservatively, preserve template uncertainty, compare only other
  services, and deduplicate identical inferred calls before analyze. Ambiguity emits no edge.
  Remote literal hosts that do not identify an inferred service cannot match local routes just
  because their paths match. Manifest override input and positional manifest input are protected
  from output overwrites, including canonical/symlink aliases.

## Revision acceptance checks

- V1. Capture fixed pre-change JSON and Markdown bytes for examples/services.yaml, then assert exact
  compatibility. Preserve all existing manifest errors and brief/full export behavior.
- V2. Test root src plus sibling services, mixed TS/Go, policy exclusions, generated declarations,
  import aliases/type-only/shadowed names, unrelated receivers and local Kafka chains.
- V3. Test automatic subpackage/runtime boundaries under one root Dockerfile, worker startup without
  scripts.start, no library service, fallback root pruning, deterministic manifest replacement and
  declared-only manifests.
- V4. Test infrastructure pkg: rules, first match, separate verdicts/counts, resource transport
  filters, and old service wildcard behavior unchanged. Test Go routes and outbound calls,
  database/cache/queue clients, aliases, request dispatch and exact/ANY confidence.
- V5. Each phase receives an independent review. No commits or pushes without a fresh user request.
  P1 includes boundary discovery, TS clients and optional report/schema fields. P2 includes TS and
  Go call matching, Go clients and finer runtime startup boundaries. P3 includes manifest overrides
  and infrastructure policies. P4 measures and reviews actual pinned source. P5 runs only after P4
  passes. The phase details below are amended by D1-D10 and V1-V5.

## Phases

### P1. `feat(services): infer services and their database, cache and queue clients from TypeScript code`
- `src/infer_services.rs` (new, no trait): markers (D3), table resource rows (D5),
  `infer(root, manifest) -> Result<Report>` using the existing analyzer.
  `src/collect/native/typescript.rs` `AppFacts`, `app_facts` (D4). `src/services.rs` `Resource`,
  `Report.resources`, `Summary.resources`, collector label and caveats by mode, `## Infrastructure`.
  `src/cli/services.rs` directory input and default (D1). `src/lib.rs` module. Schema `services`
  definition (`schema/archgraph.v1.2.schema.json:4588-4621`): collector enum, `resources`,
  `summary.resources`. Guide section retitled `## Service architecture` with an inferred part first.
  CHANGELOG.
- Tests: `tests/services.rs` new `a_directory_infers_services_and_attaches_infrastructure_clients`.
  A root `package.json` declares `pg`, `ioredis`, `bullmq`. `api/Dockerfile` and `api/src/db.ts`
  `new Pool()` give database high. `worker/package.json` with `scripts.start` and `new
  Queue('emails')` give queue high, operation `emails`. `shared/redis.ts` `new Redis()` imported by
  `api/src/app.ts` gives cache medium on `api`. `lib/seed.ts` `new Pool()` imported by nothing gives
  the unattributed caveat. `api/Dockerfile.dev` adds nothing. No fact for a local `class Pool {}`, a
  declared `fetch`, or a `.d.ts` file. `services` is exactly `["api","worker"]` and the JSON is
  schema-valid. The existing tests prove manifest files are unchanged. `tests/brief.rs`
  `brief_services_captures_markdown_but_preserves_complete_report_export` gains a directory case.
  Run `cargo test --test services --test brief --test native_typescript --lib`.

### P2. `feat(services): match outbound HTTP calls and queue topics to routes and consumers across services`
- `app_facts` adds routes, outbound calls with their method (`axios.<verb>`, a literal `method` in
  `fetch` options, else GET) and topics (D5). `src/infer_services.rs` matching and confidence (D7),
  unmatched calls as `http` resources, the ambiguity caveat. Guide, CHANGELOG. Edges already carry
  `operation`, so the schema is unchanged.
- Tests: `tests/services.rs` new
  `outbound_calls_and_topics_become_cross_service_edges_only_when_one_service_matches`. Express
  `app.get('/users/:id', h)` in `api` and `fetch('http://api/users/42')` in `worker` give `worker ->
  api` http medium `GET /users/:id`. `@Controller('invoices')` with `@Post()` in `api` and
  `axios.post('http://api/invoices')` in `worker` give high. kafkajs `send({ topic:
  'invoice.created' })` in `api` and `subscribe({ topic: 'invoice.created' })` in `worker` give
  `api -> worker` queue high, and the pair forms a cycle. `fetch('https://api.stripe.com/v1/charges')`
  stays an `http` resource. `/health` routed in two services makes no edge. Run `cargo test --test
  services --test native_typescript`.

### P3. `feat(services): let a services manifest override inferred boundaries and judge inferred calls`
- `src/cli/services.rs` `--manifest`, with a file input exiting 2. `src/infer_services.rs` D2 merge.
  Collector `inferred+declarative`. Guide example of a `forbid` rule on inferred calls. CHANGELOG.
- Tests: `tests/services.rs` new `a_manifest_overrides_inferred_boundaries_and_its_rules_judge_inferred_calls`.
  Manifest `services: [api, worker, billing]` with declared-only `billing`, one declared edge, and
  `forbid: [{id: worker-no-api, from: worker, to: api}]`. The inferred `worker -> api` call is
  forbidden, the declared edge keeps `low`, and a manifest naming `packages/x` splits the root
  service. Run `cargo test --test services`.

### P4. `docs(plans): record inferred services on immich, activepieces and outline`
- From the repository root (services span it, unlike the import-scan `path`), run `cargo run
  --release -- services <clone> --format json -o <scratchpad>/<name>-services.json` for immich
  `3ae71117413cc0a79e4a0555df015e3ffde11786`, activepieces
  `55134042a88da01be2229e06302a78c80f209ea0` and outline `095b096c2197f35e57057e6cc6ae9d7ef562330a`.
  Markers there: immich `server`, `machine-learning` (Python, no facts before P5), `packages/cli`,
  `packages/e2e-auth-server`. activepieces and outline require finer startup-based automatic boundaries despite their root `Dockerfile`. Record under
  `## Results`: services, resources by kind, edges by confidence, unattributed facts, wall time.
  Record activepieces again with a manifest naming `packages/server/api` and
  `packages/server/worker`.
- Gate: per repository, judge every service, plus the first ten resources and the first ten edges
  sorted by `(service or from, source.file, source.line)`. More than one misread in a sample of ten,
  or a library reported as a service, fails the gate and sends the fix back to P1 or P2.

### P5. `feat(services): infer FastAPI, Flask, requests, httpx, SQLAlchemy, redis and Celery facts in Python services`
- Only if P4 passes. The repository-wide source inventory includes Python files regardless of the dominant
  language, so each Python service contributes facts without changing scan language detection. `src/collect/native/python.rs` `app_facts` on its import names (`:19`):
  `@app.get|post(p)` routes, `requests`/`httpx` verbs, `create_engine`, `psycopg.connect`,
  `redis.Redis`, `Celery(...)`, `@app.task` and `send_task(name)`. Table rows, guide, CHANGELOG.
- Tests: `tests/services.rs` new `a_python_service_contributes_routes_and_clients`: TS `api` calling
  `fetch('http://ml/predict', { method: 'POST' })` and `ml/Dockerfile` with FastAPI
  `@app.post("/predict")` give `api -> ml` http high. Rerun immich and add any `server ->
  machine-learning` edge to `## Results`. Run `cargo test --test services --test native_python`.

## Risks

- R1. Under-reporting: clients built by framework modules (`TypeOrmModule.forRoot`, NestJS
  providers), URLs and topics from configuration or enums, CommonJS destructuring, route prefixes
  set by `register(plugin, { prefix })`. Reports keep `complete: false` and the guide lists these.
- R2. Mis-attribution: startup-based boundaries can miss indirect bootstraps. A shared client
  attaches to every service directly importing its file. P4 checks automatic boundaries before
  manifest overrides. Unknown wiring remains unattributed rather than guessed.
- R3. Generic paths (`/`, `/health`, `/api`) link the wrong pair when only one other service routes
  them. P4 measures it.
- R4. Cost: a directory runs a full native scan with `--symbols` plus a second parse of client and
  `fetch` files. P4 records wall time.
- R5. Report shape: manifest reports must keep their bytes. New fields skip when empty and the
  existing `tests/services.rs` cases run unchanged.

## Results

P4 passed independent source review on 2026-10-25 after correcting aggregate-root attribution.

Sources are pinned codeload archives at the commits in P4. No project code or infrastructure ran.
All service boundaries and the first ten resources and edges, sorted by service or caller and
source file and line, were reviewed. Each available sample had zero misreads after the fixes.

| Repository | Services | Database | Cache | Queue | HTTP | Calls | Unattributed facts | Wall seconds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| immich | 5 | 2 | 2 | 1 | 8 | 0 | 8 | 2.410 |
| activepieces | 4 | 5 | 4 | 11 | 18 | 0 | 87 | 24.488 |
| outline | 1 | 0 | 0 | 0 | 2 | 0 | 5 | 2.905 |
| activepieces-override | 2 | 4 | 4 | 11 | 11 | 0 | 95 | 24.018 |

Immich boundaries are `docs`, `machine-learning`, `packages/cli`, `packages/e2e-auth-server`, and
`server`. Activepieces boundaries are `packages/cli`, `packages/server/api`,
`packages/server/worker`, and `packages/web`. Outline has `server`. The activepieces override
replaces all automatic boundaries with API and worker. Documentation and CLI packages are
runnable applications, not inferred library boundaries.

All four reports contain zero service edges. The samples establish no real-repository edge
precision or recall. Fixture tests establish matching behavior. Outline has two medium HTTP
candidates in `shared/editor/lib/FileHelper.ts`, directly imported by backend storage code.
The backend uses other methods in that file, so these rows do not prove backend HTTP execution.

Timings are one observed release run each on a shared host, with filesystem caches retained.
They include collection, fact parsing, inference and JSON serialization, and exclude compilation
and source retrieval. They are not a comparative performance claim. An initial Immich run was
aborted after profiling found repeated recursive receiver resolution. The fix resolves each
receiver once and bounds unsupported chains, with a deterministic regression test.

P4 binary SHA-256: `57295ff7411e389811e53cf2e40e14f8992b40bc1793b8380930a4c03c9d3f85`. Raw reports, sorted source samples, archive hashes,
and the rerunnable measurement script are in `/private/tmp/archgraph-app-model-validation/`.
The first automatic activepieces sample failed because the aggregate root claimed benchmark
HTTP calls. Explicit workspace orchestration now removes that root, while a standalone root
image retains its own application evidence.

P5 adds Python facts without changing service boundaries. The release Immich rerun took 2.894
seconds in one observed run and found the same five services, 14 resources, and zero service
edges. The additional resource is `requests` in
`machine-learning/scripts/healthcheck.py:22`, attributed to `machine-learning` with high
confidence. No server-to-machine-learning edge was inferred. Dynamic URLs and conservative
same-name shadow suppression still hide some Python evidence.

P5 validation: `cargo test --quiet` passes 880 tests with 38 existing ignored tests and no
failures. The suite needs ordinary Unix-socket access outside the execution sandbox.
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and all 111 UI tests pass.
The final independent code review approves the implementation. Fixed JSON and Markdown fixtures
prove that manifest-file output is unchanged. No dependencies were added.

Publication follow-up: GitHub's live Go parity test exposed a preexisting dependency on the
host platform's file order. Declaration variants now have deterministic pairwise move-together
constraints. Excluding one variant retains the link between every surviving pair, preventing
individual mechanical moves of mutually exclusive declarations. The two affected collector
recordings were regenerated, and parity checks cover Darwin, Linux, and Windows file selection.
