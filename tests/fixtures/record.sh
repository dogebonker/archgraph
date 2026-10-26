#!/bin/sh
# Regenerates every recorded collector output under tests/fixtures.
#   BOUNDRAIL_DEPCRUISE  path to depcruise 18.4.0 (typescript 6.0.3 installed beside it)
#   BOUNDRAIL_PYTHON     python with grimp 3.17
#   BOUNDRAIL_GO         go 1.27.1
# The depcruise arguments and the grimp and Go helpers are the ones `boundrail scan` uses.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
depcruise=${BOUNDRAIL_DEPCRUISE:-depcruise}
python=${BOUNDRAIL_PYTHON:-python3}
go=$(command -v "${BOUNDRAIL_GO:-go}")

# The options of depcruise::options in src/collect/depcruise.rs (every fixture has a tsconfig).
config=$(mktemp "${TMPDIR:-/tmp}/boundrail-record.XXXXXX")
trap 'rm -f "$config"' EXIT
printf '%s' '{ "options": { "doNotFollow": { "path": "node_modules" }, "tsPreCompilationDeps": "specify", "tsConfig": { "fileName": "tsconfig.json" } } }' >"$config"

for fixture in ts-basic ts-cycles ts-excludes ts-fda ts-modmono ts-hexagonal \
  ts-sink ts-misplaced ts-internals ts-sdp ts-seams ts-elided; do
  (
    cd "$here/$fixture"
    "$depcruise" --output-type json --config "$config" src > depcruise.json
    # baseDir and rulesFile are absolute recording paths; drop them so the file is portable.
    node -e '
      const fs = require("fs");
      const doc = JSON.parse(fs.readFileSync("depcruise.json", "utf8"));
      delete doc.summary.optionsUsed.baseDir;
      delete doc.summary.optionsUsed.rulesFile;
      fs.writeFileSync("depcruise.json", JSON.stringify(doc, null, 2) + "\n");
    '
  )
done

# fixture, source root, package
for args in "py-cycles . mypkg" "py-clean src app" "py-deferred . deferred"; do
  set -- $args
  (
    cd "$here/$1"
    "$python" -I -B -c "$(cat "$here/../../src/collect/grimp_dump.py")" "$2" "$3" . > grimp.json
  )
done

# The environment of go::run in src/collect/go.rs; the helper runs from an empty directory outside
# the fixture.
gotmp=$(mktemp -d "${TMPDIR:-/tmp}/boundrail-record-go.XXXXXX")
trap 'rm -f "$config"; rm -rf "$gotmp"' EXIT
cp "$here/../../src/collect/go_dump.go" "$gotmp/"
for fixture in go-resolve go-cycles go-misplaced go-layers go-internals go-excludes; do
  (
    cd "$gotmp"
    # One argument per module directory, as module_dirs in src/collect/go.rs finds them.
    # shellcheck disable=SC2046
    GOTOOLCHAIN=local GOFLAGS=-mod=readonly CGO_ENABLED=0 GOWORK=off GOPROXY=off \
      "$go" run go_dump.go "$here/$fixture" "$go" \
      $(cd "$here/$fixture" && find . -name go.mod -not -path '*/vendor/*' -not -path '*/testdata/*' |
        sed 's|^\./||; s|/\{0,1\}go\.mod$||; s|^$|.|') > "$here/$fixture/go.json"
  )
done
