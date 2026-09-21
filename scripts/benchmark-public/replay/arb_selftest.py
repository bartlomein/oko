#!/usr/bin/env python3
"""Offline checks for the Agent Retrieval Bench harness: no binaries, network, or paid calls."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import arb

SAMPLE = {'id': 'abc', 'task_type': 'trace2code', 'repo': 'org/proj', 'base_commit': 'c0ffee',
          'query': {'command': 'go test ./.', 'failure_excerpt': 'FAIL listeners.go:527', 'nested': {'note': ['  ', 'x']}}}


class ArbTests(unittest.TestCase):
    def test_question_is_the_query_text_shortest_first_and_fits_oko(self):
        text, trimmed = arb.question_of(SAMPLE)
        self.assertEqual(text, 'x\n\ngo test ./.\n\nFAIL listeners.go:527')
        self.assertFalse(trimmed)
        long, trimmed = arb.question_of({'query': {'log': 'é' * 5000}})
        self.assertTrue(trimmed)
        self.assertLessEqual(len(long.encode()), arb.QUESTION_BYTES)

    def test_split_is_stable(self):
        self.assertEqual(arb.split_of('abc'), arb.split_of('abc'))
        self.assertEqual({arb.split_of(str(n)) for n in range(40)}, {'dev', 'heldout'})

    def test_workspace_is_rebuilt_from_whole_file_rows_only(self):
        with tempfile.TemporaryDirectory() as temp:
            data = Path(temp)
            folder = data / 'corpus/v2_trace2code/org__proj'
            folder.mkdir(parents=True)
            rows = [{'kind': 'file', 'path': 'pkg/a.go', 'text': 'package a\nfunc A() {}'},
                    {'kind': 'symbol', 'path': 'pkg/a.go', 'text': 'func A() {}'}]
            (folder / 'c0ffee.chunks.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in rows))
            with patch.object(arb, 'DATA', data):
                self.assertEqual(arb.build_workspace(SAMPLE, data / 'work'), 1)
                self.assertEqual((data / 'work/pkg/a.go').read_text(), 'package a\nfunc A() {}\n')
                (folder / 'c0ffee.chunks.jsonl').write_text(json.dumps({'kind': 'file', 'path': '../x', 'text': ''}) + '\n')
                with self.assertRaises(ValueError):
                    arb.build_workspace(SAMPLE, data / 'other')

    def test_ranking_is_shown_excerpts_then_candidates_by_score(self):
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp)
            (work / 'a.go').write_text('1\n2\n3\n4\n')
            packet = {'results': [{'path': 'a.go', 'startLine': 2, 'endLine': 3}],
                      'retrieval': {'candidates': [{'path': 'b.go', 'startLine': 1, 'endLine': 2, 'score': 0.1},
                                                   {'path': 'a.go', 'startLine': 2, 'endLine': 3, 'score': 0.9},
                                                   {'path': 'c.go', 'startLine': 1, 'endLine': 1, 'score': 0.4}]}}
            chunks, shown = arb.ranked(packet, work)
        self.assertEqual(shown, 1)
        self.assertEqual([c['path'] for c in chunks], ['a.go', 'c.go', 'b.go'])
        self.assertEqual(chunks[0]['text'], '2\n3')

    def test_errors_count_as_zero(self):
        ok = {'task': 'trace2code', 'metrics': dict.fromkeys(arb.HEADLINE, 1.0), 'goldShortlisted': 1.0, 'rankedFiles': 9}
        summary = arb.summarize([ok, {'task': 'trace2code', 'error': 'x'}])
        self.assertEqual(summary['all']['Recall@5'], 0.5)
        self.assertEqual(summary['trace2code']['errors'], 1)


if __name__ == '__main__':
    unittest.main()
