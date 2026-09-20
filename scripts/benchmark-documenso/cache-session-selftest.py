#!/usr/bin/env python3
"""Offline checks for persistent MCP reuse and explicit CLI session continuation."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

spec = importlib.util.spec_from_file_location('cache_session', Path(__file__).with_name('cache-session.py'))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


class CacheSessionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='cache-session-test-')
        self.trial = Path(self.temp.name)
        self.work = self.trial / 'workspace'
        self.work.mkdir()
        c.r.SETTINGS = dict(oko=str(c.r.ROOT.parents[1] / 'target/release/oko'),
                            clients={name: name for name in ('codex', 'opencode', 'claude')},
                            models=dict(codex='gpt-5.6-sol', opencode='openai/gpt-5.6-sol', claude='claude-sonnet-5'),
                            effort='low')

    def tearDown(self):
        self.temp.cleanup()

    def test_plan_is_six_conversations_without_execution(self):
        output = subprocess.check_output([sys.executable, str(c.SCRIPT)], text=True)
        self.assertIn('6 conversations, 18 turns. Execution=False', output)
        self.assertIn('cold:', output)
        self.assertIn('warm:', output)
        self.assertIn('after-edit:', output)

    def test_explicit_resume_and_private_proxy_for_all_clients(self):
        for client in ('codex', 'opencode', 'claude'):
            for enabled in (False, True):
                for phase, task_id in c.PHASES:
                    continuation = None if phase == 'cold' else 'exact-session-id'
                    args, env = c.commands(c.BY_ID[task_id], client, enabled, self.work,
                                           self.trial, '/tmp/example.sock', continuation)
                    self.assertNotIn('--ephemeral', args)
                    self.assertNotIn('--no-session-persistence', args)
                    self.assertNotIn('--last', args)
                    self.assertEqual('exact-session-id' in args, continuation is not None)
                    config = ' '.join(args) + env.get('OPENCODE_CONFIG_CONTENT', '')
                    self.assertEqual('--proxy' in config, enabled)
                    self.assertNotIn('TYPESAFE_API_KEY', env)
                    if client == 'codex':
                        self.assertIn('skip_host_skill_discovery', args)
                        self.assertIn('project_doc_max_bytes=0', args)
                    if client == 'codex' and continuation:
                        self.assertEqual(args[1:3], ['exec', 'resume'])
                        self.assertNotIn('--cd', args)
                        self.assertNotIn('--sandbox', args)
                        sandbox = 'workspace-write' if phase == 'after-edit' else 'read-only'
                        self.assertIn('sandbox_mode=' + json.dumps(sandbox), args)
                    if phase == 'after-edit' and enabled:
                        self.assertIn('After saving the edit, make another Oko search', args[-1])

    def test_session_ids_are_not_guessed_or_silently_changed(self):
        examples = {'codex': {'type': 'thread.started', 'thread_id': 'abc'},
                    'opencode': {'type': 'text', 'sessionID': 'abc'},
                    'claude': {'type': 'result', 'session_id': 'abc'}}
        for client, event in examples.items():
            self.assertEqual(c.session_id(client, [event, event]), 'abc')
            with self.assertRaises(RuntimeError):
                c.session_id(client, [])

    def test_unexercised_cache_and_missing_post_edit_evidence_fail(self):
        with self.assertRaises(RuntimeError):
            c.check_cache_calls('warm', [], 'hash')
        call = dict(isError=False, timings={'cache': {'status': 'cold'}},
                    targetSha256BeforeCall='old', evidence=[])
        with self.assertRaises(RuntimeError):
            c.check_cache_calls('warm', [call], 'new')
        with self.assertRaises(RuntimeError):
            c.check_cache_calls('after-edit', [call], 'new')

    def test_only_completed_native_read_turns_can_be_restored(self):
        session = dict(oko=False, sessionId='exact-id',
                       error='RuntimeError: Grading failed; inspect exact patch and answer',
                       turns=[dict(phase='cold', complete=True, exitCode=0,
                                   grade={'gradingError': 'invalid JSON'})])
        self.assertTrue(c.restorable_native(session))
        session['oko'] = True
        self.assertFalse(c.restorable_native(session))
        session['oko'] = False
        session['turns'][0]['grade']['unexpectedEdits'] = ['changed.ts']
        self.assertFalse(c.restorable_native(session))

    def test_reconnecting_transports_keep_one_backend_and_refresh_edits(self):
        target = self.work / c.BY_ID['document-search-delay']['path']
        target.parent.mkdir(parents=True)
        original = 'export function documentSearch() { return useDebouncedValue(searchTerm, 500); }\n'
        target.write_text(original)
        time.sleep(2.1)
        bridge = c.Bridge(self.work, self.trial, offline=True)
        pid = bridge.backend.process.pid
        try:
            for phase, _ in c.PHASES:
                bridge.phase = phase
                if phase == 'after-edit':
                    target.write_text(original.replace('500', '300'))
                transport = c.profiler.Client(Path(c.r.SETTINGS['oko']), self.work, self.trial / 'unused', 15,
                                             command=[sys.executable, str(c.SCRIPT), '--proxy', bridge.path])
                try:
                    transport.initialize()
                    response = transport.request('tools/call', {'name': 'search', 'arguments': {
                        'question': 'where is document search implemented?'}})
                    self.assertFalse(response.get('isError'))
                    self.assertNotIn('structuredContent', response)
                    packet = bridge.backend.last_metrics()
                    self.assertEqual(packet['ranking'], 'lexical')
                    if phase == 'warm':
                        self.assertEqual(packet['timings']['cache']['status'], 'memory')
                        self.assertEqual(packet['timings']['cache']['rebuiltFiles'], 0)
                    if phase == 'after-edit':
                        self.assertIn('300', packet['results'][0]['text'])
                        self.assertEqual(packet['timings']['cache']['rebuiltFiles'], 1)
                    c.check_cache_calls(phase, [bridge.calls[-1]], c.r.digest(target))
                finally:
                    transport.close()
                self.assertIsNone(bridge.backend.process.poll())
            self.assertEqual({call['serverPid'] for call in bridge.calls}, {pid})
            self.assertEqual(len(bridge.calls), 3)
        finally:
            bridge.close()


if __name__ == '__main__':
    unittest.main()
