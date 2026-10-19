#!/usr/bin/env python3
"""Codex 0.163 app-server runner: exact reviewed hook trust, never blanket trust."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import stat
import subprocess
import sys
import threading
import time

EVENTS = {'preToolUse': 'pre', 'postToolUse': 'post', 'stop': 'stop'}


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('expected a regular, non-symlink file')
    return hashlib.sha256(path.read_bytes()).hexdigest()


def quote(value):
    return "'" + str(value).replace("'", "'\\''") + "'"


def private_directory(path):
    path = Path(path)
    if path.is_symlink() or not path.is_dir():
        raise ValueError('profile/artifact directory must exist and not be a symlink')
    meta = path.stat()
    if os.name != 'posix' or meta.st_uid != os.getuid() or stat.S_IMODE(meta.st_mode) & 0o077:
        raise ValueError('runner requires a private owned POSIX directory')
    return path.resolve(strict=True)


def link_auth(profile, source):
    source = Path(source)
    meta = source.lstat()
    if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or stat.S_IMODE(meta.st_mode) & 0o077:
        raise ValueError('auth source must be a private owned regular file')
    target = private_directory(profile) / 'auth.json'
    if target.exists() or target.is_symlink():
        raise ValueError('isolated profile already has auth.json')
    target.symlink_to(source.resolve(strict=True))
    parent, link = profile.stat(), target.lstat()
    return (parent.st_dev, parent.st_ino, link.st_dev, link.st_ino, os.readlink(target))


def cleanup_auth(profile, identity):
    profile = private_directory(profile)
    parent = profile.stat()
    if (parent.st_dev, parent.st_ino) != identity[:2]:
        raise ValueError('private auth profile identity changed')
    target = profile / 'auth.json'
    try:
        current = target.lstat()
    except FileNotFoundError:
        return
    own_link = (stat.S_ISLNK(current.st_mode) and (current.st_dev, current.st_ino) == identity[2:4]
                and os.readlink(target) == identity[4])
    sdk_file = (stat.S_ISREG(current.st_mode) and current.st_uid == os.getuid()
                and not stat.S_IMODE(current.st_mode) & 0o077)
    if not own_link and not sdk_file:
        raise ValueError('reserved auth artifact changed unexpectedly; evaluation is unsafe')
    target.unlink()


def reject_project_config(cwd):
    for parent, directories, files in os.walk(cwd, followlinks=False):
        directories[:] = [name for name in directories if name not in ('.git', '.archgraph')]
        if '.codex' not in directories and '.codex' not in files:
            continue
        folder = Path(parent) / '.codex'
        if folder.is_symlink() or not folder.is_dir():
            raise ValueError('project Codex configuration path is unsafe')
        if folder != cwd / '.codex' or any(item.name != 'hooks.json' or not item.is_file()
                                           or item.is_symlink() for item in folder.iterdir()):
            raise ValueError('project Codex config/exec policies must be removed before trust')
        directories.remove('.codex')


def validate_thread(value, cwd, model):
    policy = value.get('sandbox', {})
    roots = policy.get('writableRoots', [])
    if (value.get('model') != model or value.get('cwd') != str(cwd)
            or value.get('thread', {}).get('cwd') != str(cwd)
            or value.get('modelProvider') != 'openai' or value.get('approvalPolicy') != 'never'
            or policy.get('type') != 'workspaceWrite' or policy.get('networkAccess', False)
            or not isinstance(roots, list) or any(Path(root).resolve() != cwd for root in roots)):
        raise ValueError('effective model/provider/cwd/sandbox/approval differs from reviewed settings')


def inventory(response, cwd, binary, mode, trusted=False):
    entries = response.get('data')
    if not isinstance(entries, list) or len(entries) != 1:
        raise ValueError('expected exactly one hook inventory for this clone')
    entry = entries[0]
    if entry.get('cwd') != str(cwd) or entry.get('errors') or entry.get('warnings'):
        raise ValueError('hook inventory has wrong cwd, errors or warnings')
    hooks = entry.get('hooks')
    if not isinstance(hooks, list):
        raise ValueError('missing hook inventory')
    if mode == 'bare':
        if hooks:
            raise ValueError('bare runner found unexpected hooks')
        return []
    if len(hooks) != 3 or {hook.get('eventName') for hook in hooks} != set(EVENTS):
        raise ValueError('guarded runner requires exactly the three reviewed hooks')
    if len({hook.get('key') for hook in hooks}) != 3:
        raise ValueError('duplicate hook identity')
    source = str(cwd / '.codex/hooks.json')
    for hook in hooks:
        event = EVENTS[hook['eventName']]
        command = f'{quote(binary)} hook run --agent codex --event {event} --root {quote(cwd)} # archgraph:managed:{event}'
        if (hook.get('sourcePath') != source or hook.get('handlerType') != 'command'
                or hook.get('command') != command or hook.get('matcher') not in (None, '')
                or hook.get('source') != 'project' or hook.get('isManaged') is not False
                or hook.get('async', False) or not hook.get('currentHash') or not hook.get('key')):
            raise ValueError('hook differs from exact reviewed command/source/matcher')
        if trusted and (not hook.get('enabled') or hook.get('trustStatus') != 'trusted'):
            raise ValueError('reviewed hook is not enabled and trusted')
    return sorted(hooks, key=lambda hook: hook['eventName'])


def evidence(events, cwd, thread, turn, mode):
    started, completed = {}, {}
    source = str(cwd / '.codex/hooks.json')
    safe = []
    for event in events:
        if event.get('method') not in ('hook/started', 'hook/completed'):
            continue
        params = event['params']
        run = params['run']
        if (params.get('threadId') != thread or params.get('turnId') != turn
                or run.get('sourcePath') != source or run.get('eventName') not in EVENTS
                or run.get('handlerType') != 'command'):
            raise ValueError('hook event is not correlated with reviewed thread/turn/source')
        if mode == 'bare':
            raise ValueError('bare run executed a hook')
        if event['method'] == 'hook/started':
            if run['id'] in started:
                raise ValueError('duplicate hook start')
            started[run['id']] = run['eventName']
        else:
            if (run['id'] in completed or started.get(run['id']) != run['eventName']
                    or run.get('status') not in ('completed', 'blocked')):
                raise ValueError('hook did not complete successfully after matching start')
            completed[run['id']] = run['eventName']
        texts = [entry.get('text', '') for entry in run.get('entries', [])]
        joined = '\n'.join(texts)
        snapshot = re.search(r'\bsnapshot=([a-f0-9]{64})\b', joined)
        verdict = 'UNAVAILABLE' if re.search(r'\bUNAVAILABLE\b', joined) else next(
            (word for word in ('BLOCK', 'PASS') if re.search(r'(?m)^' + word + r'(?:\s|$)', joined)), None)
        safe.append({'method': event['method'], 'threadId': thread, 'turnId': turn,
                     'id': run['id'], 'eventName': run['eventName'], 'sourcePath': source,
                     'status': run.get('status'), 'snapshot': snapshot[1] if snapshot else None,
                     'verdict': verdict})
    if mode == 'bare':
        return safe, 'verified_absent'
    if set(started) != set(completed) or set(completed.values()) != set(EVENTS):
        return safe, 'unknown'
    pre = any(item['method'] == 'hook/completed' and item['eventName'] == 'preToolUse'
              and item['snapshot'] for item in safe)
    post = any(item['method'] == 'hook/completed' and item['eventName'] == 'postToolUse'
               and item['verdict'] in ('PASS', 'BLOCK') for item in safe)
    stops = [item for item in safe if item['method'] == 'hook/completed' and item['eventName'] == 'stop']
    stop = bool(stops) and stops[-1]['status'] == 'completed'
    if any(item['verdict'] == 'UNAVAILABLE' for item in safe):
        return safe, 'unknown'
    return safe, 'verified_active' if pre and post and stop else 'unknown'


class Rpc:
    def __init__(self, command, cwd, artifact, timeout):
        self.events, self.responses, self.number = [], {}, 0
        self.deadline = time.monotonic() + timeout
        self.log = open(artifact / 'rpc.jsonl', 'x', encoding='utf-8', opener=lambda p, f: os.open(p, f, 0o600))
        self.stderr = open(artifact / 'server.stderr', 'xb', opener=lambda p, f: os.open(p, f, 0o600))
        self.proc = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=self.stderr, text=True)
        self.incoming = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        try:
            for line in self.proc.stdout:
                self.incoming.put(json.loads(line))
        except (ValueError, OSError) as error:
            self.incoming.put(error)
        finally:
            self.incoming.put(EOFError('app-server closed before completion'))

    def log_message(self, direction, message):
        # Auth/account RPC payloads are deliberately never recorded.
        if any(word in message.get('method', '').lower() for word in ('auth', 'account', 'login')):
            return
        self.log.write(json.dumps({'direction': direction, 'message': message}) + '\n')
        self.log.flush()

    def send(self, message):
        self.log_message('out', message)
        self.proc.stdin.write(json.dumps(message) + '\n')
        self.proc.stdin.flush()

    def next(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('app-server deadline exceeded')
        try:
            message = self.incoming.get(timeout=remaining)
        except queue.Empty as error:
            raise TimeoutError('app-server deadline exceeded') from error
        if isinstance(message, Exception):
            raise message
        self.log_message('in', message)
        if 'method' in message:
            if 'id' in message:
                self.send({'id': message['id'], 'error': {'code': -32601, 'message': 'Interactive requests disabled'}})
                raise ValueError('unexpected interactive server request')
            self.events.append(message)
        else:
            self.responses[message['id']] = message
        return message

    def request(self, method, params):
        self.number += 1
        identifier = self.number
        self.send({'id': identifier, 'method': method, 'params': params})
        while identifier not in self.responses:
            self.next()
        response = self.responses.pop(identifier)
        if 'error' in response:
            raise ValueError(f'app-server rejected {method}; inspect private diagnostics')
        return response['result']

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self.proc.stdin.close()
        self.proc.stdout.close()
        self.stderr.close()
        self.log.close()


def _run(args, command=None):
    cwd = Path.cwd().resolve(strict=True)
    binary = Path(args.archgraph).resolve(strict=True)
    if digest(binary) != args.archgraph_sha256:
        raise ValueError('archgraph binary digest differs from reviewed binary')
    profile = private_directory(os.environ['CODEX_HOME'])
    artifact = private_directory(args.artifact_dir)
    hook_path = cwd / '.codex/hooks.json'
    config_hash = digest(hook_path) if args.mode == 'guarded' else 'absent'
    if args.mode == 'bare' and (hook_path.exists() or hook_path.is_symlink()):
        raise ValueError('bare clone still contains hook configuration')
    if args.mode == 'guarded' and not args.trust_reviewed_archgraph:
        raise ValueError('guarded hook trust requires explicit --trust-reviewed-archgraph')
    version = 'mock-injected-protocol'
    if command is None:
        version_result = subprocess.run([args.codex, '--version'], cwd=cwd, capture_output=True, text=True, timeout=10)
        if version_result.returncode != 0 or version_result.stdout.strip() != 'codex-cli 0.163.0':
            raise ValueError('runner requires reviewed codex-cli 0.163.0 protocol')
        version = version_result.stdout.strip()
    rpc = Rpc(command or [args.codex, 'app-server', '--disable', 'apps'], cwd, artifact, args.timeout)
    try:
        rpc.request('initialize', {'clientInfo': {'name': 'archgraph_benchmark', 'version': '1'},
                                  'capabilities': {'experimentalApi': True}})
        rpc.send({'method': 'initialized'})
        config_file = str(profile / 'config.toml')
        def configure(key, value):
            return rpc.request('config/batchWrite', {'filePath': config_file, 'reloadUserConfig': True,
                'edits': [{'keyPath': key, 'value': value, 'mergeStrategy': 'upsert'}]})
        configure('features.hooks', True)
        configure('projects', {str(cwd): {'trust_level': 'trusted'}})
        before = inventory(rpc.request('hooks/list', {'cwds': [str(cwd)]}), cwd, binary, args.mode)
        if before:
            configure('hooks.state', {hook['key']: {'trusted_hash': hook['currentHash']} for hook in before})
        after = inventory(rpc.request('hooks/list', {'cwds': [str(cwd)]}), cwd, binary, args.mode, trusted=True)
        stable_keys = ('key', 'currentHash', 'sourcePath', 'eventName', 'command', 'matcher')
        if [[hook.get(key) for key in stable_keys] for hook in before] != [[hook.get(key) for key in stable_keys] for hook in after]:
            raise ValueError('reviewed hook inventory changed during trust grant')
        inventory_hash = hashlib.sha256(json.dumps(after, sort_keys=True).encode()).hexdigest()
        thread_response = rpc.request('thread/start', {'cwd': str(cwd), 'model': args.model,
            'allowProviderModelFallback': False, 'sandbox': 'workspace-write', 'approvalPolicy': 'never',
            'ephemeral': True})
        thread = thread_response['thread']['id']
        validate_thread(thread_response, cwd, args.model)
        mcp = rpc.request('mcpServerStatus/list', {'threadId': thread})
        if mcp.get('data') != [] or mcp.get('nextCursor') is not None:
            raise ValueError('unexpected MCP servers in isolated benchmark thread')
        turn_response = rpc.request('turn/start', {'threadId': thread, 'model': args.model,
            'input': [{'type': 'text', 'text': args.prompt}], 'approvalPolicy': 'never'})
        turn = turn_response['turn']['id']
        while not any(event.get('method') == 'turn/completed' and event.get('params', {}).get('threadId') == thread
                      and event['params'].get('turn', {}).get('id') == turn for event in rpc.events):
            rpc.next()
        terminal = next(event['params']['turn'] for event in rpc.events if event.get('method') == 'turn/completed'
                        and event['params'].get('threadId') == thread and event['params']['turn']['id'] == turn)
        if terminal['status'] != 'completed':
            raise ValueError('agent turn failed or was interrupted')
        if digest(binary) != args.archgraph_sha256 or (args.mode == 'guarded' and digest(hook_path) != config_hash):
            raise ValueError('reviewed binary or hook config changed during run')
        final = inventory(rpc.request('hooks/list', {'cwds': [str(cwd)]}), cwd, binary, args.mode, trusted=True)
        if final != after:
            raise ValueError('trusted hook inventory changed during run')
        safe_events, activation = evidence(rpc.events, cwd, thread, turn, args.mode)
        totals = [event['params']['tokenUsage']['total'] for event in rpc.events
            if event.get('method') == 'thread/tokenUsage/updated' and event['params'].get('threadId') == thread
            and event['params'].get('turnId') == turn]
        return {'schema': 'archgraph/benchmark-runner/v1', 'run_id': args.run_id, 'mode': args.mode,
            'model': args.model, 'cwd': str(cwd), 'codex_version': version, 'archgraph_sha256': args.archgraph_sha256,
            'hook_config_sha256': config_hash, 'inventory_sha256': inventory_hash,
            'inventory': after,
            'thread_id': thread, 'turn_id': turn, 'turn_status': terminal['status'],
            'effective_settings': {key: thread_response[key] for key in ('cwd', 'model', 'modelProvider', 'approvalPolicy', 'sandbox')},
            'mcp_servers': [],
            'activation': activation, 'hook_events': safe_events, 'usage': totals[-1] if totals else None}
    finally:
        rpc.close()


def run(args, command=None):
    profile = private_directory(os.environ['CODEX_HOME'])
    if list(profile.iterdir()):
        raise ValueError('app-server requires an empty isolated CODEX_HOME')
    reject_project_config(Path.cwd().resolve(strict=True))
    identity = link_auth(profile, args.auth_file) if args.auth_file else None
    try:
        return _run(args, command)
    finally:
        # _run closes the app-server before credentials disappear and evaluators resume.
        if identity is not None:
            cleanup_auth(profile, identity)


def validate_result(value, expected):
    if value.get('schema') != 'archgraph/benchmark-runner/v1':
        raise ValueError('not a supported runner result')
    for key in ('run_id', 'mode', 'model', 'cwd', 'archgraph_sha256', 'hook_config_sha256'):
        if value.get(key) != expected[key]:
            raise ValueError('runner identity or reviewed artifacts do not match')
    if value.get('turn_status') != 'completed' or not value.get('thread_id') or not value.get('turn_id'):
        raise ValueError('runner turn did not complete')
    settings = dict(value.get('effective_settings', {}), thread={'cwd': expected['cwd']})
    validate_thread(settings, Path(expected['cwd']), expected['model'])
    if value.get('mcp_servers') != []:
        raise ValueError('runner did not prove an empty MCP inventory')
    hooks = inventory({'data': [{'cwd': expected['cwd'], 'errors': [], 'warnings': [],
        'hooks': value.get('inventory')}]}, Path(expected['cwd']), expected['archgraph'], expected['mode'], trusted=True)
    if hashlib.sha256(json.dumps(hooks, sort_keys=True).encode()).hexdigest() != value.get('inventory_sha256'):
        raise ValueError('runner inventory digest mismatch')
    raw = []
    for item in value.get('hook_events', []):
        text = ('snapshot=' + item['snapshot']) if item.get('snapshot') else ''
        if item.get('verdict'):
            text += '\n' + item['verdict']
        raw.append({'method': item['method'], 'params': {'threadId': item['threadId'],
            'turnId': item['turnId'], 'run': {'id': item['id'], 'eventName': item['eventName'],
            'sourcePath': item['sourcePath'], 'handlerType': 'command', 'status': item['status'],
            'entries': [{'kind': 'context', 'text': text}]}}})
    _, activation = evidence(raw, Path(expected['cwd']), value['thread_id'], value['turn_id'], expected['mode'])
    if value.get('activation') != activation:
        raise ValueError('runner activation claim does not match correlated events')
    return activation


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('model', 'prompt', 'run-id', 'archgraph', 'archgraph-sha256', 'artifact-dir'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--mode', choices=('bare', 'guarded'), required=True)
    parser.add_argument('--trust-reviewed-archgraph', action='store_true')
    parser.add_argument('--auth-file')
    parser.add_argument('--codex', default='codex')
    parser.add_argument('--timeout', type=float, default=600)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args)))
    except Exception as error:
        safe_error = str(error) if type(error) in (ValueError, TimeoutError) else type(error).__name__
        print(json.dumps({'schema': 'archgraph/benchmark-runner-error/v1',
                          'error': safe_error, 'hint': 'Inspect private diagnostics; no credential payloads emitted.'}))
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
