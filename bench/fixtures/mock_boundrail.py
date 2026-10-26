#!/usr/bin/env python3
"""Mock architecture CLI for harness plumbing, never benchmark evidence."""
import json
import os
from pathlib import Path
import sys

assert 'CODEX_API_KEY' not in os.environ
assert 'CUSTOM_RUNNER_SECRET' not in os.environ
if 'CODEX_HOME' in os.environ:
    assert not (Path(os.environ['CODEX_HOME']) / 'auth.json').exists()
args = sys.argv[1:]
root = Path.cwd()
if args[:2] == ['hook', 'install']:
    agent = args[args.index('--agent') + 1]
    path = root / ('.codex/hooks.json' if agent == 'codex' else '.claude/settings.json')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text('{"hooks":{"mock":true}}\n')
    print(json.dumps({'written': [str(path)]}))
elif args == ['sync-rules']:
    (root / 'AGENTS.md').write_text('<!-- boundrail:start -->mock<!-- boundrail:end -->\n')
    print('{}')
elif len(args) > 1 and args[1] == 'verify':
    finding = args[2]
    receipt = json.loads((root / '.boundrail/applied' / (finding + '.json')).read_text())
    if set(receipt) != {'schema', 'finding_id', 'before', 'recipe', 'fingerprints'}:
        raise SystemExit(2)
    print(json.dumps({'kind': 'verification', 'finding_id': finding, 'verify': {'pass': True}}))
elif 'mcp-call' in args:
    policy = root / 'layout.yaml'
    enabled = not policy.exists() or 'forbid:' in policy.read_text()
    violation = enabled and 'forbidden' in (root / 'src/domain.txt').read_text()
    print(json.dumps({'new_forbidden': [{'from': 'domain', 'to': 'adapters'}] if violation else [],
                      'new_cycles': [], 'grown_cycles': [], 'findings': []}))
else:
    raise SystemExit('unexpected mock command: ' + repr(args))
