#!/usr/bin/env python3
"""Check the Documenso plan, frozen fixture, and adapters without model calls."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from runner import engine as r
from shared import load_engine


class DocumensoTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        r.SETTINGS = json.loads((r.STATE / 'settings.json').read_text())
        cls.temp = tempfile.TemporaryDirectory(prefix='offline-validation-', dir=r.STATE)
        cls.work = Path(cls.temp.name) / 'workspace'
        r.checkout(cls.work)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def test_dry_plans(self):
        self.assertEqual(sum(t['kind'] == 'search' for t in r.TASKS), 3)
        self.assertEqual(sum(t['kind'] == 'edit' for t in r.TASKS), 2)
        for flags, count in [([], 30), (['--fast'], 30), (['--pilot'], 12),
                             (['--clients', 'codex,claude'], 20)]:
            output = subprocess.check_output([sys.executable, str(r.ROOT / 'runner.py'), *flags], text=True)
            self.assertIn(f'{count} sessions.', output)
            self.assertIn('Execution=False', output)
        self.assertEqual(r.PROJECT_NAME, 'Documenso')
        self.assertEqual(r.STATE.name, 'documenso')
        self.assertEqual(len(r.select_tasks(pilot=True)), 2)

    def test_frozen_source_and_grading(self):
        self.assertEqual(r.digest(r.STATE / 'baseline.tar'), r.SETTINGS['archiveSha256'])
        self.assertEqual(r.digest(r.ROOT / 'tasks.json'), r.SETTINGS['tasksSha256'])
        for task in r.TASKS:
            targets = task['expected'] if task['kind'] == 'search' else [task]
            for target in targets:
                self.assertEqual(r.digest(self.work / target['path']), target['sha256'])
            if task['kind'] == 'search':
                target = task['expected'][0]
                answer = json.dumps({'results': [{key: target[key] for key in ('path', 'startLine', 'endLine')}]})
                self.assertTrue(r.grade(task, self.work, answer)['correctFirst'])
                self.assertFalse(r.grade(task, self.work, '{"results": []}')['correctFirst'])
            else:
                file = self.work / task['path']
                original = file.read_text()
                self.assertEqual(original.count(task['old']), 1)
                self.assertFalse(r.grade(task, self.work, '')['expectedPatchMatch'])
                try:
                    file.write_text(original.replace(task['old'], task['new'], 1))
                    self.assertTrue(r.grade(task, self.work, '')['expectedPatchMatch'])
                finally:
                    file.write_text(original)
        self.assertFalse(r.git(self.work, 'status', '--porcelain'))

    def test_all_thirty_adapter_configurations(self):
        plan = r.make_plan(r.TASKS)
        self.assertEqual(len(plan), 30)
        for task, client, enabled, repeat in plan:
            args, env = r.args_for(task, client, enabled, self.work, Path(self.temp.name))
            self.assertIn(task['question'], args[-1])
            self.assertNotIn('TYPESAFE_API_KEY', env)
            self.assertNotIn('dev/documenso', args[-1])
            self.assertEqual(repeat, 1)
            config = ' '.join(args) + env.get('OPENCODE_CONFIG_CONTENT', '')
            if client != 'claude' or enabled:
                self.assertIn(str(r.ROOT / 'oko-server.py'), config)
            if task['kind'] == 'edit':
                self.assertNotIn(task['path'], args[-1])
                self.assertNotIn(task['old'], args[-1])

    def test_customizations_removed_and_recreation_is_rejected(self):
        path = self.work / '.opencode/.gitignore'
        self.assertFalse(path.parent.exists())
        self.assertEqual(json.loads((self.work.parent / "isolation.json").read_text())["version"], r.ISOLATION_VERSION)
        task = r.TASKS[0]
        target = task['expected'][0]
        answer = json.dumps({'results': [{key: target[key] for key in ('path', 'startLine', 'endLine')}]})
        try:
            path.parent.mkdir()
            path.write_text('unexpected setup')
            self.assertIn('gradingError', r.grade(task, self.work, answer))
        finally:
            path.unlink()
            path.parent.rmdir()
        self.assertTrue(r.grade(task, self.work, answer)['correctFirst'])

    def test_oko_launcher_uses_documenso_state_and_cache(self):
        launcher = load_engine('oko-server')
        cache = Path(self.temp.name) / 'cache'
        with patch.object(sys, 'argv', ['oko-server.py', str(self.work), str(cache)]), \
             patch.dict(os.environ, {'TYPESAFE_API_KEY': 'offline-test-only'}), \
             patch.object(os, 'execve') as execute:
            launcher.main(root=r.ROOT, state_name='documenso')
            executable, args, env = execute.call_args.args
            self.assertEqual(executable, r.SETTINGS['oko'])
            self.assertEqual(args[-1], str(self.work.resolve()))
            self.assertEqual(env['OKO_CACHE_DIR'], str(cache.resolve()))
        with patch.object(sys, 'argv', ['oko-server.py', '/tmp/outside', str(cache)]):
            with self.assertRaisesRegex(RuntimeError, 'within benchmark artifacts'):
                launcher.main(root=r.ROOT, state_name='documenso')


if __name__ == '__main__':
    unittest.main()
