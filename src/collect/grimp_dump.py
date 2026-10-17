import ast
from collections import deque
from functools import lru_cache
import json
import importlib.machinery
import os
import sys

import grimp

# Run with `python -I`: the scanned source root is not on sys.path while the standard library
# and grimp load, and is appended last, so a file in the repository cannot shadow them.
src, package = sys.argv[1], sys.argv[2]
scan_root = sys.argv[3] if len(sys.argv) > 3 else src
include_tests = "--include-tests" in sys.argv[4:]
python_paths = [os.path.realpath(sys.argv[i + 1]) for i in range(4, len(sys.argv) - 1) if sys.argv[i] == "--python-path"]
scan_root = os.path.realpath(scan_root)
local_roots = list(dict.fromkeys([os.path.realpath(src), *python_paths, scan_root]))
external_roots = [path for path in sys.path if path and not os.path.realpath(path).startswith(scan_root + os.sep) and os.path.realpath(path) != scan_root]
external_roots += [path for path in python_paths if not path.startswith(scan_root + os.sep) and path != scan_root]
for path in local_roots:
    if path not in sys.path:
        sys.path.append(path)


def inside_root(path):
    return os.path.commonpath([scan_root, os.path.realpath(path)]) == scan_root


@lru_cache(maxsize=None)
def directory_entries(directory):
    try:
        return frozenset(os.listdir(directory))
    except OSError:
        return frozenset()


def exact_path(directory, relative):
    # Import names are case-sensitive even on a case-insensitive filesystem.
    for part in relative.split(os.sep):
        if part not in directory_entries(directory):
            return None
        directory = os.path.join(directory, part)
    return directory


@lru_cache(maxsize=None)
def source_path(module):
    for directory in local_roots:
        base = os.path.join(*module.split("."))
        for relative in [os.path.join(base, "__init__.py"), base + ".py"]:
            path = exact_path(directory, relative)
            if path and os.path.isfile(path) and inside_root(path):
                return path
    return None


@lru_cache(maxsize=None)
def namespace_path(module):
    for directory in local_roots:
        path = exact_path(directory, os.path.join(*module.split(".")))
        if module and path and os.path.isdir(path) and inside_root(path):
            return os.path.relpath(path, scan_root).replace(os.sep, "/")
    return None


def namespace(module):
    return namespace_path(module) is not None


@lru_cache(maxsize=None)
def external_package(module):
    """Find specs without import_module/find_spec, which would execute package initializers."""
    if not module or source_path(module.split(".")[0]) or namespace(module.split(".")[0]):
        return None
    # os.path is a standard-library alias of posixpath/ntpath, not a package directory.
    if module == "os.path":
        return "os" if external_package(os.path.__name__) else None
    search = external_roots
    for index, part in enumerate(module.split(".")):
        full = ".".join(module.split(".")[: index + 1])
        try:
            spec = (importlib.machinery.BuiltinImporter.find_spec(full) or importlib.machinery.FrozenImporter.find_spec(full)) if index == 0 else None
            spec = spec or importlib.machinery.PathFinder.find_spec(full, search)
        except (ImportError, AttributeError, ValueError):
            return None
        if spec is None:
            return None
        search = spec.submodule_search_locations
        if index < len(module.split(".")) - 1 and search is None:
            return None
    return module.split(".")[0]


@lru_cache(maxsize=None)
def native_module(module):
    for directory in local_roots:
        for suffix in importlib.machinery.EXTENSION_SUFFIXES:
            path = exact_path(directory, os.path.join(*module.split(".")) + suffix)
            if path and os.path.isfile(path):
                return True
    return False


if source_path(package) is None and namespace(package):
    class NamespaceGraph:
        modules = set()
        def find_modules_directly_imported_by(self, module):
            return []
    graph = NamespaceGraph()
    for directory in local_roots:
        base = exact_path(directory, os.path.join(*package.split(".")))
        if not base or not os.path.isdir(base) or not inside_root(base):
            continue
        for parent, dirs, files in os.walk(base, followlinks=False):
            dirs[:] = sorted(name for name in dirs if not name.startswith(".") and name != "__pycache__")
            for name in sorted(files):
                if name.endswith(".py"):
                    module = os.path.relpath(os.path.join(parent, name), directory)[:-3].replace(os.sep, ".")
                    graph.modules.add(module[:-9] if module.endswith(".__init__") else module)
    runtime = graph
else:
    # cache_dir=None: grimp writes nothing into the scanned repository.
    graph = grimp.build_graph(package, cache_dir=None)
    runtime = grimp.build_graph(package, exclude_type_checking_imports=True, cache_dir=None)

FUNCTIONS = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)
deferred_lines = {}


def deferred(module):
    """Lines of the module's import statements inside a function body. The file is parsed with
    ast, never run."""
    if module not in deferred_lines:
        path = source_path(module)
        lines = set()
        try:
            with open(path, "rb") as f:
                stack = [(ast.parse(f.read()), False)]
        except (OSError, SyntaxError, ValueError, TypeError):
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
    return source_path(module)


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


def dynamic_import_calls(tree):
    """Recognize calls only through stable lexical import bindings, never by spelling alone."""
    binding_cache = {}
    unsafe_globals = {name for node in ast.walk(tree) if isinstance(node, (ast.Global, ast.Nonlocal)) for name in node.names}
    mutated_modules = {node.value.id for node in ast.walk(tree) if isinstance(node, ast.Attribute) and node.attr == "import_module" and isinstance(node.value, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del))}
    if any(isinstance(node, ast.Attribute) and node.attr == "__import__" and isinstance(node.ctx, (ast.Store, ast.Del)) for node in ast.walk(tree)):
        unsafe_globals.add("__import__")

    def bindings(scope):
        if id(scope) in binding_cache:
            return binding_cache[id(scope)]
        result = {}
        body = scope.body if isinstance(scope, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) else [scope.body]
        direct = {id(node) for node in body}

        def bind(name, kind=None, line=0):
            result.setdefault(name, []).append((kind, line))

        class Bound(ast.NodeVisitor):
            def visit_Name(self, node):
                if isinstance(node.ctx, (ast.Store, ast.Del)):
                    bind(node.id)
            def visit_Import(self, node):
                for alias in node.names:
                    bind(alias.asname or alias.name.split(".")[0], "module" if id(node) in direct and alias.name == "importlib" else None, (node.lineno, node.col_offset))
            def visit_ImportFrom(self, node):
                for alias in node.names:
                    if alias.name == "*":
                        # An unknown wildcard can overwrite any global binding.
                        bind("*")
                    else:
                        trusted = id(node) in direct and not node.level and (node.module, alias.name) in {("importlib", "import_module"), ("builtins", "__import__")}
                        bind(alias.asname or alias.name, "function" if trusted else None, (node.lineno, node.col_offset))
            def visit_FunctionDef(self, node):
                bind(node.name)
                for expression in [*node.decorator_list, *node.args.defaults, *[default for default in node.args.kw_defaults if default]]:
                    self.visit(expression)
            visit_AsyncFunctionDef = visit_FunctionDef
            def visit_ClassDef(self, node):
                bind(node.name)
                for expression in [*node.decorator_list, *node.bases, *[keyword.value for keyword in node.keywords]]:
                    self.visit(expression)
            def visit_Lambda(self, node):
                for expression in [*node.args.defaults, *[default for default in node.args.kw_defaults if default]]:
                    self.visit(expression)
            def visit_ExceptHandler(self, node):
                if node.name:
                    bind(node.name)
                self.generic_visit(node)
            def visit_Global(self, node):
                for name in node.names:
                    bind(name)
            visit_Nonlocal = visit_Global
        visitor = Bound()
        for node in body:
            visitor.visit(node)
        if isinstance(scope, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            for argument in [*scope.args.posonlyargs, *scope.args.args, *scope.args.kwonlyargs, scope.args.vararg, scope.args.kwarg]:
                if argument:
                    bind(argument.arg)
        binding_cache[id(scope)] = result
        return result

    proven = set()
    scopes = [tree]

    def resolve(name, line):
        if name in unsafe_globals:
            return None
        inside_function = False
        for scope in reversed(scopes):
            if isinstance(scope, ast.ClassDef) and inside_function:
                continue  # A method's unqualified names do not use its class namespace.
            inside_function |= isinstance(scope, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda))
            local = bindings(scope)
            if "*" in local:
                return None
            if name in local:
                events = local[name]
                kinds = {kind for kind, _ in events}
                return events[0][0] if len(kinds) == 1 and None not in kinds and min(at for _, at in events) < line else None
        return "function" if name == "__import__" else None

    class Calls(ast.NodeVisitor):
        def visit_Call(self, node):
            if isinstance(node.func, ast.Name):
                if resolve(node.func.id, (node.lineno, node.col_offset)) == "function":
                    proven.add(id(node))
            elif isinstance(node.func, ast.Attribute) and node.func.attr == "import_module" and isinstance(node.func.value, ast.Name):
                if node.func.value.id not in mutated_modules and resolve(node.func.value.id, (node.lineno, node.col_offset)) == "module":
                    proven.add(id(node))
            self.generic_visit(node)
        def visit_FunctionDef(self, node):
            for expression in [*node.decorator_list, *node.args.defaults, *[default for default in node.args.kw_defaults if default]]:
                self.visit(expression)
            scopes.append(node)
            for statement in node.body:
                self.visit(statement)
            scopes.pop()
        visit_AsyncFunctionDef = visit_FunctionDef
        def visit_Lambda(self, node):
            scopes.append(node)
            self.visit(node.body)
            scopes.pop()
        def visit_ClassDef(self, node):
            for expression in [*node.decorator_list, *node.bases]:
                self.visit(expression)
            scopes.append(node)
            for statement in node.body:
                self.visit(statement)
            scopes.pop()
    Calls().visit(tree)
    return proven


parsed = {}
static_all = {}
attributes = {}
pending = deque(sorted(set(graph.modules) | extra_paths.keys()))
queued = set(pending)
while pending:
    module = pending.popleft()
    if module in parsed:
        continue
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
    dynamic_calls = dynamic_import_calls(tree)
    for node in ast.walk(tree):
        candidates = []
        if isinstance(node, ast.Import):
            candidates = [alias.name for alias in node.names]
        elif id(node) in dynamic_calls and node.args and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str):
            candidates = [node.args[0].value]
        elif isinstance(node, ast.ImportFrom):
            parent = module.split(".") if os.path.basename(path) == "__init__.py" else module.split(".")[:-1]
            base = node.module or ""
            if node.level and node.level <= len(parent):
                base = ".".join(parent[:len(parent) - node.level + 1] + ([base] if base else []))
            elif node.level:
                base = ""
            if base:
                candidates = [base] + [base + "." + alias.name for alias in node.names if alias.name != "*"]
        for candidate in candidates:
            candidate_path = module_path(candidate)
            if candidate_path and candidate not in queued:
                extra_paths.setdefault(candidate, candidate_path)
                pending.append(candidate)
                queued.add(candidate)
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
import_records = []
for module, (path, tree, declared) in parsed.items():
    routes, exports = [], []
    dynamic_calls = dynamic_import_calls(tree)
    guards = {"TYPE_CHECKING"}
    for node in tree.body:
        if isinstance(node, ast.ImportFrom) and node.module == "typing":
            guards.update(alias.asname or alias.name for alias in node.names if alias.name == "TYPE_CHECKING")
    stack = [(node, False, False, True) for node in reversed(tree.body)]
    while stack:
        node, inside, guarded, top_level = stack.pop()
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            base = relative_name(module, path, node) if isinstance(node, ast.ImportFrom) else None
            records = {}
            for alias in node.names:
                raw = ("." * node.level + (node.module or "")) if isinstance(node, ast.ImportFrom) else alias.name
                record = records.setdefault(raw, {"from": os.path.relpath(path, scan_root).replace(os.sep, "/"), "specifier": raw, "line": node.lineno, "kind": "type" if guarded else "lazy" if inside else "runtime", "targets": []})
                destination, names, wildcard = None, [], False
                if isinstance(node, ast.ImportFrom):
                    if not base:
                        record["reason"] = "missing"
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
                    external = external_package(base if isinstance(node, ast.ImportFrom) else alias.name)
                    if external:
                        record["external"] = external
                    elif namespace_path(child if isinstance(node, ast.ImportFrom) and alias.name != "*" else base if isinstance(node, ast.ImportFrom) else alias.name):
                        record["namespace"] = namespace_path(child if isinstance(node, ast.ImportFrom) and alias.name != "*" else base if isinstance(node, ast.ImportFrom) else alias.name)
                    else:
                        record["reason"] = "native" if native_module(base if isinstance(node, ast.ImportFrom) else alias.name) else "missing"
                    continue
                target = os.path.relpath(target, scan_root).replace(os.sep, "/")
                record["targets"].append(target)
                import_kind = "type" if guarded else "lazy" if inside else "runtime"
                routes.append({"to": target, "names": names, "wildcard": wildcard,
                    "kind": import_kind, "specifier": ("." * node.level + (node.module or "")) if isinstance(node, ast.ImportFrom) else alias.name,
                    "line": node.lineno, "side_effect": False})
                if top_level:
                    exported = alias.asname or alias.name.split(".")[0]
                    exports.append({"exported": exported, "imported": imported, "to": target})
            for record in records.values():
                record["targets"] = sorted(set(record["targets"]))
                if "reason" in record:
                    record.pop("external", None)
                    record.pop("namespace", None)
                import_records.append(record)
            continue
        if id(node) in dynamic_calls:
            argument = node.args[0] if node.args else None
            literal = argument.value if isinstance(argument, ast.Constant) and isinstance(argument.value, str) else None
            record = {"from": os.path.relpath(path, scan_root).replace(os.sep, "/"), "specifier": literal if literal is not None else ast.unparse(argument) if argument else "<dynamic>", "line": node.lineno, "kind": "type" if guarded else "lazy" if inside else "runtime", "targets": []}
            target = module_path(literal) if literal and not literal.startswith(".") else None
            if target:
                target = os.path.relpath(target, scan_root).replace(os.sep, "/")
                record["targets"] = [target]
                routes.append({"to": target, "names": [], "wildcard": True, "kind": record["kind"], "specifier": literal, "line": node.lineno, "side_effect": False})
            elif literal and namespace_path(literal):
                record["namespace"] = namespace_path(literal)
            elif literal and external_package(literal):
                record["external"] = external_package(literal)
            else:
                record["reason"] = "dynamic" if literal is None else "missing"
            import_records.append(record)
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
    {"grimp_version": grimp.__version__, "modules": sorted(graph.modules), "imports": imports, "barrel_modules": barrel_modules, "files": sorted(set(os.path.relpath(path, scan_root).replace(os.sep, "/") for path, _, _ in parsed.values())), "module_paths": {module: os.path.relpath(path, scan_root).replace(os.sep, "/") for module, (path, _, _) in parsed.items()}, "import_records": import_records},
    sys.stdout,
    indent=1,
)
sys.stdout.write("\n")
