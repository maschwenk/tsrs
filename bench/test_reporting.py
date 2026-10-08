"""Hardware changes must not relabel historical results or look like code regressions."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

import history
import run


class ReportingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.historical = json.loads((Path(__file__).parent / 'results/2026-10-08-6e82c526ff8e.json').read_text())

    def resized(self):
        result = copy.deepcopy(self.historical)
        result['modes'] = ['checkers16' if mode == 'checkers64' else mode for mode in result['modes']]
        result['mode_reps']['checkers16'] = result['mode_reps'].pop('checkers64')
        for project in result['projects'].values():
            project['checkers16'] = project.pop('checkers64')
            for mode in ('wide', 'checkers16'):
                machine = project[mode]['machine']
                machine.update(cpus=16, memory_gb=63, label='Depot CI 16 vCPU')
        return result

    def test_historical_and_new_results_name_their_own_hardware(self):
        old = run.markdown(self.historical, ['wide', 'checkers64'])
        new = run.markdown(self.resized(), ['wide', 'checkers16'])
        self.assertIn('Default mode on a 64-vCPU machine', old)
        self.assertIn('`--checkers 64`', old)
        self.assertIn('Default mode on a 16-vCPU machine', new)
        self.assertIn('bun `--threads 16`', new)
        self.assertNotIn('64-vCPU', new)

    def test_merge_keeps_fixed_and_wide_machine_provenance(self):
        resized = self.resized()
        partials = []
        for modes in (['default', 'single', 'checkers8'], ['wide', 'checkers16']):
            part = copy.deepcopy(resized)
            part['modes'] = modes
            part['projects'] = {'vscode': {mode: copy.deepcopy(resized['projects']['vscode'][mode]) for mode in modes}}
            if 'wide' in modes:
                part['machine'] = part['projects']['vscode']['wide']['machine']
            for cell in part['projects']['vscode'].values():
                cell.pop('machine', None)
            part['raw'] = []
            partials.append(part)
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / f'{i}.json' for i in range(2)]
            for path, part in zip(paths, partials):
                path.write_text(json.dumps(part))
            merged = run.merge_results({'projects': [{'name': 'vscode'}]}, paths)
        self.assertEqual(8, merged['machine']['cpus'])
        self.assertEqual(16, merged['projects']['vscode']['wide']['machine']['cpus'])
        self.assertIn('Default mode on a 16-vCPU machine', run.markdown(merged, ['wide']))

    def test_history_starts_new_baseline_only_for_changed_hardware(self):
        new = self.resized()
        old_wall = self.historical['projects']['vscode']['wide']['tsrs']['wall_s']
        new['projects']['vscode']['wide']['tsrs']['wall_s'] = old_wall * 2
        later = copy.deepcopy(new)
        later['projects']['vscode']['wide']['tsrs']['wall_s'] *= 1.1
        rendered = history.table([self.historical, new, later], ['vscode'], 'wide_wall', True).splitlines()
        self.assertIn('(new baseline)', rendered[2])
        self.assertNotIn('+100.0%', rendered[2])
        self.assertIn('+10.0%', rendered[3])
        fixed = history.table([self.historical, new], ['vscode'], 'wall', True)
        self.assertNotIn('new baseline', fixed)


if __name__ == '__main__':
    unittest.main()
