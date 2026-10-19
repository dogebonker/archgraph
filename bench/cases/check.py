"""Isolated synthetic feature oracle; deliberately not an upstream test suite."""
import ast
import functools
import inspect
import os
from pathlib import Path
import subprocess
import sys
import typing


def function(path, name):
    tree = ast.parse(Path(path).read_text())
    selected = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == name]
    assert len(selected) == 1, f'expected exactly one {name}'
    module = ast.Module(body=[ast.ImportFrom(module='__future__', names=[ast.alias(name='annotations')], level=0), *selected], type_ignores=[])
    namespace = dict(os=os, t=typing, typing=typing, functools=functools)
    exec(compile(ast.fix_missing_locations(module), path, 'exec'), namespace)
    return namespace[name]


def main(case):
    if case == 'flask':
        load = function('src/flask/helpers.py', 'get_load_dotenv')
        for value in ('  FALSE  ', '\tNo\n', ' 0 '):
            os.environ['FLASK_SKIP_DOTENV'] = value
            assert load() is True, 'whitespace-padded false values should enable dotenv'
        for value in ('  ', '\t'):
            os.environ['FLASK_SKIP_DOTENV'] = value
            assert load(False) is False and load(True) is True, 'blank values should use the default'
        os.environ['FLASK_SKIP_DOTENV'] = 'yes'
        assert load() is False
        os.environ.pop('FLASK_SKIP_DOTENV')
        assert load(False) is False and load() is True
    elif case == 'httpx':
        convert = function('httpx/_utils.py', 'to_bytes')
        assert 'bytearray' in str(inspect.signature(convert).parameters['value'].annotation), 'bytearray input annotation is missing'
        value = bytearray(b'abc')
        result = convert(value)
        assert type(result) is bytes and result == b'abc', 'bytearray should become immutable bytes'
        value[0] = 120
        assert result == b'abc', 'conversion must not retain a mutable alias'
        assert convert(bytearray()) == b''
        assert convert('é') == b'\xc3\xa9' and convert('é', 'latin1') == b'\xe9'
        original = b'abc'
        assert convert(original) is original
    elif case == 'pydantic':
        unique = function('pydantic/_internal/_utils.py', 'unique_list')
        parameter = inspect.signature(unique).parameters.get('keep_first')
        assert parameter is not None, 'keep_first keyword is missing'
        assert parameter.kind == inspect.Parameter.KEYWORD_ONLY and parameter.default is False, 'keep_first must be keyword-only and default False'
        values = [('a', 1), ('b', 2), ('a', 3), ('c', 4), ('b', 5)]
        name = lambda item: item[0]
        assert unique(values, name_factory=name, keep_first=True) == [('a', 1), ('b', 2), ('c', 4)]
        assert unique(values, name_factory=name) == [('a', 3), ('b', 5), ('c', 4)]
        assert unique(tuple(values), name_factory=name, keep_first=False) == [('a', 3), ('b', 5), ('c', 4)]
        assert unique([], keep_first=True) == [] and len(values) == 5
    elif case == 'trpc':
        subprocess.run([sys.argv[2], '--experimental-strip-types', '--input-type=module', '-e', '''
import assert from 'node:assert/strict';
import { isAsyncIterable } from './packages/server/src/unstable-core-do-not-import/utils.ts';
assert.equal(isAsyncIterable({[Symbol.asyncIterator]: 42}), false, 'iterator must be callable');
assert.equal(isAsyncIterable({[Symbol.asyncIterator]: undefined}), false);
assert.equal(isAsyncIterable(null), false);
assert.equal(isAsyncIterable({}), false);
assert.equal(isAsyncIterable({[Symbol.asyncIterator]() { return this; }}), true);
assert.equal(isAsyncIterable((async function* () { yield 1; })()), true);
'''], check=True)
    else:
        raise ValueError(case)
    print(f'{case}: isolated feature oracle passed (not the upstream suite)')


if __name__ == '__main__':
    main(sys.argv[1])
