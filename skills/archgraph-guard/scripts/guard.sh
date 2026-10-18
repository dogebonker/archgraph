#!/usr/bin/env bash
# Compare the working tree with its base commit; exit 1 when the change adds a forbidden import or
# an import cycle (new unplanned imports are listed; ARCHGRAPH_FAIL_ON can make them fail too).
# Prints a header line and the diff as markdown. Exit 0 pass, 1 new violations, 2 could not run.
#
#   guard.sh [--hook]
#
# Environment:
#   ARCHGRAPH_BASE       base commit or ref (default: merge-base with origin/HEAD, else main, else master)
#   ARCHGRAPH_PATH       project folder inside the repository (default: the repository root)
#   ARCHGRAPH_RULES      rules file (default: <project>/layout.yaml when it exists)
#   ARCHGRAPH_SCAN_ARGS  extra scan flags, e.g. "--language python --python .venv/bin/python --depth 1"
#   ARCHGRAPH_FAIL_ON    diff conditions that fail (default: new-forbidden,new-cycles)
#   ARCHGRAPH            archgraph executable (default: archgraph on PATH)
#
# --hook: Claude Code Stop hook mode. Reads the hook JSON on stdin and lets the agent stop when the
# hook already blocked this stop once. New violations go to stderr with exit 2, which keeps the
# agent working on them; a guard that cannot run exits 1, which reports without blocking.
set -euo pipefail

hook=
fail() {
  echo "guard: $*" >&2
  if [[ -n $hook ]]; then exit 1; fi
  exit 2
}
trap 'fail "unexpected error at line $LINENO"' ERR

if [[ ${1:-} == --hook ]]; then
  hook=1
  if grep -q '"stop_hook_active"[[:space:]]*:[[:space:]]*true' 2>/dev/null; then exit 0; fi
fi

archgraph=${ARCHGRAPH:-archgraph}
command -v "$archgraph" >/dev/null || fail "archgraph is not installed"
repo=$(git rev-parse --show-toplevel 2>/dev/null) || fail "not inside a git repository"
project=${ARCHGRAPH_PATH:-.}
project=${project#./}
[[ $project == . ]] && project=
prefix=${project:+$project/}
dir=$repo${project:+/$project}
[[ -d $dir ]] || fail "$project is not a folder in $repo"

base=${ARCHGRAPH_BASE:-}
if [[ -z $base ]]; then
  for ref in origin/HEAD main master; do
    if base=$(git -C "$repo" merge-base HEAD "$ref" 2>/dev/null); then break; fi
  done
fi
[[ -n $base ]] || fail "no base commit; set ARCHGRAPH_BASE"
base=$(git -C "$repo" rev-parse --verify --quiet "$base^{commit}") || fail "base ${ARCHGRAPH_BASE:-} is not a commit"
short=$(git -C "$repo" rev-parse --short "$base")

notes=()
if [[ $base == "$(git -C "$repo" rev-parse HEAD)" ]]; then
  notes+=("Note: the base is HEAD itself, so only uncommitted changes are checked; set ARCHGRAPH_BASE to include committed work.")
fi

# Nothing to compare: no tracked change and no new file in the project since the base.
if git -C "$repo" diff --quiet "$base" -- "${project:-.}" \
  && [[ -z $(git -C "$repo" ls-files --others --exclude-standard -- "${project:-.}") ]]; then
  echo "archgraph guard: no changes in ${project:-the repository} against $short"
  exit 0
fi

# Rules: the explicit file, else the project's layout.yaml as it is now, compared with the base's.
rules=${ARCHGRAPH_RULES:-$dir/layout.yaml}
[[ $rules == /* ]] || rules=$(pwd -P)/$rules
rel=${rules#"$repo/"}
in_base=
if [[ $rel != /* ]] && git -C "$repo" cat-file -e "$base:$rel" 2>/dev/null; then in_base=1; fi
if [[ -f $rules ]]; then
  if [[ -z $in_base ]]; then
    [[ $rel == /* ]] || notes+=("Note: this change adds the rules file $rel, so the base had no rules to compare against.")
  elif ! git -C "$repo" show "$base:$rel" | cmp -s - "$rules"; then
    notes+=("Note: this change edits the rules file $rel, so imports a removed or loosened rule forbade do not show as new.")
  fi
else
  [[ -z $in_base ]] || notes+=("Note: this change deletes the rules file $rel, so nothing checks the rules any more.")
  [[ -z ${ARCHGRAPH_RULES:-} ]] || fail "rules file $rules does not exist"
  rules=
fi

# Word-split the extra flags without glob expansion (they may hold --exclude patterns).
set -f
# shellcheck disable=SC2206 # word splitting is intended; globbing is off
extra=(${ARCHGRAPH_SCAN_ARGS:-})
set +f
args=(${extra[@]+"${extra[@]}"})
[[ -z $rules ]] || args+=(--rules "$rules")
# The base worktree has no node_modules: run the repository's own dependency-cruiser by path.
if [[ " ${extra[*]-} " != *" --depcruise "* && -x $dir/node_modules/.bin/depcruise ]]; then
  args+=(--depcruise "$dir/node_modules/.bin/depcruise")
fi

tmp=$(mktemp -d)
# shellcheck disable=SC2317,SC2329 # run by the EXIT trap
cleanup() {
  git -C "$repo" worktree remove --force "$tmp/base" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT
git -C "$repo" -c core.fsmonitor=false -c core.hooksPath=/dev/null \
  worktree add --detach --quiet "$tmp/base" "$base" || fail "could not check out $short"
# tsconfig `extends` and path aliases may point into node_modules.
if [[ -d $dir/node_modules && ! -e $tmp/base/${prefix}node_modules ]]; then
  ln -s "$dir/node_modules" "$tmp/base/${prefix}node_modules"
fi

"$archgraph" scan "$dir" ${args[@]+"${args[@]}"} -o "$tmp/head.json" 2>"$tmp/head.err" \
  || fail "scanning the working tree failed: $(cat "$tmp/head.err")"
"$archgraph" scan "$tmp/base/$project" ${args[@]+"${args[@]}"} -o "$tmp/base.json" 2>"$tmp/base.err" \
  || fail "scanning the base commit $short failed: $(cat "$tmp/base.err")"

status=0
"$archgraph" diff "$tmp/base.json" "$tmp/head.json" \
  --fail-on "${ARCHGRAPH_FAIL_ON:-new-forbidden,new-cycles}" --format markdown \
  >"$tmp/diff.md" 2>"$tmp/diff.err" || status=$?
((status <= 1)) || fail "diff failed: $(cat "$tmp/diff.err")"

# Each file entry carries exactly one "barrel" key; edges carry none.
files=$({ grep -o '"barrel":' "$tmp/head.json" || true; } | wc -l | tr -d ' ')
header="archgraph guard: $files files scanned against $short${rules:+ with rules ${rules#"$repo/"}}"

out=1
if [[ -n $hook && $status -eq 1 ]]; then out=2; fi
{
  for n in ${notes[@]+"${notes[@]}"}; do echo "$n"; done
  echo "$header"
  cat "$tmp/diff.md"
} >&"$out" || true # a reader that closes the pipe early (| head) is not a guard failure
if [[ -n $hook && $status -eq 1 ]]; then exit 2; fi
exit "$status"
