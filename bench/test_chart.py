"""Keep the README chart's established cards, shared scales and measured values."""
import copy
import json
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

import chart


NS = {'s': 'http://www.w3.org/2000/svg'}


class ChartTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.result = json.loads((Path(__file__).parent / 'results/2026-10-11-b8bdd76bf789.json').read_text())

    def test_original_cards_geometry_colors_and_raw_comparisons(self):
        tree = ET.fromstring(chart.svg(self.result))
        self.assertEqual('0 0 960 940', tree.get('viewBox'))
        cards = tree.findall('s:g/s:g', NS)
        self.assertEqual(['vscode', 't3code-server', 'mui-docs'], [card.get('id') for card in cards])
        for card, y in zip(cards, [88, 370, 652]):
            box = card.find('s:rect', NS)
            self.assertEqual(('24', str(y), '912', '266', '#ffffff'),
                             tuple(box.get(key) for key in ('x', 'y', 'width', 'height', 'fill')))
        visible = ''.join(node.text or '' for node in tree.findall('.//s:text', NS))
        for value in ['10,427 files', '2,945 files', '16,766 files', 'Heavy use of Effect and Zod',
                      '12.7× faster', '73% less', '10.0× faster', '65% less', '14.2× faster', '79% less']:
            self.assertIn(value, visible)
        self.assertNotIn('bun', visible)
        bars = cards[0].findall('s:g/s:rect[@data-compiler]', NS)
        self.assertEqual(['272.5305', '21.4049', '379.3859', '104.0598'], [bar.get('width') for bar in bars])
        self.assertEqual(['#3178c6', '#087b5b'] * 2, [bar.get('fill') for bar in bars])

    def test_shared_scales_grow_for_all_cards_without_clipping(self):
        result = copy.deepcopy(self.result)
        result['projects']['vscode']['wide']['tsgo'].update(wall_s=20, peak_rss_bytes=12 * chart.GIB)
        result['projects']['t3code-server']['wide']['tsgo'].update(wall_s=10, peak_rss_bytes=6 * chart.GIB)
        tree = ET.fromstring(chart.svg(result))
        for name, expected in [('vscode', 398), ('t3code-server', 199)]:
            card = tree.find(f"s:g/s:g[@id='{name}']", NS)
            for bar in card.findall("s:g/s:rect[@data-compiler='tsgo']", NS):
                self.assertEqual(expected, float(bar.get('width')))
        self.assertTrue(all(float(bar.get('width')) <= 398 for bar in tree.findall('.//s:rect[@data-compiler]', NS)))

    def test_partial_failed_and_nonfinite_measurements_are_unavailable(self):
        result = copy.deepcopy(self.result)
        result['projects'] = {'vscode': result['projects']['vscode']}
        cell = result['projects']['vscode']['wide']
        cell['tsgo']['ok_runs'] = 0
        cell['tsrs']['wall_s'] = float('nan')
        cell['tsrs']['peak_rss_bytes'] = float('inf')
        tree = ET.fromstring(chart.svg(result))
        self.assertEqual(3, len(tree.findall('s:g/s:g', NS)))
        self.assertFalse(tree.findall('.//s:rect[@data-compiler]', NS))
        for panel in tree.findall('s:g/s:g/s:g', NS):
            self.assertEqual('n/a', panel.findall('s:text', NS)[1].text)

    def test_accessible_metadata_uses_selected_hardware_and_current_run(self):
        result = copy.deepcopy(self.result)
        result['projects'] = {'vscode': result['projects']['vscode']}
        result['projects']['vscode']['wide']['machine']['label'] = '16 vCPU <runner> & host'
        image = chart.svg(result)
        text = ''.join(ET.fromstring(image).itertext())
        for expected in ['16 vCPU <runner> & host', result['date'], result['tsrs']['commit'],
                         'Median of 10 interleaved runs (3 for tsgo)']:
            self.assertIn(expected, text)
        self.assertNotIn('depot-ubuntu-24.04-8', text)
        self.assertEqual(image, chart.svg(result))

    def test_no_requested_mode_fails(self):
        result = copy.deepcopy(self.result)
        for project in result['projects'].values():
            project.pop('wide', None)
        with self.assertRaisesRegex(ValueError, "no benchmark results for mode 'wide'"):
            chart.svg(result)


if __name__ == '__main__':
    unittest.main()
