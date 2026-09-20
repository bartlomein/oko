#!/usr/bin/env python3
"""Offline runner tests; no model calls or application dependencies."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import runner as r


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.work = Path(self.temp.name) / 'workspace'
        self.work.mkdir()
        r.git(self.work, 'init', '-q')
        (self.work / 'sample.ts').write_text('export const LIMIT = 10;\n')
        r.git(self.work, 'add', '.')
        r.git(self.work, '-c', 'user.name=Test', '-c', 'user.email=test@localhost',
              '-c', 'commit.gpgsign=false', 'commit', '-qm', 'test baseline')
        self.edit = dict(id='edit', kind='edit', question='Set the limit to 20.',
                         path='sample.ts', old='LIMIT = 10', new='LIMIT = 20')
        self.read = dict(id='read', kind='search', question='Where is the limit?',
                         expected=[dict(path='sample.ts', startLine=1, endLine=1)])
        self.answer = json.dumps({'results': self.read['expected']})
        r.SETTINGS = dict(clients={name: name for name in ('codex', 'opencode', 'claude')},
                          models=dict(codex='test-model', opencode='openai/test-model', claude='test-model'),
                          timeoutSeconds=5)

    def tearDown(self):
        self.temp.cleanup()

    def test_blank_slate_flags_ignore_inherited_customizations(self):
        with patch.dict(r.os.environ, {'OPENCODE_CONFIG': '/poison/config',
                        'CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD': '1',
                        'CODEX_THREAD_ID': 'inherited'}):
            for client in ('codex', 'opencode', 'claude'):
                args, env = r.args_for(self.read, client, True, self.work, Path(self.temp.name))
                self.assertNotIn('OPENCODE_CONFIG', env)
                self.assertNotIn('CODEX_THREAD_ID', env)
                self.assertNotIn('CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD', env)
                if client == 'codex':
                    self.assertIn('skip_host_skill_discovery', args)
                    self.assertIn('project_doc_max_bytes=0', args)
                    self.assertIn('--ignore-rules', args)
                    self.assertEqual(Path(env['CODEX_HOME']), Path(self.temp.name) / 'harness-config/codex')
                    self.assertTrue(any(x.startswith('skills.config=') for x in args))
                elif client == 'opencode':
                    config = json.loads(env['OPENCODE_CONFIG_CONTENT'])
                    self.assertFalse(config['agent']['comparison']['tools']['skill'])
                    self.assertEqual(env['OPENCODE_DISABLE_PROJECT_CONFIG'], '1')
                    self.assertEqual(env['OPENCODE_DISABLE_EXTERNAL_SKILLS'], '1')
                    self.assertTrue(Path(env['XDG_CONFIG_HOME']).is_dir())
                else:
                    self.assertIn('--disable-slash-commands', args)
                    settings = json.loads(args[args.index('--settings') + 1])
                    self.assertEqual(settings['claudeMdExcludes'], ['/**'])
                    self.assertFalse(settings['autoMemoryEnabled'])
                    self.assertTrue(settings['disableAllHooks'])

    def test_strip_customizations_keeps_product_code_and_symlink_targets(self):
        outside = Path(self.temp.name) / 'outside'
        outside.mkdir()
        (outside / 'SKILL.md').write_text('keep me')
        (self.work / '.agents').symlink_to(outside, target_is_directory=True)
        (self.work / 'nested').mkdir()
        (self.work / 'nested/AGENTS.md').write_text('unwanted')
        (self.work / 'nested/SKILL.md').write_text('unwanted')
        removed = r.strip_customizations(self.work)
        self.assertEqual(removed, ['.agents', 'nested/AGENTS.md', 'nested/SKILL.md'])
        self.assertEqual((outside / 'SKILL.md').read_text(), 'keep me')
        self.assertTrue((self.work / 'sample.ts').exists())

    def test_plan_and_cli_filters(self):
        self.assertEqual(sum(t['kind'] == 'search' for t in r.TASKS), 10)
        self.assertEqual(sum(t['kind'] == 'edit' for t in r.TASKS), 5)
        plan = r.make_plan(r.TASKS)
        self.assertEqual(len(plan), 90)
        for task in r.TASKS:
            self.assertEqual({(c, o) for t, c, o, _ in plan if t['id'] == task['id']}, set(r.MODES))
        self.assertEqual(len(r.make_plan(r.TASKS, repeats=3)), 270)
        for flags, expected in [([], '15 tasks, 90 sessions'), (['--pilot'], '2 tasks, 12 sessions'),
                                (['--fast'], '5 tasks, 30 sessions'),
                                (['--fast', '--clients', 'codex,claude'], '5 tasks, 20 sessions'),
                                (['--clients', 'claude', '--condition', 'native'], '15 tasks, 15 sessions')]:
            output = subprocess.check_output([sys.executable, str(r.ROOT / 'runner.py'), *flags], text=True)
            self.assertIn(expected, output)
            self.assertIn('Execution=False', output)

    def test_fast_preset_split_and_conditions(self):
        tasks = r.select_tasks(fast=True)
        self.assertEqual(sum(task['kind'] == 'search' for task in tasks), 3)
        self.assertEqual(sum(task['kind'] == 'edit' for task in tasks), 2)
        self.assertEqual([task['id'] for task in tasks], list(r.FAST_TASK_IDS))
        plan = r.make_plan(tasks)
        self.assertEqual(len(plan), 30)
        for task in tasks:
            self.assertEqual({(c, o) for t, c, o, _ in plan if t['id'] == task['id']}, set(r.MODES))
        result = subprocess.run([sys.executable, str(r.ROOT / 'runner.py'), '--fast', '--pilot'], capture_output=True)
        self.assertNotEqual(result.returncode, 0)

    def test_commands_keep_conditions_and_edit_permissions_separate(self):
        for task in r.TASKS:
            for client, enabled in r.MODES:
                args, env = r.args_for(task, client, enabled, self.work, Path(self.temp.name))
                self.assertIn(task['question'], args[-1])
                self.assertNotIn('TYPESAFE_API_KEY', env)
                self.assertNotIn('expected', args[-1])
                if client == 'codex':
                    self.assertEqual(args[args.index('--sandbox') + 1], 'workspace-write' if task['kind'] == 'edit' else 'read-only')
                elif client == 'opencode':
                    config = json.loads(env['OPENCODE_CONFIG_CONTENT'])
                    self.assertEqual(config['mcp']['oko']['enabled'], enabled)
                    self.assertEqual(config['agent']['comparison']['permission']['edit'], 'allow' if task['kind'] == 'edit' else 'deny')
                else:
                    self.assertIn('--restricted', args)
                    self.assertEqual('Edit' in args[args.index('--tools') + 1], task['kind'] == 'edit')
                    self.assertEqual(bool(json.loads(args[args.index('--mcp-config') + 1])['mcpServers']), enabled)

    def test_reasoning_effort_reaches_each_client(self):
        for effort in ('low', 'medium', 'high'):
            r.SETTINGS['effort'] = effort
            for client in ('codex', 'opencode', 'claude'):
                for task in (self.read, self.edit):
                    for enabled in (False, True):
                        args, _ = r.args_for(task, client, enabled, self.work, Path(self.temp.name))
                        if client == 'codex':
                            self.assertIn('model_reasoning_effort=' + json.dumps(effort), args)
                        else:
                            flag = '--variant' if client == 'opencode' else '--effort'
                            self.assertEqual(args[args.index(flag) + 1], effort)
        r.SETTINGS['effort'] = 'invalid'
        with self.assertRaises(ValueError):
            r.args_for(self.read, 'codex', False, self.work, Path(self.temp.name))

    def test_edit_grading_staged_changes_and_untracked_files(self):
        self.assertFalse(r.grade(self.edit, self.work, '')['expectedPatchMatch'])
        (self.work / 'sample.ts').write_text('export const LIMIT = 20;\n')
        self.assertTrue(r.grade(self.edit, self.work, '')['expectedPatchMatch'])
        r.git(self.work, 'add', 'sample.ts')
        self.assertTrue(r.grade(self.edit, self.work, '')['expectedPatchMatch'])
        (self.work / 'unrelated.txt').write_text('unrequested edit')
        result = r.grade(self.edit, self.work, '')
        self.assertFalse(result['expectedPatchMatch'])
        self.assertTrue(result['reviewRequired'])
        self.assertEqual(result['otherChangedFiles'], ['unrelated.txt'])

    def test_read_grading_rejects_invalid_results_and_edits(self):
        self.assertTrue(r.grade(self.read, self.work, self.answer)['correctFirst'])
        self.assertFalse(r.grade(self.read, self.work, '{"results": []}')['correctTopFive'])
        for path, start, end in [('../outside', 1, 1), ('/tmp/outside', 1, 1),
                                 ('sample.ts', 0, 1), ('sample.ts', 1, 121), ('sample.ts', 1, 2),
                                 ('sample.ts', True, 1)]:
            answer = json.dumps({'results': [dict(path=path, startLine=start, endLine=end)]})
            self.assertIn('gradingError', r.grade(self.read, self.work, answer))
        (self.work / 'sample.ts').write_text('changed\n')
        r.git(self.work, 'add', 'sample.ts')
        self.assertIn('gradingError', r.grade(self.read, self.work, self.answer))

    def test_edit_symlink_rejected(self):
        target = self.work / 'sample.ts'
        target.unlink()
        target.symlink_to(Path(self.temp.name) / 'outside')
        self.assertIn('gradingError', r.grade(self.edit, self.work, ''))

    def test_three_event_formats_and_errors(self):
        streams = {
            'codex': [dict(type='item.completed', item=dict(type='mcp_tool_call', server='oko')),
                      dict(type='item.completed', item=dict(type='agent_message', text=self.answer)),
                      dict(type='turn.completed', usage=dict(input_tokens=10, cached_input_tokens=4, output_tokens=2))],
            'opencode': [dict(type='tool_use', part=dict(tool='oko_search')),
                         dict(type='text', part=dict(messageID='old', text='not final')),
                         dict(type='text', part=dict(messageID='last', text=self.answer)),
                         dict(type='step_finish', part=dict(reason='stop', messageID='last', tokens=dict(total=12)))],
            'claude': [dict(type='assistant', message=dict(content=[dict(type='tool_use', name='mcp__oko__search')])),
                       dict(type='result', is_error=False, result=self.answer,
                            usage=dict(input_tokens=3, cache_read_input_tokens=4, cache_creation_input_tokens=3, output_tokens=2))],
        }
        for client, events in streams.items():
            parsed = r.parse_events(client, events)
            self.assertTrue(parsed['complete'])
            self.assertEqual(parsed['final'], self.answer)
            self.assertEqual(parsed['okoCalls'], 1)
            self.assertIsNone(parsed['tokens']['total'])
            self.assertFalse(r.parse_events(client, [])['complete'])
        self.assertTrue(r.parse_events('claude', [dict(type='result', is_error=True)])['providerErrors'])
        self.assertTrue(r.parse_events('codex', [dict(type='turn.failed')])['providerErrors'])
        self.assertTrue(r.parse_events('opencode', [dict(type='error')])['providerErrors'])

    def test_explicit_provider_totals_are_copied_without_inference(self):
        codex = r.parse_events('codex', [
            dict(type='turn.completed', usage=dict(input_tokens=10, output_tokens=2, total_tokens=17)),
        ])
        self.assertEqual(codex['tokens']['total'], 17)

        claude = r.parse_events('claude', [
            dict(type='result', is_error=False, result=self.answer,
                 usage=dict(input_tokens=3, output_tokens=2, total_tokens=19)),
        ])
        self.assertEqual(claude['tokens']['total'], 19)

        opencode = r.parse_events('opencode', [
            dict(type='text', part=dict(messageID='last', text=self.answer)),
            dict(type='step_finish', part=dict(reason='stop', messageID='last',
                                                tokens=dict(input=10, output=2, total=12),
                                                usage=dict(total_tokens=23))),
        ])
        self.assertEqual(opencode['tokens']['total'], 23)
        self.assertEqual(opencode['tokens']['steps'][0]['total'], 12)

    def test_summary_separates_model_effort_and_observed_condition(self):
        def row(model, effort, observed):
            return dict(
                client='codex', requestedModel=model, requestedEffort=effort,
                oko=True, observedCacheState=observed, kind='search', seconds=1.0,
                tokens={'total': None}, grade={'correctTopFive': True, 'correctFirst': True},
            )

        result = r.summary([
            row('model-a', 'low', None),
            row('model-b', 'low', None),
            row('model-a', 'high', None),
            row('model-a', 'low', 'oko-warm'),
        ])
        self.assertEqual(len(result), 4)
        self.assertEqual(
            {
                (item['client'], item['model'], item['effort'], item['condition']['requested'], item['condition']['observedCacheState'])
                for item in result
            },
            {
                ('codex', 'model-a', 'low', 'oko-cold', None),
                ('codex', 'model-b', 'low', 'oko-cold', None),
                ('codex', 'model-a', 'high', 'oko-cold', None),
                ('codex', 'model-a', 'low', 'oko-cold', 'oko-warm'),
            },
        )

    def test_process_run_saves_artifacts_and_rejects_wrong_condition(self):
        def minimal_checkout(work):
            import shutil
            shutil.copytree(self.work, work)
        events = [dict(type='item.completed', item=dict(type='agent_message', text=self.answer)),
                  dict(type='turn.completed', usage=dict(input_tokens=1, output_tokens=1))]
        command = [sys.executable, '-c', 'print(' + repr('\n'.join(json.dumps(e) for e in events)) + ')']
        with patch.object(r, 'checkout', minimal_checkout), patch.object(r, 'args_for', return_value=(command, None)):
            row = r.run_one(self.read, 'codex', False, Path(self.temp.name), 1)
            self.assertNotIn('error', row)
            self.assertTrue(row['grade']['correctFirst'])
            trial = Path(row['artifact'])
            self.assertTrue((trial / 'result.json').exists())
            self.assertTrue((trial / 'changes.patch').exists())
            self.assertFalse((trial / 'workspace').exists())
            row = r.run_one(self.read, 'codex', True, Path(self.temp.name), 2)
            self.assertIn('Oko usage', row['error'])

    def test_timeout_is_failure(self):
        def minimal_checkout(work):
            import shutil
            shutil.copytree(self.work, work)
        r.SETTINGS['timeoutSeconds'] = 0.05
        command = [sys.executable, '-c', 'import time; time.sleep(10)']
        with patch.object(r, 'checkout', minimal_checkout), patch.object(r, 'args_for', return_value=(command, None)):
            row = r.run_one(self.read, 'codex', False, Path(self.temp.name), 1)
        self.assertIn('TimeoutExpired', row['error'])
        self.assertLess(row['seconds'], 3)

    def test_resume_recovers_saved_result_and_refuses_failed_trial(self):
        output = Path(self.temp.name) / 'results'
        trial = output / '001-read-codex-native'
        trial.mkdir(parents=True)
        row = dict(id='read', kind='search', client='codex', oko=False, complete=True)
        r.save(trial / 'result.json', row)
        plan = [(self.read, 'codex', False, 1), (self.edit, 'codex', False, 1)]
        recovered = r.load_completed_runs(output, plan)
        self.assertEqual(len(recovered), 1)
        self.assertEqual(recovered[0]['repeat'], 1)
        row['error'] = 'failed model session'
        r.save(trial / 'result.json', row)
        with self.assertRaisesRegex(RuntimeError, 'Refusing to retry'):
            r.load_completed_runs(output, plan)


if __name__ == '__main__':
    unittest.main()
