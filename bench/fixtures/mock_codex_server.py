#!/usr/bin/env python3
"""Offline app-server protocol fixture. Does not launch Codex or contact a model."""
import json
import os
from pathlib import Path
import sys

if '--version' in sys.argv:
    print('codex-cli 0.163.0')
    raise SystemExit(0)
if 'app-server' in sys.argv:
    assert '--disable' in sys.argv and sys.argv[sys.argv.index('--disable') + 1] == 'apps'
cwd = Path.cwd().resolve()
binary = Path(os.environ.get('MOCK_ARCHGRAPH_BINARY', str(Path(sys.argv[0]).with_name('mock-archgraph')))).resolve()
trusted = False
thread, turn = 'fixture-thread', 'fixture-turn'

def send(value):
    print(json.dumps(value), flush=True)

def quote(value):
    return "'" + str(value).replace("'", "'\\''") + "'"

for line in sys.stdin:
    message = json.loads(line)
    method = message.get('method')
    if 'id' not in message:
        continue
    params = message.get('params', {})
    if method == 'initialize':
        result = {'userAgent': 'MOCK-Codex-0.163'}
    elif method == 'config/batchWrite':
        assert params['filePath'] == str(Path(os.environ['CODEX_HOME']).resolve() / 'config.toml')
        assert params['reloadUserConfig'] is True
        edit = params['edits'][0]
        assert edit['mergeStrategy'] == 'upsert'
        if edit['keyPath'] == 'features.hooks':
            assert edit['value'] is True
        elif edit['keyPath'] == 'projects':
            assert edit['value'] == {str(cwd): {'trust_level': 'trusted'}}
        else:
            assert edit['keyPath'] == 'hooks.state'
            assert len(edit['value']) == 3
            assert all(value == {'trusted_hash': 'a' * 64} for value in edit['value'].values())
            trusted = True
        with open(Path(os.environ['CODEX_HOME']) / 'mock-config-edits.jsonl', 'a') as handle:
            handle.write(json.dumps(params) + '\n')
        result = {'version': 'fixture'}
    elif method == 'hooks/list':
        assert params == {'cwds': [str(cwd)]}
        hooks = []
        if (cwd / '.codex/hooks.json').exists():
            for event, native in [('pre', 'preToolUse'), ('post', 'postToolUse'), ('stop', 'stop')]:
                command = f'{quote(binary)} hook run --agent codex --event {event} --root {quote(cwd)} # archgraph:managed:{event}'
                if os.environ.get('MOCK_TAMPER_COMMAND'):
                    command += ' --unexpected'
                hooks.append({'key': 'fixture-' + event, 'currentHash': 'a' * 64,
                    'sourcePath': str(cwd / '.codex/hooks.json'), 'eventName': native,
                    'matcher': None, 'handlerType': 'command', 'command': command,
                    'enabled': True, 'source': 'project', 'isManaged': False, 'trustStatus': 'trusted' if trusted else 'untrusted'})
        result = {'data': [{'cwd': str(cwd), 'errors': [], 'warnings': [], 'hooks': hooks}]}
    elif method == 'thread/start':
        assert params['sandbox'] == 'workspace-write'
        assert params['approvalPolicy'] == 'never' and params['ephemeral'] is True
        assert params['allowProviderModelFallback'] is False
        result = {'thread': {'id': thread, 'cwd': str(cwd)}, 'model': params['model'],
            'cwd': str(cwd), 'modelProvider': 'openai', 'approvalPolicy': 'never',
            'sandbox': {'type': 'workspaceWrite', 'networkAccess': False, 'writableRoots': []}}
        bad = os.environ.get('MOCK_BAD_RESPONSE')
        if bad == 'provider': result['modelProvider'] = 'unreviewed'
        if bad == 'approval': result['approvalPolicy'] = 'on-request'
        if bad == 'sandbox': result['sandbox'] = {'type': 'dangerFullAccess'}
    elif method == 'mcpServerStatus/list':
        assert params == {'threadId': thread}
        result = {'data': [{'name': 'unreviewed'}] if os.environ.get('MOCK_BAD_RESPONSE') == 'mcp' else []}
    elif method == 'turn/start':
        if (cwd / 'src/domain.txt').exists():
            (cwd / 'src/domain.txt').write_text('safe change\n')
        assert params['threadId'] == thread
        assert params['input'][0]['type'] == 'text'
        if os.environ.get('MOCK_REPLACE_AUTH'):
            auth = Path(os.environ['CODEX_HOME']) / 'auth.json'
            auth.unlink()
            auth.write_text('SDK replacement mock token')
            auth.chmod(0o600)
        if os.environ.get('MOCK_BAD_RESPONSE') == 'turn':
            send({'method': 'turn/completed', 'params': {'threadId': thread, 'turn': {'id': turn, 'status': 'failed'}}})
            send({'id': message['id'], 'result': {'turn': {'id': turn, 'status': 'inProgress'}}})
            continue
        result = {'turn': {'id': turn, 'status': 'inProgress'}}
        if (cwd / '.codex/hooks.json').exists():
            assert trusted
            for index, event in enumerate(('preToolUse', 'postToolUse', 'stop')):
                run = {'id': 'hook-' + str(index), 'eventName': event,
                       'sourcePath': str(cwd / '.codex/hooks.json'), 'handlerType': 'command',
                       'entries': [], 'status': 'running'}
                send({'method': 'hook/started', 'params': {'threadId': thread, 'turnId': turn, 'run': run}})
                run = dict(run, status='completed')
                if event == 'preToolUse':
                    run['entries'] = [{'kind': 'context', 'text': 'snapshot=' + 'b' * 64}]
                elif event == 'postToolUse':
                    run['entries'] = [{'kind': 'context', 'text': 'PASS\n{"violations":[]}'}]
                send({'method': 'hook/completed', 'params': {'threadId': thread, 'turnId': turn, 'run': run}})
        send({'method': 'thread/tokenUsage/updated', 'params': {'threadId': thread, 'turnId': turn,
            'tokenUsage': {'total': {'inputTokens': 10, 'outputTokens': 5, 'totalTokens': 15}, 'last': {}}}})
        send({'method': 'turn/completed', 'params': {'threadId': thread, 'turn': {'id': turn, 'status': 'completed'}}})
    else:
        raise AssertionError('Unexpected RPC method ' + str(method))
    send({'id': message['id'], 'result': result})
