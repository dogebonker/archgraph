#!/usr/bin/env python3
"""A deterministic attempted violation and guarded repair with a receipt."""
import json
import os
from pathlib import Path
import sys

root = Path.cwd()
assert '--model' in sys.argv and '--prompt' in sys.argv
source = root / 'src/domain.txt'
if 'CUSTOM_RUNNER_SECRET' in os.environ:
    assert os.environ.get('CODEX_API_KEY') == 'fixture-key'
    (root / 'src/auth-observed.txt').write_text('agent saw credentials\n')
    print(os.environ['CUSTOM_RUNNER_SECRET'])
prompt = sys.argv[sys.argv.index('--prompt') + 1]
no_help = prompt == 'No code change needed'
weaken_policy = prompt == 'Weaken policy'
if weaken_policy:
    (root / 'layout.yaml').write_text('# no rules\n')
if not no_help:
    source.write_text('forbidden\n')
if (root / '.claude/settings.json').exists() and not no_help and not weaken_policy:
    state = root / '.archgraph'
    state.mkdir(exist_ok=True)
    (state / 'fires.log').write_text(json.dumps({'rule': 'domain-no-adapters',
        'file': 'src/domain.txt', 'agent': 'claude', 'ts': 1, 'finding_id': 'mock-1'}) + '\n')
    receipts = state / 'applied'
    receipts.mkdir()
    (receipts / 'mock-1.json').write_text(json.dumps({
        'schema': 'archgraph/apply-receipt/v1', 'finding_id': 'mock-1',
        **({'before': {}, 'recipe': {}, 'fingerprints': {}} if prompt == 'Valid receipt' else {})}))
    source.write_text('repaired\n')
print(json.dumps({'type': 'result', 'usage': {'input_tokens': 10, 'output_tokens': 5}}))
