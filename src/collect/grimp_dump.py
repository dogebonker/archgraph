import ast
import json
import os
import sys

import grimp

# Run with `python -I`: the scanned source root is not on sys.path while the standard library
# and grimp load, and is appended last, so a file in the repository cannot shadow them.
src, package = sys.argv[1], sys.argv[2]
scan_root = sys.argv[3] if len(sys.argv) > 3 else src
include_tests = "--include-tests" in sys.argv[4:]
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


extra_paths = {}
if include_tests:
    known_paths = {os.path.realpath(os.path.join(src, *module.split(".")) + ".py") for module in graph.modules}
    for directory, dirs, files in os.walk(scan_root, followlinks=False):
        dirs[:] = sorted(name for name in dirs if not name.startswith(".") and name not in {"node_modules", "vendor", "venv", "target", "__pycache__"})
        relative = os.path.relpath(directory, scan_root)
        test_directory = any(part in {"tests", "test", "__tests__"} for part in relative.split(os.sep))
        for name in sorted(files):
            if not name.endswith(".py") or not (test_directory or name == "conftest.py" or name.startswith("test_") or name.endswith("_test.py")):
                continue
            path = os.path.join(directory, name)
            if os.path.islink(path) or os.path.realpath(path) in known_paths:
                continue
            relative_source = os.path.relpath(path, src)
            relative_module = relative_source if not relative_source.startswith(".." + os.sep) else os.path.relpath(path, scan_root)
            module = relative_module[:-3].replace(os.sep, ".")
            if module.endswith(".__init__"):
                module = module[:-9]
            if module not in graph.modules:
                extra_paths[module] = path


def module_path(module):
    if module in extra_paths:
        return extra_paths[module]
    base = os.path.join(src, *module.split("."))
    init = os.path.join(base, "__init__.py")
    path = init if os.path.isfile(init) else base + ".py"
    return path if os.path.isfile(path) else None


def exported_names(value):
    """Static __all__ only: arbitrary expressions must not imply complete exports."""
    if isinstance(value, (ast.List, ast.Tuple, ast.Set)) and all(
        isinstance(item, ast.Constant) and isinstance(item.value, str)
        for item in value.elts
    ):
        return [item.value for item in value.elts]
    if isinstance(value, ast.BinOp) and isinstance(value.op, ast.Add):
        left, right = exported_names(value.left), exported_names(value.right)
        if left is not None and right is not None:
            return left + right
    return None


def all_assignment(node):
    if isinstance(node, ast.Assign) and all(
        isinstance(target, ast.Name) and target.id == "__all__" for target in node.targets
    ):
        return exported_names(node.value)
    if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and node.target.id == "__all__":
        return exported_names(node.value)
    return None


def pure_statement(node):
    return isinstance(node, (ast.Import, ast.ImportFrom)) or (
        isinstance(node, ast.Expr) and isinstance(node.value, ast.Constant)
        and isinstance(node.value.value, str)
    ) or all_assignment(node) is not None


def type_guard(node, aliases):
    return (isinstance(node, ast.Name) and node.id in aliases) or (
        isinstance(node, ast.Attribute) and node.attr == "TYPE_CHECKING"
    )


parsed = {}
static_all = {}
attributes = {}
for module in sorted(set(graph.modules) | extra_paths.keys()):
    path = module_path(module)
    if path is None:
        continue
    try:
        with open(path, "rb") as source:
            tree = ast.parse(source.read())
    except (OSError, SyntaxError, ValueError):
        continue
    declared = []
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            declared.append(node.name)
        elif isinstance(node, ast.Assign):
            declared.extend(target.id for target in node.targets if isinstance(target, ast.Name) and target.id != "__all__")
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and node.target.id != "__all__":
            declared.append(node.target.id)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            declared.extend(alias.asname or alias.name.split(".")[0] for alias in node.names if alias.name != "*")
    explicit_all = [all_assignment(node) for node in tree.body if all_assignment(node) is not None]
    attributes[module] = declared
    if explicit_all:
        static_all[module] = explicit_all[-1]
    parsed[module] = (path, tree, explicit_all[-1] if explicit_all else declared)


def relative_name(importer, path, node):
    if not node.level:
        return node.module or ""
    parent = importer.split(".") if os.path.basename(path) == "__init__.py" else importer.split(".")[:-1]
    if node.level > len(parent):
        return ""
    parent = parent[: len(parent) - node.level + 1]
    return ".".join(parent + ([node.module] if node.module else []))


# Star-bound package attributes win over same-named submodules. Propagate only statically
# visible names, bounded like the re-export resolver; source modules are never imported.
for _ in range(10):
    changed = False
    for module, (path, tree, _) in parsed.items():
        for node in tree.body:
            if not isinstance(node, ast.ImportFrom) or not any(alias.name == "*" for alias in node.names):
                continue
            base = relative_name(module, path, node)
            names = static_all.get(base, [name for name in attributes.get(base, []) if not name.startswith("_")])
            for name in names:
                if name not in attributes[module]:
                    attributes[module].append(name)
                    changed = True
        if module not in static_all:
            parsed[module] = (path, tree, attributes[module])
    if not changed:
        break


barrel_modules = []
for module, (path, tree, declared) in parsed.items():
    routes, exports = [], []
    guards = {"TYPE_CHECKING"}
    for node in tree.body:
        if isinstance(node, ast.ImportFrom) and node.module == "typing":
            guards.update(alias.asname or alias.name for alias in node.names if alias.name == "TYPE_CHECKING")
    stack = [(node, False, False, True) for node in reversed(tree.body)]
    while stack:
        node, inside, guarded, top_level = stack.pop()
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            base = relative_name(module, path, node) if isinstance(node, ast.ImportFrom) else None
            for alias in node.names:
                destination, names, wildcard = None, [], False
                if isinstance(node, ast.ImportFrom):
                    if not base:
                        continue
                    child = base + "." + alias.name
                    base_path = module_path(base)
                    base_declared = attributes.get(base, [])
                    if base == module and top_level:
                        # This import cannot bind its own package attribute before it runs.
                        base_declared = []
                        for previous in tree.body:
                            if previous.lineno >= node.lineno:
                                break
                            if isinstance(previous, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                                base_declared.append(previous.name)
                            elif isinstance(previous, ast.Assign):
                                base_declared.extend(target.id for target in previous.targets if isinstance(target, ast.Name))
                            elif isinstance(previous, ast.AnnAssign) and isinstance(previous.target, ast.Name):
                                base_declared.append(previous.target.id)
                            elif isinstance(previous, (ast.Import, ast.ImportFrom)):
                                for binding in previous.names:
                                    if binding.name == "*" and isinstance(previous, ast.ImportFrom):
                                        previous_base = relative_name(module, path, previous)
                                        base_declared.extend(static_all.get(previous_base, [name for name in attributes.get(previous_base, []) if not name.startswith("_")]))
                                    else:
                                        base_declared.append(binding.asname or binding.name.split(".")[0])
                    if alias.name != "*" and module_path(child) and alias.name not in base_declared:
                        destination, imported = child, "*"
                        wildcard = True
                    elif base_path:
                        destination, imported = base, alias.name
                        wildcard = alias.name == "*"
                        names = [] if wildcard else [alias.name]
                        if wildcard and base in static_all:
                            names, wildcard = static_all[base], False
                    elif alias.name != "*" and module_path(child):
                        destination, imported = child, "*"
                        wildcard = True
                else:
                    destination, imported, wildcard = alias.name, "*", True
                target = module_path(destination) if destination else None
                if target is None:
                    continue
                target = os.path.relpath(target, scan_root).replace(os.sep, "/")
                import_kind = "type" if guarded else "lazy" if inside else "runtime"
                routes.append({"to": target, "names": names, "wildcard": wildcard,
                    "kind": import_kind, "specifier": ("." * node.level + (node.module or "")) if isinstance(node, ast.ImportFrom) else alias.name,
                    "line": node.lineno, "side_effect": False})
                if top_level:
                    exported = alias.asname or alias.name.split(".")[0]
                    exports.append({"exported": exported, "imported": imported, "to": target})
            continue
        if isinstance(node, ast.If) and type_guard(node.test, guards):
            stack.extend((child, inside, True, False) for child in reversed(node.body))
            stack.extend((child, inside, guarded, False) for child in reversed(node.orelse))
        elif isinstance(node, ast.If) and isinstance(node.test, ast.UnaryOp) and isinstance(node.test.op, ast.Not) and type_guard(node.test.operand, guards):
            stack.extend((child, inside, guarded, False) for child in reversed(node.body))
            stack.extend((child, inside, True, False) for child in reversed(node.orelse))
        else:
            inside = inside or isinstance(node, FUNCTIONS)
            stack.extend((child, inside, guarded, False) for child in reversed(list(ast.iter_child_nodes(node))))
    barrel_modules.append({"file": os.path.relpath(path, scan_root).replace(os.sep, "/"),
        "pure": os.path.basename(path) == "__init__.py" and bool(tree.body) and all(pure_statement(node) for node in tree.body),
        "declared": sorted(set(declared)), "exports": exports, "imports": routes})


imports = []
for importer in sorted(graph.modules):
    for imported in sorted(graph.find_modules_directly_imported_by(importer)):
        imports.append(
            {"importer": importer, "imported": imported, "kind": kind(importer, imported)}
        )
json.dump(
    {"grimp_version": grimp.__version__, "modules": sorted(graph.modules), "imports": imports, "barrel_modules": barrel_modules, "files": sorted(os.path.relpath(path, scan_root).replace(os.sep, "/") for path in extra_paths.values())},
    sys.stdout,
    indent=1,
)
sys.stdout.write("\n")
