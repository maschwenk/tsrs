"""The chart must keep measured hardware and unavailable samples honest."""
import copy
import json
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

import chart


class ChartTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.result = json.loads((Path(__file__).parent / 'results/2026-10-08-6e82c526ff8e.json').read_text())

    def test_mode_hardware_reps_and_escaping(self):
        result = copy.deepcopy(self.result)
        result['projects'] = {'a < b & c': result['projects']['vscode']}
        image = chart.svg(result)
        tree = ET.fromstring(image)
        text = ''.join(tree.itertext())
        self.assertIn('a < b & c', text)
        self.assertIn('Default settings on 64 vCPUs', text)
        self.assertIn('Median of 10 interleaved runs (3 for tsgo)', text)
        self.assertNotIn('depot-ubuntu-24.04-8', text)
        self.assertEqual(image, chart.svg(result))

    def test_failed_and_missing_measurements_have_no_bar(self):
        result = copy.deepcopy(self.result)
        result['projects'] = {'vscode': result['projects']['vscode']}
        result['projects']['vscode']['wide']['tsgo']['ok_runs'] = 0
        result['projects']['vscode']['wide']['bun']['wall_s'] = None
        tree = ET.fromstring(chart.svg(result))
        ns = {'s': 'http://www.w3.org/2000/svg'}
        groups = tree.findall('s:g', ns)
        unavailable = [group for group in groups if 'n/a' in ''.join(group.itertext())]
        self.assertEqual(3, len(unavailable))
        self.assertTrue(all(group.find('s:rect', ns) is None for group in unavailable))

    def test_no_requested_mode_fails(self):
        result = copy.deepcopy(self.result)
        for project in result['projects'].values():
            project.pop('wide', None)
        with self.assertRaisesRegex(ValueError, "no benchmark results for mode 'wide'"):
            chart.svg(result)


if __name__ == '__main__':
    unittest.main()
