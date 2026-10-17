"""Read Python public names without importing or executing scanned modules."""
import ast
import json
import pathlib
import sys


def collect(data):
    root = pathlib.Path(data['root'])
    files = set(data['files'])
    modules = {}
    for file in sorted(files):
        name = file[:-3].replace('/', '.')
        if name.endswith('.__init__'):
            name = name[:-9]
        modules[name] = file
        # src-layout imports are rooted at the source directory.
        if name.startswith('src.'):
            modules.setdefault(name[4:], file)
    trees = {}
    limitations = set()
    complete = True
    for file in sorted(files):
        try:
            trees[file] = ast.parse((root / file).read_bytes(), filename=file)
        except (SyntaxError, OSError) as error:
            limitations.add(f'Cannot parse {file}: {error}')
            complete = False
    exports = {}
    explicit = {}
    imports = []
    pending = []
    namespaces = {}

    def module_name(file, node):
        if not node.level:
            return node.module or ''
        package = file[:-3].replace('/', '.').split('.')
        package = package[:-1]
        if node.level > len(package):
            return ''
        return '.'.join(package[:len(package) - node.level + 1] + ([node.module] if node.module else []))

    for file, tree in trees.items():
        names = {}
        constants = {}
        all_names = None
        all_seen = False

        def strings(value):
            if isinstance(value, (ast.List, ast.Tuple, ast.Set)):
                values = [item.value for item in value.elts if isinstance(item, ast.Constant) and isinstance(item.value, str)]
                return values if len(values) == len(value.elts) else None
            if isinstance(value, ast.Name):
                return constants.get(value.id)
            if isinstance(value, ast.BinOp) and isinstance(value.op, ast.Add):
                left, right = strings(value.left), strings(value.right)
                return left + right if left is not None and right is not None else None
            return None

        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names[node.name] = {'file': file, 'name': node.name}
                if node.name == '__getattr__':
                    limitations.add(f'Dynamic module attributes in {file}; exports are incomplete')
                    complete = False
            elif isinstance(node, (ast.Assign, ast.AnnAssign)):
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                for target in targets:
                    if isinstance(target, ast.Name):
                        names[target.id] = {'file': file, 'name': target.id}
                        constants[target.id] = strings(node.value)
                        if target.id == '__all__':
                            all_seen = True
                            all_names = constants[target.id]
                    else:
                        limitations.add(f'Compound assignment target in {file}; exports are incomplete')
                        complete = False
            elif isinstance(node, ast.AugAssign) and isinstance(node.target, ast.Name) and node.target.id == '__all__':
                right = strings(node.value)
                all_seen = True
                all_names = all_names + right if isinstance(node.op, ast.Add) and all_names is not None and right is not None else None
            elif isinstance(node, ast.AugAssign):
                limitations.add(f'Mutated module binding in {file}; exports are incomplete')
                complete = False
            if isinstance(node, ast.ImportFrom):
                module = module_name(file, node)
                to = modules.get(module)
                for alias in node.names:
                    child = modules.get(module + '.' + alias.name)
                    if child and (not to or alias.name not in names):
                        # A package submodule is a namespace, not a declared value.
                        local = alias.asname or alias.name
                        names[local] = {'file': child, 'name': '*'}
                        namespaces[(file, to or child, alias.name, local)] = names[local]
                        pending.append((file, to or child, alias.name, local, False))
                        limitations.add(f'Namespace member access in {file} is conservative')
                        complete = False
                    elif to:
                        pending.append((file, to, alias.name, alias.asname or alias.name, alias.name == '*'))
                    else:
                        limitations.add(f'Unresolved or external re-export in {file}: {module}.{alias.name}')
                        complete = False
            elif isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name in modules:
                        local = alias.asname or alias.name.split('.')[0]
                        namespace = modules[alias.name] if alias.asname else modules.get(alias.name.split('.')[0], modules[alias.name])
                        names[local] = {'file': namespace, 'name': '*'}
                        pending.append((file, modules[alias.name], '*', local, True))
                        limitations.add(f'Namespace member access in {file} is conservative')
                        complete = False
                    else:
                        limitations.add(f'External namespace export in {file}: {alias.name}')
                        complete = False
            elif not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Assign, ast.AnnAssign, ast.AugAssign, ast.Pass, ast.Expr, ast.Assert, ast.Raise, ast.Global, ast.Nonlocal)):
                limitations.add(f'Conditional module declarations in {file}; exports are incomplete')
                complete = False
        # Nested imports count as consumers but do not create module exports.
        top_ids = {id(node) for node in tree.body}
        for node in ast.walk(tree):
            if isinstance(node, ast.NamedExpr):
                limitations.add(f'Assignment expression in {file}; exports are incomplete')
                complete = False
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name) and (node.func.value.id == '__all__' or constants.get(node.func.value.id) is not None):
                limitations.add(f'Mutated __all__ or constant alias in {file}; exports are incomplete')
                complete = False
            if id(node) in top_ids:
                continue
            if isinstance(node, ast.ImportFrom):
                to = modules.get(module_name(file, node))
                if to:
                    for alias in node.names:
                        pending.append((file, to, alias.name, None, alias.name == '*'))
            elif isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name in modules:
                        pending.append((file, modules[alias.name], '*', None, True))
        if all_seen and all_names is None:
            limitations.add(f'Dynamic __all__ in {file}; exports are incomplete')
            complete = False
        explicit[file] = set(all_names) if all_names is not None else None
        exports[file] = names
    # Re-exports can form chains and cycles. Iterate to a bounded fixed point.
    for _ in range(len(files) + 1):
        changed = False
        for file, to, name, local, wildcard in pending:
            if local is None:
                continue
            source = exports.get(to, {})
            public = explicit.get(to)
            if wildcard and name == '*' and local == '*':
                updates = {key: value for key, value in source.items() if key in public} if public is not None else {key: value for key, value in source.items() if not key.startswith('_')}
            elif wildcard:
                updates = {}
            else:
                namespace = namespaces.get((file, to, name, local))
                updates = {local: source[name]} if name in source else {local: namespace} if namespace else {}
            for key, value in updates.items():
                if exports[file].get(key) != value:
                    exports[file][key] = value
                    changed = True
        if not changed:
            break
    else:
        limitations.add('Cyclic re-exports did not converge; exports are incomplete')
        complete = False
    result_exports = []
    for file, names in exports.items():
        public = explicit[file]
        for name in public if public is not None else names:
            if public is None and name.startswith('_'):
                continue
            if name in names:
                result_exports.append({'file': file, 'name': name, 'target': names[name]})
            else:
                limitations.add(f'Unresolved export {file}:{name}')
                complete = False
    for file, to, name, local, wildcard in pending:
        target = None if wildcard else exports.get(to, {}).get(name, namespaces.get((file, to, name, local)))
        imports.append({'from': file, 'to': to, 'name': name, 'target': target, 'wildcard': wildcard})
        if not wildcard and target is None:
            limitations.add(f'Unresolved import {file}:{name}')
            complete = False
    return {'exports': sorted(result_exports, key=lambda item: (item['file'], item['name'])), 'imports': imports, 'limitations': sorted(limitations), 'complete': complete}


if __name__ == '__main__':
    json.dump(collect(json.load(sys.stdin)), sys.stdout)
