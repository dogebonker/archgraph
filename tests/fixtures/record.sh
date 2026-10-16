#!/bin/sh
# Regenerates every recorded collector output under tests/fixtures.
#   ARCHGRAPH_DEPCRUISE  path to depcruise 18.4.0 (typescript 6.0.3 installed beside it)
#   ARCHGRAPH_PYTHON     python with grimp 3.17
# The depcruise arguments and the grimp helper are the ones `archgraph scan` uses.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
depcruise=${ARCHGRAPH_DEPCRUISE:-depcruise}
python=${ARCHGRAPH_PYTHON:-python3}

# The options of depcruise::options in src/collect/depcruise.rs (every fixture has a tsconfig).
config=$(mktemp "${TMPDIR:-/tmp}/archgraph-record.XXXXXX")
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
    "$python" -I -B -c "$(cat "$here/../../src/collect/grimp_dump.py")" "$2" "$3" > grimp.json
  )
done
