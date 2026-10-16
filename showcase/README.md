# Showcase

archgraph reports on well-known open-source repositories, each scanned at a pinned commit. [build.sh](build.sh) builds them from [repos.json](repos.json) into an output folder that is exactly the published site: `<name>.html` and `<name>.json` per repository, `index.json` with per-repository stats and failures, and `index.html` from [index.template.html](index.template.html). Logs and per-repository stats stay in `--work`, under `.stats/`. The [showcase workflow](../.github/workflows/showcase.yml) builds it on pushes to main that touch `showcase/`, `assets/graph.html` or `src/`, or by hand, and publishes it with GitHub Pages; a repository that fails fails the run.

## Rebuild locally

Needs git, jq, an archgraph binary, dependency-cruiser 18.4.0 with typescript 6.0.3 beside it, a Python with grimp 3.17 and Go 1.27.1 (`go` on PATH, or `ARCHGRAPH_GO`):

```sh
cargo build --release
npm install --prefix /opt/archgraph-tools dependency-cruiser@18.4.0 typescript@6.0.3
python3 -m venv /opt/archgraph-py && /opt/archgraph-py/bin/pip install grimp==3.17

ARCHGRAPH_DEPCRUISE=/opt/archgraph-tools/node_modules/.bin/depcruise \
ARCHGRAPH_PYTHON=/opt/archgraph-py/bin/python \
  showcase/build.sh                     # all repositories into showcase/out (git-ignored)
showcase/build.sh --only django         # one repository; index.json keeps the others' last results
showcase/build.sh --work /big/disk --out /tmp/showcase --keep
showcase/build.sh --out /tmp/after --compare /tmp/before   # exit 1 when a scan's results moved
```

`--compare <dir>` checks that a change to archgraph leaves the scans alone: after the build it compares each scan's `config`, `files`, `edges`, `cycles`, `summary` and `findings` with `<dir>/<name>.json` from an earlier build (say, `ARCHGRAPH=<old binary> showcase/build.sh --out /tmp/before`), prints `unchanged` or `differs` per repository, and exits 1 when any differs or is missing.

`ARCHGRAPH` picks another archgraph binary (default `target/release/archgraph`). Clones go under `--work` (default `$TMPDIR/archgraph-showcase`) and are deleted after their scan unless `--keep` is given; a kept clone at the pinned commit is reused. A repository that fails to fetch, scan or render is recorded in `index.json` with the reason, the others still build, and the exit status is 1. Its clone is deleted too unless `--keep`, as is the one in progress when the build is interrupted. Before anything runs, every entry is checked (name, repo and flags as below); one bad entry stops the build with exit 2.

The scans run each repository's `tsconfig.json` through dependency-cruiser, so treat the targets as you would any code you run tooling on (see the warning in the main README). Nothing is installed into them: npm packages stay unresolved, and the clones are shallow, so there is no `--cochange`.

## How commits are pinned

Each entry's `commit` is the full SHA of the default branch's HEAD when the entry was added (`git ls-remote <repo> HEAD`). `build.sh` fetches exactly that commit (`git init`, `git fetch --depth 1 origin <sha>`, checkout), so a rebuild scans the same tree until someone changes the SHA. To move a repository forward, replace its `commit` with a new `ls-remote` result and rebuild it with `--only`.

## Add a repository

Append an entry to `repos.json`:

```json
{
  "name": "flask",
  "repo": "https://github.com/pallets/flask",
  "commit": "<full SHA from git ls-remote https://github.com/pallets/flask HEAD>",
  "path": ".",
  "language": "python",
  "flags": ["--package", "flask", "--depth", "1"],
  "why": "One line: what makes it interesting."
}
```

- `name`: lowercase letters, digits, `.`, `_`, `-`, starting with a letter or digit, without `..`, unique; it names the output files.
- `repo`: `https://github.com/<owner>/<name>`.
- `path`: the folder to scan inside the repository, the `archgraph scan` root; in a monorepo, one package. archgraph cruises its `src/` when it has one.
- `language`: `typescript`, `python` or `go`.
- `flags`: extra `archgraph scan` flags, only `--depth <n>`, `--exclude <glob>`, `--package <name>`, `--include-tests` and `--no-default-excludes`. Pick `--depth` so modules are meaningful: open the report, and when most files sit in a handful of modules or every folder is its own module, change it.
- Check the scan before keeping the entry: `unresolved` in `index.json` counts imports dependency-cruiser could not resolve. npm packages always land there; when relative or alias imports of the repository's own files do too (a `tsconfig.json` that extends an uninstalled package, workspace aliases), most edges are missing and the report says little.
