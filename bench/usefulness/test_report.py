import unittest
from report import TASKS, summarize


class ScoringTests(unittest.TestCase):
    def test_failures_unknown_and_missing_stay_in_fixed_denominators(self):
        rows = [dict(task=t, arm=a, repeat=i, agent_exit=0, tests_pass=True,
                     expected_area_touched=True, policy_changed=False,
                     hook_activation='verified_absent' if a == 'bare' else 'verified_active',
                     new_violations=0, errors=[], edit_blocks=0, wall_seconds=1,
                     tokens={'total': 10}, activation_evidence={'hook_events': []})
                for t in TASKS for a in ('bare', 'guarded') for i in (1, 2, 3)]
        data = dict(repeats=3, tasks=[{'id': t} for t in TASKS], runs=rows)
        self.assertEqual(summarize(data)['groups']['boundary']['guarded']['joint_successes'], 9)
        rows[3]['tests_pass'] = False
        rows[4]['hook_activation'] = 'unknown'
        rows[4].pop('activation_evidence')
        rows[5]['new_violations'] = None
        rows.pop(9)
        result = summarize(data)['groups']['boundary']['guarded']
        self.assertEqual((result['planned'], result['recorded'], result['joint_successes']), (9, 8, 5))
        self.assertEqual(result['wall_seconds_all_runs']['measured'], 8)
        self.assertEqual(result['unavailable_evidence_unknown'], 2)
        data['runs'].append(dict(rows[0]))
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            summarize(data)
