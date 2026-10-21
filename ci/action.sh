#!/usr/bin/env bash
# The archgraph GitHub Action (action.yml). Every input arrives as an INPUT_* environment
# variable, so this runs outside Actions too, given RUNNER_TEMP, GITHUB_OUTPUT and
# GITHUB_STEP_SUMMARY:
#
#   INPUT_PATH INPUT_RULES INPUT_DEPTH INPUT_LANGUAGE INPUT_ENGINE INPUT_PACKAGE INPUT_EXCLUDE INPUT_FAIL_ON
#   INPUT_FAIL_ON_RULES_CHANGE INPUT_VERSION INPUT_BINARY INPUT_BASE_REF   as in action.yml
#   ARCHGRAPH_DEPCRUISE, ARCHGRAPH_PYTHON, ARCHGRAPH_GO   a collector to use instead of installing one
#   ARCHGRAPH_ELIXIR   a preinstalled Elixir executable (default elixir)
#
# Without a base ref: `report` on INPUT_PATH, its summary in the job summary, exit 1 as `report`
# exits (a forbidden import the known list does not hold, or a stale known entry). With one: `report` on HEAD, a scan of the base commit (in a worktree) with
# the same rules file and options, then `diff --fail-on` in markdown into the job summary, exit 1
# when the diff has a --fail-on condition. Exit 2 on errors.
set -euo pipefail
# A failed setup step (install, npm ci, venv, pip) is a failure to run, not a gate result.
trap 'exit 2' ERR

# Everything printed below may hold text from the scanned repository (paths, collector and git
# messages), which must not run as workflow commands: one stream, commands off until exit.
exec 2>&1
token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
echo "::stop-commands::$token"
worktree_added=
failure=
# shellcheck disable=SC2317,SC2329 # run by the EXIT trap
cleanup() {
  local status=$?
  if [[ -n $worktree_added ]]; then
    git -C "$repo" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  fi
  echo "::$token::"
  # Exit 1 is a gate result the summary explains; 2 and above is a failure to run.
  if ((status >= 2)); then
    echo "::error::archgraph: ${failure:-failed with exit $status; see the log above}"
  fi
}
trap cleanup EXIT
die() {
  failure="$*"
  exit 2
}

here=$(cd "$(dirname "$0")/.." && pwd)
: "${RUNNER_TEMP:?}" "${GITHUB_OUTPUT:?}" "${GITHUB_STEP_SUMMARY:?}"

path=${INPUT_PATH:-.}
[[ -d $path ]] || die "path $path is not a directory"
lang=${INPUT_LANGUAGE:-}
engine=${INPUT_ENGINE:-reference}
case $engine in
  reference | native | depcruise) ;;
  *) die "engine must be reference, native or depcruise" ;;
esac
fail_on=${INPUT_FAIL_ON-new-forbidden,new-cycles,stale-known,new-vacuous}
fail_on=${fail_on//[[:space:]]/}
base_ref=${INPUT_BASE_REF:-}
work=$RUNNER_TEMP/archgraph
rm -rf "$work"
mkdir -p "$work"

if [[ -n ${INPUT_BINARY:-} ]]; then
  archgraph=$INPUT_BINARY
  command -v "$archgraph" >/dev/null || die "binary $archgraph is not an executable"
else
  # The release this checkout of the action was cut from.
  version=${INPUT_VERSION:-$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/Cargo.toml" | head -n 1)}
  ARCHGRAPH_VERSION=$version ARCHGRAPH_INSTALL_DIR=$work/bin sh "$here/install.sh"
  archgraph=$work/bin/archgraph
fi
"$archgraph" --version

# Options shared by the HEAD report and the base scan.
# Keep reference defaults compatible with previously released binaries that lack --engine.
opts=()
if [[ $engine != reference ]]; then
  opts+=(--engine "$engine")
fi
if [[ -n ${INPUT_DEPTH:-} ]]; then
  opts+=(--depth "$INPUT_DEPTH")
fi
rules=${INPUT_RULES:-}
if [[ -z $rules && -f $path/layout.yaml ]]; then
  rules=$path/layout.yaml
fi
if [[ -n $rules ]]; then
  [[ -f $rules ]] || die "rules file $rules does not exist"
  # Absolute and physical, so the base scan reads HEAD's rules file and it compares with git's
  # top-level path.
  rules=$(cd "$(dirname "$rules")" && pwd -P)/$(basename "$rules")
  opts+=(--rules "$rules")
fi
if [[ -n $lang ]]; then
  opts+=(--language "$lang")
fi
if [[ -n ${INPUT_PACKAGE:-} ]]; then
  opts+=(--package "$INPUT_PACKAGE")
fi
# One glob per line or comma; commas inside {a,b} belong to the glob.
while IFS= read -r glob; do
  if [[ -n $glob ]]; then
    opts+=(--exclude "$glob")
  fi
done < <(printf '%s\n' "${INPUT_EXCLUDE:-}" | awk '{
  depth = 0; cur = ""
  for (i = 1; i <= length($0); i++) {
    c = substr($0, i, 1)
    if (c == "{") depth++
    else if (c == "}" && depth > 0) depth--
    if (c == "," && depth == 0) { print cur; cur = "" } else cur = cur c
  }
  print cur
}' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')

has() {
  local f
  for f in "$@"; do
    if [[ -e $path/$f ]]; then return 0; fi
  done
  return 1
}
if [[ $engine != native ]]; then
if [[ $lang == typescript ]] || { [[ -z $lang ]] && has package.json tsconfig.json; }; then
  depcruise=${ARCHGRAPH_DEPCRUISE:-}
  if [[ -z $depcruise ]]; then
    node_major=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null) ||
      die "TypeScript scans need Node 22 or later on PATH: add actions/setup-node"
    ((node_major >= 22)) ||
      die "dependency-cruiser 18.4.0 needs Node 22 or later, found $(node --version): add actions/setup-node"
    tools=$RUNNER_TEMP/archgraph-tools
    mkdir -p "$tools"
    cp "$here/ci/tools/package.json" "$here/ci/tools/package-lock.json" "$tools/"
    npm ci --prefix "$tools" --ignore-scripts --no-audit --no-fund --loglevel=error
    depcruise=$tools/node_modules/.bin/depcruise
  fi
  opts+=(--depcruise "$depcruise")
fi
if [[ $lang == python ]] || { [[ -z $lang ]] && has pyproject.toml setup.py setup.cfg; }; then
  python=${ARCHGRAPH_PYTHON:-}
  if [[ -z $python ]]; then
    venv=$RUNNER_TEMP/archgraph-venv
    python3 -m venv "$venv"
    "$venv/bin/python" -c 'import sys; sys.exit(not (3, 10) <= sys.version_info[:2] <= (3, 14))' ||
      die "grimp 3.17 has wheels for CPython 3.10 to 3.14, found $("$venv/bin/python" --version): add actions/setup-python"
    "$venv/bin/pip" install --quiet --only-binary :all: --require-hashes \
      -r "$here/ci/tools/requirements.txt"
    python=$venv/bin/python
  fi
  opts+=(--python "$python")
fi

if [[ $lang == go ]] || { [[ -z $lang ]] && has go.mod go.work; }; then
  go=${ARCHGRAPH_GO:-}
  if [[ -z $go ]]; then
    # Go 1.27.1 from go.dev, checked against these SHA-256 sums. archgraph runs it with
    # GOTOOLCHAIN=local, GOPROXY=off, GOWORK=off, CGO_ENABLED=0 and GOFLAGS=-mod=readonly.
    case $(uname -s)-$(uname -m) in
      Linux-x86_64) platform=linux-amd64 sum=63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445 ;;
      Linux-aarch64 | Linux-arm64) platform=linux-arm64 sum=3450b45a3f9ee8568792736a5c5e70a1f2e9b36c35a8f74958c03e51d7d92bec ;;
      Darwin-x86_64) platform=darwin-amd64 sum=8f8f52c6649542cf027bbc9b9c68d1ec042f9f34808a40413f0b8b3f66f3caa4 ;;
      Darwin-arm64) platform=darwin-arm64 sum=ee215d57e0ec269c60cc9ceca68e6bda321ba9ee5afe24f4b0988703c2d87d12 ;;
      *) die "no Go 1.27.1 download for $(uname -s) $(uname -m): set ARCHGRAPH_GO" ;;
    esac
    goroot=$RUNNER_TEMP/archgraph-go
    rm -rf "$goroot"
    mkdir -p "$goroot"
    curl --proto '=https' --tlsv1.2 -fsSL -o "$goroot/go.tar.gz" "https://go.dev/dl/go1.27.1.$platform.tar.gz" ||
      die "downloading Go 1.27.1 failed"
    if command -v sha256sum >/dev/null 2>&1; then
      got=$(sha256sum "$goroot/go.tar.gz" | cut -d ' ' -f 1)
    else
      got=$(shasum -a 256 "$goroot/go.tar.gz" | cut -d ' ' -f 1)
    fi
    [[ $got == "$sum" ]] || die "Go 1.27.1 download has SHA-256 $got, expected $sum; not using it"
    tar -xzf "$goroot/go.tar.gz" -C "$goroot"
    rm -f "$goroot/go.tar.gz"
    go=$goroot/go/bin/go
  fi
  opts+=(--go "$go")
fi

if [[ $lang == elixir ]] || { [[ -z $lang ]] && has mix.exs; }; then
  elixir=${ARCHGRAPH_ELIXIR:-elixir}
  if [[ $lang == elixir ]]; then
    command -v "$elixir" >/dev/null ||
      die "Elixir scans need Elixir 1.14+ / OTP 25+ on PATH: install Elixir or set ARCHGRAPH_ELIXIR"
  fi
  opts+=(--elixir "$elixir")
fi
elif [[ -n ${ARCHGRAPH_GO:-} ]]; then
  # Native Go only reads package metadata; use an explicitly provided toolchain without installing one.
  opts+=(--go "$ARCHGRAPH_GO")
fi

head=$work/head
status=0
"$archgraph" report "$path" ${opts[@]+"${opts[@]}"} --out-dir "$head" >"$work/report.txt" || status=$?
cat "$work/report.txt"
((status <= 1)) || exit "$status"
{
  echo "json=$head/archgraph.json"
  echo "html=$head/archgraph.html"
  echo "forbidden=$(jq '.summary.forbidden' "$head/archgraph.json")"
} >>"$GITHUB_OUTPUT"

# SARIF for github/codeql-action/upload-sarif, from HEAD's rules; exit 1 is "forbidden imports".
if [[ -n $rules ]]; then
  sarif_status=0
  "$archgraph" check "$head/archgraph.json" --format sarif >"$head/archgraph.sarif" || sarif_status=$?
  ((sarif_status <= 1)) || die "check --format sarif failed with exit $sarif_status"
  echo "sarif=$head/archgraph.sarif" >>"$GITHUB_OUTPUT"
fi

if [[ -z $base_ref ]]; then
  # A fence longer than any backtick run in the text, which the repository controls.
  fence='```'
  longest=$(awk '{
    line = $0
    while (match(line, /`+/)) {
      if (RLENGTH > m) m = RLENGTH
      line = substr(line, RSTART + RLENGTH)
    }
  } END { print m + 0 }' "$work/report.txt")
  while ((${#fence} <= longest)); do fence+='`'; done
  {
    echo '### archgraph report'
    echo
    echo "${fence}text"
    cat "$work/report.txt"
    echo "$fence"
  } >>"$GITHUB_STEP_SUMMARY"
  exit "$status"
fi

[[ $base_ref != -* ]] || die "base-ref $base_ref is not a commit"
repo=$(git -C "$path" rev-parse --show-toplevel)
prefix=$(git -C "$path" rev-parse --show-prefix)
if ! base=$(git -C "$repo" rev-parse --verify --quiet "$base_ref^{commit}"); then
  # A shallow checkout (fetch-depth: 1) lacks the base commit: fetch that one commit.
  git -C "$repo" fetch --no-tags --depth=1 origin "$base_ref" ||
    die "could not fetch $base_ref; check out with fetch-depth: 0"
  base=$(git -C "$repo" rev-parse --verify "FETCH_HEAD^{commit}")
fi
worktree=$RUNNER_TEMP/base
git -C "$repo" worktree remove --force "$worktree" >/dev/null 2>&1 || rm -rf "$worktree"
git -C "$repo" -c core.hooksPath=/dev/null worktree add --force --detach "$worktree" "$base"
worktree_added=1
# tsconfig `extends` and path aliases may point into node_modules.
if [[ -d $repo/${prefix}node_modules && ! -e $worktree/${prefix}node_modules ]]; then
  ln -s "$repo/${prefix}node_modules" "$worktree/${prefix}node_modules"
fi
"$archgraph" scan "$worktree${prefix:+/${prefix%/}}" ${opts[@]+"${opts[@]}"} -o "$work/base.json" ||
  die "scanning the base commit $base failed"

"$archgraph" diff "$work/base.json" "$head/archgraph.json" >"$work/diff.json"
status=0
"$archgraph" diff "$work/base.json" "$head/archgraph.json" ${fail_on:+--fail-on "$fail_on"} \
  --format markdown >"$work/diff.md" || status=$?
((status <= 1)) || exit "$status"

# Both commits are judged by HEAD's rules, so a pull request that drops a forbid rule and adds
# the import it forbade passes the diff: say so when the rules file differs from the base's.
rules_rel=
if [[ -z $rules ]]; then
  rules_rel=${prefix}layout.yaml
elif [[ $rules == "$repo"/* ]]; then
  rules_rel=${rules#"$repo"/}
fi
base_rules=$worktree/$rules_rel
rules_changed=false
if [[ -n $rules_rel ]]; then
  if [[ -n $rules && -f $base_rules ]]; then
    cmp -s "$rules" "$base_rules" || rules_changed=true
  elif [[ -n $rules || -f $base_rules ]]; then
    rules_changed=true
  fi
fi
{
  if [[ $rules_changed == true ]]; then
    echo "> [!WARNING]"
    if [[ ! -f $base_rules ]]; then
      echo "> **This pull request adds the rules file \`$rules_rel\`.** The base commit had none."
    elif [[ -z $rules ]]; then
      echo "> **This pull request deletes the rules file \`$rules_rel\`.** Nothing checks the rules any more."
    else
      echo "> **This pull request changes the rules file \`$rules_rel\`.** The diff below judges both commits by HEAD's rules, so imports that a removed or loosened rule forbade do not show as new."
      echo
      # check exits 1 on forbidden imports; `?` when the base's rules do not load.
      count() {
        rm -f "$work/under-base.json"
        "$archgraph" check "$1" --rules "$base_rules" -o "$work/under-base.json" >/dev/null || true
        jq '.summary.forbidden' "$work/under-base.json" 2>/dev/null || echo '?'
      }
      echo "| forbidden imports | under the base's rules | under HEAD's rules |"
      echo "| --- | --- | --- |"
      echo "| base | $(count "$work/base.json") | $(jq '.summary.forbidden' "$work/base.json") |"
      echo "| HEAD | $(count "$head/archgraph.json") | $(jq '.summary.forbidden' "$head/archgraph.json") |"
    fi
    if [[ ${INPUT_FAIL_ON_RULES_CHANGE:-false} == true ]]; then
      echo
      echo "Failing: \`fail-on-rules-change\` is set."
    fi
    echo
  fi
  cat "$work/diff.md"
} >"$work/summary.md"
cat "$work/summary.md"
cat "$work/summary.md" >>"$GITHUB_STEP_SUMMARY"
{
  echo "new-forbidden=$(jq '.new_forbidden | length' "$work/diff.json")"
  echo "new-cycles=$(jq '.new_cycles | length' "$work/diff.json")"
  echo "rules-changed=$rules_changed"
} >>"$GITHUB_OUTPUT"
if [[ $rules_changed == true && ${INPUT_FAIL_ON_RULES_CHANGE:-false} == true ]]; then
  status=1
fi
exit "$status"
