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
        self.assertEqual(len(plan), 135)
        for task in r.TASKS:
            self.assertEqual({(c, o) for t, c, o, _ in plan if t['id'] == task['id']}, set(r.MODES))
        self.assertEqual(len(r.make_plan(r.TASKS, repeats=3)), 405)
        for flags, expected in [([], '15 tasks, 135 sessions'), (['--pilot'], '2 tasks, 18 sessions'),
                                (['--fast'], '5 tasks, 45 sessions'),
                                (['--fast', '--clients', 'codex,claude'], '5 tasks, 30 sessions'),
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
        self.assertEqual(len(plan), 45)
        for task in tasks:
            self.assertEqual({(c, o) for t, c, o, _ in plan if t['id'] == task['id']}, set(r.MODES))
        result = subprocess.run([sys.executable, str(r.ROOT / 'runner.py'), '--fast', '--pilot'], capture_output=True)
        self.assertNotEqual(result.returncode, 0)

    def test_commands_keep_conditions_and_edit_permissions_separate(self):
        for task in r.TASKS:
            for client, condition in r.MODES:
                enabled = r.condition_enabled(condition)
                args, env = r.args_for(task, client, condition, self.work, Path(self.temp.name))
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
                        args, _ = r.args_for(task, client, 'oko-cold' if enabled else 'native', self.work, Path(self.temp.name))
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

    def test_text_tool_results_take_cache_state_from_the_metrics_file(self):
        text = 'auth.rs:1-1 (whole file)\n```\nfn authenticate() {}\n```\n'
        streams = {
            'codex': [dict(type='item.completed', item=dict(type='mcp_tool_call', server='oko', tool='search',
                                                          result=dict(content=[dict(type='text', text=text)])))],
            'opencode': [dict(type='tool_use', part=dict(tool='oko_search', state=dict(output=text)))],
            'claude': [dict(type='assistant', message=dict(content=[dict(type='tool_use', name='mcp__oko__search', id='one')])),
                       dict(type='user', message=dict(content=[dict(type='tool_result', tool_use_id='one', content=text)]))],
        }
        with tempfile.TemporaryDirectory() as directory:
            metrics = Path(directory) / 'oko-metrics.jsonl'
            metrics.write_text(json.dumps({'ranking': 'jev', 'timings': {'totalMs': 9, 'cache': {
                'status': 'disk', 'rebuiltFiles': 0, 'reusedFiles': 7}}}) + '\n')
            for client, events in streams.items():
                row = r.parse_events(client, events)
                self.assertEqual(r.cache_observations(row), [], client)
                self.assertFalse(r.check_condition('oko-warm', [], row['okoCalls']))
                self.assertEqual(r.observability.attach_oko_metrics(row['tools'], metrics), 1)
                observations = r.cache_observations(row)
                self.assertEqual([o['status'] for o in observations], ['disk'], client)
                self.assertTrue(r.check_condition('oko-warm', observations, row['okoCalls']))
                self.assertFalse(r.check_condition('oko-cold', observations, row['okoCalls']))

    def test_startup_preparation_decides_the_session_cache_state(self):
        events = [dict(type='tool_use', part=dict(tool='oko_search', state=dict(output='auth.rs:1-1 (whole file)'))),
                  dict(type='tool_use', part=dict(tool='oko_search', state=dict(output='auth.rs:1-1 (whole file)')))]
        memory = {'timings': {'totalMs': 1, 'cache': {'status': 'memory', 'rebuiltFiles': 0, 'reusedFiles': 7}}}
        for condition, other, prepared in (
                ('oko-cold', 'oko-warm', {'status': 'cold', 'rebuiltFiles': 7, 'reusedFiles': 0}),
                ('oko-warm', 'oko-cold', {'status': 'disk', 'rebuiltFiles': 0, 'reusedFiles': 7})):
            with tempfile.TemporaryDirectory() as directory:
                metrics = Path(directory) / 'oko-metrics.jsonl'
                lines = [{'event': 'prewarm', 'cache': prepared}, memory, memory]
                metrics.write_text(''.join(json.dumps(line) + '\n' for line in lines))
                row = r.parse_events('opencode', events)
                self.assertEqual(r.observability.attach_oko_metrics(row['tools'], metrics), 2)
            self.assertNotIn('okoPrewarm', row['tools'][1])
            self.assertEqual([len(tool['okoMetrics']) for tool in row['tools']], [1, 1])
            observations = r.cache_observations(row)
            self.assertEqual([o['status'] for o in observations], [prepared['status'], 'memory'])
            self.assertTrue(r.check_condition(condition, observations, row['okoCalls']))
            self.assertFalse(r.check_condition(other, observations, row['okoCalls']))
            record = r.observability.make_record(
                run_id='test', record_id='prewarm', task_id='task', client='opencode', client_version='v',
                model='m', effort='low', enabled=True, task={'cacheCondition': condition.removeprefix('oko-')},
                row={**row, 'durationNs': 10, 'grade': {'passed': True}}, total_wall_ns=10)
            self.assertIn({'cache': prepared}, record['okoUsage']['phaseMetrics'])

    def test_three_event_formats_and_errors(self):
        streams = {
            'codex': [dict(type='item.completed', item=dict(type='mcp_tool_call', server='oko')),
                      dict(type='item.completed', item=dict(type='agent_message', text=self.answer)),
                      dict(type='turn.completed', usage=dict(input_tokens=10, cached_input_tokens=4, output_tokens=2))],
            'opencode': [dict(type='tool_use', part=dict(tool='oko_search', state=dict(output=json.dumps({'timings': {'cache': {'status': 'cold'}}, 'providerCalls': []})))),
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
        opencode = r.parse_events('opencode', streams['opencode'])
        self.assertEqual(opencode['tools'][0]['result'][0]['timings']['cache']['status'], 'cold')
        self.assertTrue(r.parse_events('claude', [dict(type='result', is_error=True)])['providerErrors'])
        self.assertTrue(r.parse_events('codex', [dict(type='turn.failed')])['providerErrors'])
        self.assertTrue(r.parse_events('opencode', [dict(type='error')])['providerErrors'])

    def test_opencode_step_tokens_are_preserved_across_record_and_summary(self):
        events = [
            dict(type='step_finish', part=dict(
                reason='tool-calls', messageID='first',
                tokens=dict(input=10, output=3, reasoning=2, cache=dict(read=4, write=1)),
            )),
            dict(type='step_finish', part=dict(
                reason='stop', messageID='last',
                tokens=dict(input=7, output=5, reasoning=1, cache=dict(read=6, write=2)),
            )),
        ]
        parsed = r.parse_events('opencode', events)
        record = r.observability.make_record(
            run_id='test', record_id='multi-step', task_id='read', client='opencode',
            client_version='test', model='test', effort='medium', enabled=False,
            task=self.read, row=parsed, total_wall_ns=1,
        )
        self.assertEqual(len(record['agentUsageSteps']), 2)
        self.assertEqual(record['agentUsage']['inputTokens'], 17)
        self.assertEqual(record['agentUsage']['outputTokens'], 8)
        self.assertEqual(record['agentUsage']['cacheReadTokens'], 10)
        self.assertEqual(record['agentUsage']['cacheWriteTokens'], 3)
        self.assertEqual(record['agentUsage']['reasoningTokens'], 3)
        self.assertIsNone(record['agentUsage']['totalTokens'])
        summary = r.observability.aggregate([record], run_id='test')
        self.assertEqual(summary['groups'][0]['agentUsage']['inputTokens'], 17)
        self.assertEqual(summary['groups'][0]['agentUsage']['outputTokens'], 8)
        self.assertEqual(summary['groups'][0]['agentUsage']['cacheReadTokens'], 10)
        self.assertEqual(summary['groups'][0]['agentUsage']['cacheWriteTokens'], 3)
        self.assertEqual(summary['groups'][0]['agentUsage']['reasoningTokens'], 3)
        self.assertIsNone(summary['groups'][0]['agentUsage']['totalTokens'])

    def test_claude_tool_results_reach_shareable_measurements(self):
        call = dict(phase='normal', durationNs=123, requestBytes=40, responseBytes=80,
                    httpStatus=200, success=True, errorClass=None, usage={'totalTokens': 7})
        payload = json.dumps({'stats': {'providerCalls': [call]},
                              'sourceBody': 'PRIVATE_SOURCE', 'prompt': 'PRIVATE_PROMPT'})
        for content in (payload, [{'type': 'text', 'text': payload},
                                  {'type': 'text', 'text': 'not JSON'}]):
            with self.subTest(content=content):
                events = [
                    dict(type='assistant', message={'content': [
                        dict(type='tool_use', id='oko-1', name='mcp__oko__search', input={}),
                        dict(type='tool_use', id='native-1', name='Read', input={}),
                    ]}),
                    dict(type='user', message={'content': [
                        dict(type='tool_result', tool_use_id='native-1', content=payload),
                        dict(type='tool_result', tool_use_id='oko-1', content=content),
                        dict(type='tool_result', tool_use_id='unmatched', content=payload),
                    ]}),
                    dict(type='result', is_error=False, result=self.answer),
                ]
                original = json.dumps(events)
                row = r.parse_events('claude', events)
                self.assertEqual(json.dumps(events), original)
                self.assertEqual(row['toolCalls'], 2)
                self.assertEqual(row['okoCalls'], 1)
                record = r.observability.make_record(
                    run_id='test', record_id='1', task_id='read', client='claude',
                    client_version='test', model='test', effort='medium', enabled=True, row=row,
                )
                self.assertEqual(record['calls']['jevCalls'], 1)
                self.assertEqual(record['okoUsage']['providerCalls'][0]['durationNs'], 123)
                self.assertEqual(record['okoUsage']['tokenUsage'][0]['totalTokens'], 7)
                self.assertNotIn('PRIVATE_', json.dumps(record))

        row = r.parse_events('claude', [
            dict(type='assistant', message={'content': [
                dict(type='tool_use', id='oko-1', name='mcp__oko__search', input={}),
            ]}),
            dict(type='user', message={'content': [
                dict(type='tool_result', tool_use_id='oko-1', content='tool failed', is_error=True),
            ]}),
        ])
        self.assertEqual(row['tools'][0]['result'], [])

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
            row('model-a', 'low', 'disk'),
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
                ('codex', 'model-a', 'low', 'oko-cold', 'disk'),
            },
        )

    def test_cache_conditions_require_observed_disk_or_cold(self):
        self.assertTrue(r.check_condition('native', []))
        self.assertFalse(r.check_condition('native', [], oko_calls=1))
        self.assertTrue(r.check_condition('oko-cold', [{'status': 'cold'}]))
        self.assertFalse(r.check_condition('oko-cold', [{'status': 'disk'}]))
        self.assertTrue(r.check_condition('oko-warm', [{'status': 'disk', 'rebuiltFiles': 0, 'reusedFiles': 4}]))
        self.assertFalse(r.check_condition('oko-warm', [{'status': 'memory', 'rebuiltFiles': 0, 'reusedFiles': 4}]))

    def test_pilot_canary_waits_for_all_sessions_and_updates_every_report(self):
        for resume in (False, True):
            for failed in (False, True):
                with self.subTest(resume=resume, failed=failed):
                    state = Path(self.temp.name).resolve() / f'pilot-{resume}-{failed}'
                    state.mkdir()
                    settings = dict(r.SETTINGS, commit='a' * 40, versions={'codex': 'test'},
                                    archiveSha256='b' * 64, tasksSha256='b' * 64,
                                    okoSha256='b' * 64, oko='unused', effort='medium')
                    r.save(state / 'settings.json', settings)

                    def row(task, complete=True):
                        result = dict(id=task['id'], kind=task['kind'], client='codex',
                                      condition='native', observedCacheState='native', oko=False,
                                      complete=complete, providerErrors=[] if complete else ['failed'],
                                      seconds=1, durationNs=1000000000, tools=[],
                                      okoCalls=0, toolCalls=0, grade={})
                        if not complete:
                            result['error'] = 'client failed'
                        return result

                    argv = ['runner.py', '--pilot', '--execute', '--clients', 'codex',
                            '--condition', 'native']
                    if resume:
                        output = state / 'results-resume'
                        output.mkdir()
                        r.save(output / 'report.json', {
                            'settings': settings, 'plannedSessions': 2,
                            'isolation': r.ISOLATION_VERSION,
                            'canary': {'status': 'passed', 'plannedSessions': 1},
                        })
                        argv += ['--resume', str(output)]

                    def run(task, client, condition, output, index):
                        saved = json.loads((output / 'report.json').read_text())
                        bundle = json.loads((output / 'shareable/benchmark.json').read_text())
                        self.assertIsNone(saved['canary'])
                        self.assertIsNone(bundle['canary'])
                        self.assertNotIn('Canary: passed', (output / 'report.md').read_text())
                        return row(task, complete=not (failed and index == 2))

                    with patch.object(r, 'STATE', state), \
                         patch.object(sys, 'argv', argv), \
                         patch.object(r, 'select_tasks', return_value=[self.read, self.edit]), \
                         patch.object(r, 'digest', return_value='b' * 64), \
                         patch.object(r, 'source_state', return_value={'commit': 'a' * 40, 'status': ''}), \
                         patch.object(r.subprocess, 'check_output', return_value='test'), \
                         patch.object(r, 'load_completed_runs', return_value=[row(self.read)]), \
                         patch.object(r, 'run_one', side_effect=run), \
                         patch('builtins.print'):
                        if failed:
                            with self.assertRaises(SystemExit):
                                r.main()
                        else:
                            r.main()
                    output = next(state.glob('results-*'))
                    saved = json.loads((output / 'report.json').read_text())
                    bundle = json.loads((output / 'shareable/benchmark.json').read_text())
                    self.assertEqual(saved['canary'], bundle['canary'])
                    self.assertEqual(saved['canary']['plannedSessions'], 2)
                    self.assertEqual(saved['canary']['status'], 'failed' if failed else 'passed')
                    self.assertIn('Canary: ' + saved['canary']['status'],
                                  (output / 'report.md').read_text())

    def test_canary_failure_is_explicit(self):
        row = dict(
            id='read', kind='search', taskKind='search', client='codex', oko=False,
            condition='native', complete=False, providerErrors=[{'type': 'error'}],
            error='client failed', durationNs=1, seconds=0.000000001, tokens=None,
            tools=[], okoCalls=0, toolCalls=0, final='', grade={},
        )
        result = r.canary_result([row], [self.read], ['codex'], {'codex': 'test'}, 'canary')
        self.assertEqual(result['status'], 'failed')
        self.assertFalse(result['checks']['completedClientRuns'])

    def test_canary_requires_provider_evidence_for_non_native_rows(self):
        row = dict(
            id='read', kind='search', taskKind='search', client='codex', oko=True,
            condition='oko-cold', complete=True, providerErrors=[],
            durationNs=1, seconds=0.000000001, tokens=None, tools=[],
            cacheObservations=[{'status': 'cold'}], okoCalls=1, toolCalls=1,
            final=self.answer, grade={},
        )
        result = r.canary_result([row], [self.read], ['codex'], {'codex': 'test'}, 'canary')
        self.assertEqual(result['status'], 'failed')
        self.assertFalse(result['checks']['okoMetrics'])

    def test_failed_provider_call_counts_as_instrumentation_evidence(self):
        failed_call = dict(
            phase='normal', durationNs=123, requestBytes=40, responseBytes=0,
            httpStatus=503, success=False, errorClass='provider_error', usage=None,
        )
        row = dict(
            id='read', kind='search', taskKind='search', client='codex', oko=True,
            condition='oko-cold', complete=True, providerErrors=[],
            durationNs=1, seconds=0.000000001, tokens=None,
            tools=[{'server': 'oko', 'result': {
                'timings': {'totalMs': 1, 'cache': {'status': 'cold'}},
                'providerCalls': [failed_call],
            }}],
            cacheObservations=[{'status': 'cold'}], okoCalls=1, toolCalls=1,
            final=self.answer, grade={},
        )
        result = r.canary_result([row], [self.read], ['codex'], {'codex': 'test'}, 'canary')
        self.assertEqual(result['status'], 'passed')
        self.assertTrue(result['checks']['okoMetrics'])

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
            row = r.run_one(self.read, 'codex', 'oko-cold', Path(self.temp.name), 2)
            self.assertIn('Observed Oko cache state', row['error'])

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
        row = dict(id='read', kind='search', taskKind='search', client='codex', oko=False, condition='native', complete=True)
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
