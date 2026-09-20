#!/usr/bin/env python3
"""Offline checks for the retrieval replay: no binaries, network, or paid calls."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('replay', Path(__file__).resolve().parent / 'replay.py')  # noqa: E501
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)


class ReplayTests(unittest.TestCase):
    def test_questions_come_from_all_three_clients_once_each(self):
        runs = [
            {'id': 'task', 'tools': [
                {'server': 'oko', 'tool': 'search', 'arguments': {'question': 'where is auth?', 'intent': 'implementation'}},
                {'name': 'mcp__oko__search', 'input': {'question': 'where is auth?', 'intent': 'implementation'}},
                {'tool': 'oko_search', 'state': json.dumps({'input': {'question': 'token check', 'deep': True,
                                                                        'directory': '/tmp/x/099-task-opencode/workspace/src'}})},
                {'tool': 'oko_search', 'state': {'input': {'question': 'root', 'directory': '/tmp/x/workspace'}}},
                {'tool': 'oko_search', 'state': {'input': {'question': 'elsewhere', 'directory': '/somewhere/else'}}},
                {'name': 'Grep', 'input': {'question': 'not oko'}},
                {'name': 'mcp__oko__search', 'input': {'pattern': 'no question'}}]},
            {'id': 'other-task', 'tools': [{'server': 'oko', 'arguments': {'question': 'ignored'}}]}]
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / 'report.json'
            report.write_text(json.dumps({'runs': runs}))
            found = r.collect_questions([report, report], {'task': ('repo', {})})
        self.assertEqual(found, {'task': [
            {'question': 'where is auth?', 'intent': 'implementation'},
            {'question': 'token check', 'directory': 'src'},
            {'question': 'root'},
            {'question': 'elsewhere'}]})

    def test_anchors_cover_search_locations_and_the_lines_an_edit_must_change(self):
        search = {'kind': 'search', 'expected': [{'path': 'a.ts', 'startLine': 46, 'endLine': 46},
                                                  {'path': 'b.ts', 'startLine': 135, 'endLine': 141}]}
        self.assertEqual(r.anchors(search), [('a.ts', 46, 46), ('b.ts', 135, 141)])
        self.assertEqual(r.anchors({'kind': 'edit', 'path': 'c.py', 'allowedLines': [38, 43]}), [('c.py', 38, 43)])
        self.assertEqual(r.anchors({'kind': 'edit', 'path': 'c.py', 'allowedLines': [1, 2],
                                    'allowedRanges': [[1, 1], [4, 4]]}), [('c.py', 1, 1), ('c.py', 4, 4)])

    def test_score_requires_the_whole_anchor_in_one_excerpt_and_respects_subdirectories(self):
        expected = [('src/a.ts', 46, 46), ('src/a.ts', 81, 81), ('src/b.ts', 135, 141)]
        packet = {'results': [{'path': 'src/a.ts', 'startLine': 18, 'endLine': 77, 'truncated': True}],
                  'related': [{'path': 'src/b.ts', 'startLine': 118, 'endLine': 140}]}
        scored = r.score(packet, expected)
        self.assertEqual((scored['covered'], scored['expected'], scored['full']), (1, 3, False))
        self.assertEqual((scored['excerpts'], scored['lines'], scored['results']), (2, 83, 1))
        whole = {'results': [{'path': 'a.ts', 'startLine': 18, 'endLine': 136},
                             {'path': 'b.ts', 'startLine': 118, 'endLine': 143}], 'related': []}
        self.assertFalse(r.score(whole, expected)['full'], 'paths are relative to the searched directory')
        self.assertTrue(r.score(whole, expected, 'src')['full'])
        self.assertEqual(r.score({'results': [], 'related': []}, expected)['covered'], 0)

    def test_missed_locations_are_placed_when_the_build_records_its_candidates(self):
        expected = [('a.ts', 46, 46), ('b.ts', 10, 12), ('c.ts', 5, 5), ('d.ts', 1, 1)]
        packet = {'results': [{'path': 'a.ts', 'startLine': 18, 'endLine': 136}], 'related': [],
                  'retrieval': {'candidates': [
                      {'path': 'a.ts', 'startLine': 18, 'endLine': 136, 'score': 0.9},
                      {'path': 'b.ts', 'startLine': 1, 'endLine': 40, 'score': 0.31},
                      {'path': 'c.ts', 'startLine': 1, 'endLine': 40, 'score': 0.04}]}}
        self.assertEqual(r.score(packet, expected)['missed'], {'listed': 1, 'rejected': 1, 'notShortlisted': 1})
        keyword = {'results': [], 'related': [], 'retrieval': {'candidates': [{'path': 'b.ts', 'startLine': 1, 'endLine': 40}]}}
        self.assertEqual(r.score(keyword, expected[1:2])['missed']['listed'], 1)
        self.assertNotIn('missed', r.score({'results': [], 'related': []}, expected))

    def test_summary_pairs_the_same_question_and_keeps_failures_visible(self):
        def row(build, question, covered, **extra):
            return {'build': build, 'task': 'task', 'question': question, 'covered': covered, 'expected': 2,
                    'full': covered == 2, 'excerpts': 1, 'lines': 10, 'responseBytes': 100, 'okoMs': 700,
                    'jevMs': 500, 'firstInProcess': False, 'ranking': 'jev', **extra}
        rows = [row('old', 'q1', 1), row('old', 'q2', 2), row('old', 'q3', 2),
                row('new', 'q1', 2), row('new', 'q2', 1, ranking='lexical-fallback'),
                {'build': 'new', 'task': 'task', 'question': 'q3', 'error': 'RuntimeError: busy'}]
        summary = r.summarize(rows, ['old', 'new'])
        self.assertAlmostEqual(summary['builds']['old']['anchorsCovered'], 5 / 6)
        self.assertEqual(summary['builds']['new']['errors'], 1)
        self.assertEqual(summary['builds']['new']['fallbacks'], 1)
        self.assertEqual(summary['builds']['old']['okoLocalMs'], 200)
        paired = summary['paired']['new']
        self.assertEqual((paired['pairs'], paired['gainedFullCoverage'], paired['lostFullCoverage']), (2, 1, 1))
        text = r.render(summary, ['old', 'new'], 'note')
        self.assertIn('| old | 3 | 83% | 67% |', text)
        self.assertIn('Failed queries: 1. Keyword fallbacks (Jev slow or unavailable): 1.', text)

    def test_nothing_is_sent_without_execute(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'oko'
            binary.write_text('not run')
            self.assertEqual(r.main(['--build', f'a={binary}', '--limit', '1']), 0)
            self.assertFalse(list(Path(directory).glob('results-*')))


if __name__ == '__main__':
    unittest.main()
