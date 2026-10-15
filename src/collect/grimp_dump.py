import json
import sys

import grimp

# Run with `python -I`: the scanned source root is not on sys.path while the standard library
# and grimp load, and is appended last, so a file in the repository cannot shadow them.
src, package = sys.argv[1], sys.argv[2]
sys.path.append(src)
# cache_dir=None: grimp writes nothing into the scanned repository.
graph = grimp.build_graph(package, cache_dir=None)
imports = []
for importer in sorted(graph.modules):
    for imported in sorted(graph.find_modules_directly_imported_by(importer)):
        imports.append({"importer": importer, "imported": imported})
json.dump(
    {"grimp_version": grimp.__version__, "modules": sorted(graph.modules), "imports": imports},
    sys.stdout,
    indent=1,
)
sys.stdout.write("\n")
