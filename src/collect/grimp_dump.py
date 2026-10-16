import ast
import json
import os
import sys

import grimp

# Run with `python -I`: the scanned source root is not on sys.path while the standard library
# and grimp load, and is appended last, so a file in the repository cannot shadow them.
src, package = sys.argv[1], sys.argv[2]
sys.path.append(src)
# cache_dir=None: grimp writes nothing into the scanned repository.
graph = grimp.build_graph(package, cache_dir=None)
# The same imports without those under `if TYPE_CHECKING:`.
runtime = grimp.build_graph(package, exclude_type_checking_imports=True, cache_dir=None)

FUNCTIONS = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)
deferred_lines = {}


def deferred(module):
    """Lines of the module's import statements inside a function body. The file is parsed with
    ast, never run."""
    if module not in deferred_lines:
        base = os.path.join(src, *module.split("."))
        path = os.path.join(base, "__init__.py") if os.path.isdir(base) else base + ".py"
        lines = set()
        try:
            with open(path, "rb") as f:
                stack = [(ast.parse(f.read()), False)]
        except (OSError, SyntaxError, ValueError):
            stack = []
        while stack:
            node, inside = stack.pop()
            if inside and isinstance(node, (ast.Import, ast.ImportFrom)):
                lines.update(range(node.lineno, (node.end_lineno or node.lineno) + 1))
            inside = inside or isinstance(node, FUNCTIONS)
            stack.extend((child, inside) for child in ast.iter_child_nodes(node))
        deferred_lines[module] = lines
    return deferred_lines[module]


def kind(importer, imported):
    """`type` when every occurrence is under `if TYPE_CHECKING:`, `dynamic` when every other
    occurrence is inside a function body, else `static`."""
    if not runtime.direct_import_exists(importer=importer, imported=imported):
        return "type"
    details = runtime.get_import_details(importer=importer, imported=imported)
    lines = [d["line_number"] for d in details]
    if lines and all(line in deferred(importer) for line in lines):
        return "dynamic"
    return "static"


imports = []
for importer in sorted(graph.modules):
    for imported in sorted(graph.find_modules_directly_imported_by(importer)):
        imports.append(
            {"importer": importer, "imported": imported, "kind": kind(importer, imported)}
        )
json.dump(
    {"grimp_version": grimp.__version__, "modules": sorted(graph.modules), "imports": imports},
    sys.stdout,
    indent=1,
)
sys.stdout.write("\n")
