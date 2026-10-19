"""Score every planned controlled run; unknown/failed runs never disappear."""
import argparse
import json
from pathlib import Path
import statistics

TASKS = ('tax', 'inventory', 'receipt', 'discount')
ARMS = ('bare', 'guarded')


def success(row):
    activation = 'verified_absent' if row.get('arm') == 'bare' else 'verified_active'
    return (row.get('agent_exit') == 0 and row.get('tests_pass') is True
            and row.get('expected_area_touched') is True and row.get('policy_changed') is False
            and row.get('hook_activation') == activation
            and type(row.get('new_violations')) is int and row['new_violations'] == 0
            and not row.get('errors'))


def stats(values):
    return {'measured': len(values), 'mean': statistics.mean(values) if values else None,
            'sample_sd': statistics.stdev(values) if len(values) > 1 else None}


def summarize(data):
    if data.get('repeats') != 3 or {t['id'] for t in data['tasks']} != set(TASKS):
        raise ValueError('expected the four predeclared tasks and three repetitions')
    index = {}
    for row in data['runs']:
        key = (row['task'], row['arm'], row['repeat'])
        if key in index or key[0] not in TASKS or key[1] not in ARMS or key[2] not in (1, 2, 3):
            raise ValueError('duplicate or unexpected run identity')
        index[key] = row
    groups = {}
    for name, tasks in [('boundary', TASKS[:3]), ('control', TASKS[3:]), *[(t, (t,)) for t in TASKS]]:
        groups[name] = {}
        for arm in ARMS:
            rows = [index.get((task, arm, repeat), {}) for task in tasks for repeat in (1, 2, 3)]
            wall = [r['wall_seconds'] for r in rows if type(r.get('wall_seconds')) in (int, float)]
            tokens = [r['tokens']['total'] for r in rows
                      if isinstance(r.get('tokens'), dict) and type(r['tokens'].get('total')) is int]
            fires = [r['edit_blocks'] for r in rows if type(r.get('edit_blocks')) is int]
            groups[name][arm] = dict(planned=len(rows), recorded=sum(bool(r) for r in rows),
                joint_successes=sum(success(r) for r in rows),
                business_passes=sum(r.get('tests_pass') is True for r in rows),
                final_violation_runs=sum(type(r.get('new_violations')) is int and r['new_violations'] > 0 for r in rows),
                final_violations_unknown=sum(type(r.get('new_violations')) is not int for r in rows),
                activation_unknown=sum(r.get('hook_activation') != ('verified_absent' if arm == 'bare' else 'verified_active') for r in rows),
                unavailable_hook_events=sum(e.get('method') == 'hook/completed' and e.get('verdict') == 'UNAVAILABLE'
                    for r in rows for e in r.get('activation_evidence', {}).get('hook_events', [])),
                unavailable_evidence_unknown=sum(not isinstance(r.get('activation_evidence', {}).get('hook_events'), list) for r in rows),
                rule_fire_records=sum(fires), fire_counts_unknown=len(rows)-len(fires),
                wall_seconds_all_runs=stats(wall), tokens_all_runs=stats(tokens))
    control = groups['control']['guarded']
    return {'schema': 'archgraph/controlled-usefulness/v1', 'planned_runs': 24,
            'joint_criterion': 'agent exit 0; business oracle passes; expected area touched; policy unchanged; '
                'verified arm activation/absence; measured zero final violations; no harness errors',
            'groups': groups,
            'control_rule_blocks': 'none observed' if control['fire_counts_unknown'] == 0 and control['rule_fire_records'] == 0
                else 'requires diff/policy inspection; not automatically a false positive',
            'limits': 'Authored synthetic stress tasks, one model/host, three repetitions; bundle of hooks and '
                'generated instructions. No real-world, component-isolated or unconditional productivity claim.'}


def render(data, raw_path, output):
    summary = summarize(data)
    output.with_suffix('.json').write_text(json.dumps(summary, indent=2) + '\n')
    lines = ['# Controlled usefulness test', '', summary['joint_criterion'] + '.', '',
             'Every planned run stays in the denominator; failed, missing and unverified runs are non-successes.', '',
             '| Task/group | Bare success | Guarded success | Bare business pass | Guarded business pass | Guarded rule-fire records | Guarded UNAVAILABLE observed / runs unknown |',
             '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for name, arms in summary['groups'].items():
        b, g = arms['bare'], arms['guarded']
        lines.append(f'| {name} | {b["joint_successes"]}/{b["planned"]} | {g["joint_successes"]}/{g["planned"]} | '
                     f'{b["business_passes"]}/{b["planned"]} | {g["business_passes"]}/{g["planned"]} | '
                     f'{g["rule_fire_records"]} | {g["unavailable_hook_events"]} / {g["unavailable_evidence_unknown"]} |')
    lines += ['', 'All-run costs (mean ± sample SD; unknown values are not imputed as zero):', '',
              '| Group | Arm | Agent seconds | Tokens |', '| --- | --- | ---: | ---: |']
    def cell(value):
        if value['mean'] is None:
            return '? (n=0)'
        sd = '?' if value['sample_sd'] is None else f'{value["sample_sd"]:.1f}'
        return f'{value["mean"]:.1f} ± {sd} (n={value["measured"]})'
    for group in ('boundary', 'control'):
        for arm, row in summary['groups'][group].items():
            lines.append(f'| {group} | {arm} | {cell(row["wall_seconds_all_runs"])} | {cell(row["tokens_all_runs"])} |')
    lines += ['', 'Control rule blocks: ' + summary['control_rule_blocks'] + '.', '',
              'Fire records can repeat one finding and are not a count of independent defects. '
              'UNAVAILABLE is an interruption, not a caught architecture violation. Missing native evidence '
              'has unknown interruption counts, not proven zero.', '', summary['limits'], '',
              f'[All original runs, variance, graph results and source provenance]({raw_path.with_suffix(".md").name}).', '']
    output.with_suffix('.md').write_text('\n'.join(lines))
    return summary


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(render(json.loads(args.raw.read_text()), args.raw, args.output), indent=2))
