#!/usr/bin/env bash
# Builds the boundrail showcase: for every entry of repos.json, a shallow fetch of its pinned
# commit, an boundrail scan of its `path`, the rendered report and a stats row in index.json, then
# index.html from index.template.html.
#
#   showcase/build.sh [--only <name>] [--work <dir>] [--out <dir>] [--keep] [--compare <dir>]
#
#   BOUNDRAIL            boundrail binary (default: target/release/boundrail of this repository)
#   BOUNDRAIL_DEPCRUISE  dependency-cruiser executable for TypeScript entries (required for them)
#   BOUNDRAIL_PYTHON     Python with grimp 3.17 for Python entries (default: python3)
#   BOUNDRAIL_GO         Go 1.27.1 for Go entries (default: go)
#
# --compare <dir> then compares the config, files, edges, cycles, summary and findings of every
# scan built with <dir>/<name>.json, an earlier build's output (by another boundrail, say), and
# exits 1 when any differs or is missing there.
#
# Nothing is installed into the targets. Every entry is validated first; a bad one stops the build
# (exit 2). A failed repository is recorded in index.json and the others still run; the exit status
# is 1 when any failed. Work trees are deleted after their build, failed or not, and on interrupt,
# unless --keep is given; a kept tree at the pinned commit is reused on the next run.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
manifest=$here/repos.json
template=$here/index.template.html
boundrail=${BOUNDRAIL:-$here/../target/release/boundrail}
python=${BOUNDRAIL_PYTHON:-python3}
depcruise=${BOUNDRAIL_DEPCRUISE:-}
go=${BOUNDRAIL_GO:-go}
work=${TMPDIR:-/tmp}/boundrail-showcase
out=$here/out
only=
keep=
compare=

die() {
  echo "build.sh: $*" >&2
  exit 2
}

while (($#)); do
  case $1 in
    --only) only=${2:?--only needs a name}; shift 2 ;;
    --work) work=${2:?--work needs a directory}; shift 2 ;;
    --out) out=${2:?--out needs a directory}; shift 2 ;;
    --keep) keep=1; shift ;;
    --compare) compare=${2:?--compare needs a directory}; shift 2 ;;
    -h | --help) sed -n '2,19p' "$0"; exit 0 ;;
    *) die "unknown option $1" ;;
  esac
done

command -v jq >/dev/null || die "jq is required"
command -v git >/dev/null || die "git is required"
[[ -x $boundrail ]] || die "no boundrail binary at $boundrail (cargo build --release, or set BOUNDRAIL)"
work=${work%/}
mkdir -p "$work/.stats" "$out"
work=$(cd "$work" && pwd)
out=$(cd "$out" && pwd)
# Logs and per-repository stats stay beside the clones, so --out holds exactly the site:
# index.html, index.json and <name>.html and <name>.json per repository.
stats=$work/.stats

# Every entry is checked before anything runs: names become file names and paths, repo is
# fetched and flags reach boundrail. Any bad entry stops the build.
validate_manifest() {
  local entry name repo i
  local -a flags
  jq -e 'type == "array" and all(.[]; type == "object" and (.name | type) == "string"
    and (.repo | type) == "string" and (.flags | type) == "array"
    and all(.flags[]; type == "string"))' "$manifest" >/dev/null ||
    die "$manifest must be an array of entries with string name, repo and flags"
  jq -e '[.[].name] | length == (unique | length)' "$manifest" >/dev/null ||
    die "names in $manifest must be unique"
  while IFS= read -r entry; do
    name=$(jq -r .name <<<"$entry")
    repo=$(jq -r .repo <<<"$entry")
    [[ $name =~ ^[a-z0-9][a-z0-9._-]*$ && $name != *..* ]] || die "bad name: $name"
    [[ $repo =~ ^https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
      die "$name: repo must be https://github.com/<owner>/<name>"
    flags=()
    while IFS= read -r f; do flags+=("$f"); done < <(jq -r '.flags[]' <<<"$entry")
    # Only the flags that choose what is scanned; --collector-input, --rules and the rest stay
    # out of a manifest.
    i=0
    while ((i < ${#flags[@]})); do
      case ${flags[i]} in
        --include-tests | --include-generated | --no-default-excludes) i=$((i + 1)) ;;
        --depth | --exclude | --package | --granularity)
          [[ $((i + 1)) -lt ${#flags[@]} && ${flags[i + 1]} != -* ]] ||
            die "$name: ${flags[i]} needs a value"
          [[ ${flags[i]} != --depth || ${flags[i + 1]} =~ ^[0-9]+$ ]] ||
            die "$name: --depth takes a number"
          [[ ${flags[i]} != --granularity || ${flags[i + 1]} == file || ${flags[i + 1]} == folder ]] ||
            die "$name: --granularity takes file or folder"
          i=$((i + 2)) ;;
        *) die "$name: flag ${flags[i]} is not allowed" ;;
      esac
    done
  done < <(jq -c '.[]' "$manifest")
}
validate_manifest

if [[ -n $only ]] && ! jq -e --arg n "$only" 'any(.[]; .name == $n)' "$manifest" >/dev/null; then
  die "no entry named $only in $manifest"
fi

# One repository: fetch, scan, render, stats. Prints why it failed and returns 1 on failure.
build_one() {
  local entry=$1 name repo commit path language dir log start end
  local -a flags=()
  name=$(jq -r .name <<<"$entry")
  repo=$(jq -r .repo <<<"$entry")
  commit=$(jq -r .commit <<<"$entry")
  path=$(jq -r .path <<<"$entry")
  language=$(jq -r .language <<<"$entry")
  while IFS= read -r f; do flags+=("$f"); done < <(jq -r '.flags[]' <<<"$entry")
  [[ $commit =~ ^[0-9a-f]{40}$ ]] || { echo "commit is not a full SHA"; return 1; }
  [[ $path != /* && /$path/ != */../* ]] || { echo "path must stay inside the repository"; return 1; }

  dir=$work/$name
  log=$stats/$name.log
  : >"$log" || return 1
  if [[ ! -d $dir/.git ]] || [[ $(git -C "$dir" rev-parse HEAD 2>/dev/null) != "$commit" ]]; then
    rm -rf "$dir" || return 1
    mkdir -p "$dir" || return 1
    # LFS objects are never needed: imports live in source files.
    if ! { git -C "$dir" init -q &&
      git -C "$dir" remote add origin "$repo" &&
      GIT_LFS_SKIP_SMUDGE=1 git -C "$dir" fetch -q --depth 1 origin "$commit" &&
      GIT_LFS_SKIP_SMUDGE=1 git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD; } >>"$log" 2>&1; then
      echo "fetch failed: $(tail -n 3 "$log" | tr '\n' ' ')"
      return 1
    fi
  fi

  local -a collector=()
  case $language in
    typescript)
      [[ -n $depcruise ]] || { echo "BOUNDRAIL_DEPCRUISE is not set"; return 1; }
      collector=(--depcruise "$depcruise") ;;
    python) collector=(--python "$python") ;;
    go) collector=(--go "$go") ;;
    *) echo "unsupported language $language"; return 1 ;;
  esac

  # EPOCHREALTIME (bash 5) uses the locale's decimal separator.
  start=${EPOCHREALTIME:-$(date +%s)}
  start=${start/,/.}
  if ! "$boundrail" scan "$dir/$path" --language "$language" "${collector[@]}" "${flags[@]}" \
    -o "$out/$name.json" >>"$log" 2>&1; then
    echo "scan failed: $(grep -v '^warning:' "$log" | tail -n 3 | tr '\n' ' ')"
    return 1
  fi
  end=${EPOCHREALTIME:-$(date +%s)}
  end=${end/,/.}
  if ! "$boundrail" render "$out/$name.json" -o "$out/$name.html" >>"$log" 2>&1; then
    echo "render failed: $(tail -n 3 "$log" | tr '\n' ' ')"
    return 1
  fi

  jq --arg seconds "$(awk -v a="$start" -v b="$end" 'BEGIN { printf "%.1f", b - a }')" '{
    files: (.files | length),
    modules: (.modules | length),
    imports: (.edges | length),
    cruised: .source.collector.cruised,
    unresolved: .source.collector.unresolved,
    import_resolution: .import_resolution,
    external_packages: ((.external_nodes // []) | length),
    collector: { name: .source.collector.name, version: .source.collector.version },
    boundrail: .tool.version,
    depth: .config.depth,
    source_root: .config.source_root,
    module_cycles: ([.cycles[] | select(.level == "module")] | length),
    largest_module_cycle: ([.cycles[] | select(.level == "module") | .members | length] | max // 0),
    file_cycles: ([.cycles[] | select(.level == "file")] | length),
    forbidden: .summary.forbidden,
    findings: (.findings | group_by(.kind) | map({ key: .[0].kind, value: length }) | from_entries),
    scan_seconds: ($seconds | tonumber)
  }' "$out/$name.json" >"$stats/$name.json.tmp" || { echo "stats failed"; return 1; }
  mv "$stats/$name.json.tmp" "$stats/$name.json" || return 1
}

# The work tree being built, removed after its build whatever the outcome (and on interrupt)
# unless --keep. build_one runs in a subshell, so the caller cleans up.
current=
cleanup() {
  if [[ -z $keep && -n $current ]]; then
    rm -rf "$current"
  fi
  current=
}
trap cleanup EXIT
trap 'exit 130' INT TERM

failed=0
while IFS= read -r entry; do
  name=$(jq -r .name <<<"$entry")
  [[ -z $only || $name == "$only" ]] || continue
  echo "== $name" >&2
  rm -f "$stats/$name.json" "$stats/$name.error"
  current=$work/$name
  if ! reason=$(build_one "$entry"); then
    failed=1
    echo "   $reason" >&2
    printf '%s\n' "$reason" >"$stats/$name.error"
    rm -f "$out/$name.json" "$out/$name.html"
  else
    jq -r '"   \(.files) files, \(.modules) modules, \(.imports) imports, \(.unresolved // 0) unresolved, \(.scan_seconds)s"' \
      "$stats/$name.json" >&2
  fi
  cleanup
done < <(jq -c '.[]' "$manifest")

# index.json: every manifest entry, with its stats, its failure, or "not built" (--only runs keep
# the other entries' earlier results).
while IFS= read -r entry_name; do
  if [[ -f $stats/$entry_name.json ]]; then
    jq -n --arg n "$entry_name" --slurpfile s "$stats/$entry_name.json" '{ name: $n, status: "ok", stats: $s[0] }'
  elif [[ -f $stats/$entry_name.error ]]; then
    jq -n --arg n "$entry_name" --rawfile e "$stats/$entry_name.error" '{ name: $n, status: "failed", error: ($e | rtrimstr("\n")) }'
  else
    jq -n --arg n "$entry_name" '{ name: $n, status: "not built" }'
  fi
done < <(jq -r '.[].name' "$manifest") | jq -s --slurpfile m "$manifest" \
  --arg generated "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg commit "$(git -C "$here/.." rev-parse HEAD 2>/dev/null || echo unknown)" \
  --arg version "$("$boundrail" --version | awk '{ print $2 }')" '
  (map({ (.name): . }) | add) as $r
  | {
      generated_at: $generated,
      boundrail: { version: $version, commit: $commit },
      collectors: ([.[].stats.collector | select(.)] | unique),
      cochange: false,
      repos: [$m[0][] | . + $r[.name]]
    }' >"$out/index.json"

# The page: index.json embedded as JSON with every `<` escaped, as render.rs does.
jq -rn --rawfile tpl "$template" --slurpfile idx "$out/index.json" \
  '$tpl | split("{{INDEX}}") | if length == 2 then join($idx[0] | tojson | gsub("<"; "\\u003c")) else error("template needs exactly one {{INDEX}}") end' \
  >"$out/index.html"

echo "wrote $out/index.html" >&2

# What a change to boundrail may not move: config, files, edges, cycles, summary and findings.
# Left out: schema, kind, tool, source (collector, git, generated_at), package_entries, modules,
# module_edges and cochange.
if [[ -n $compare ]]; then
  changed=0
  while IFS= read -r name; do
    [[ -z $only || $name == "$only" ]] || continue
    [[ -f $out/$name.json ]] || continue
    if [[ ! -f $compare/$name.json ]]; then
      echo "compare: $name is missing from $compare" >&2
      changed=1
    elif ! cmp -s <(jq -S '{config, files, edges, cycles, summary, findings}' "$compare/$name.json") \
      <(jq -S '{config, files, edges, cycles, summary, findings}' "$out/$name.json"); then
      echo "compare: $name differs from $compare/$name.json" >&2
      changed=1
    else
      echo "compare: $name unchanged" >&2
    fi
  done < <(jq -r '.[].name' "$manifest")
  ((changed == 0)) || failed=1
fi
exit "$failed"
