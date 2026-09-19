#!/usr/bin/env python3
"""Three-turn Documenso cache comparison. Plan-only unless --execute is given.

Each condition resumes one coding-agent conversation while a private local bridge
keeps the same Oko process alive. No model calls are made when printing the plan.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

from runner import engine as r

SPEC = importlib.util.spec_from_file_location("cache_profiler", r.ROOT.parent / "profile-cache.py")
profiler = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(profiler)
SCRIPT = Path(__file__).resolve()
BY_ID = {task['id']: task for task in r.TASKS}
PHASES = (('cold', 'pdf-page-count'), ('warm', 'next-recipient'),
          ('after-edit', 'document-search-delay'))


def proxy(path):
    """A CLI-owned stdio transport; disconnecting does not stop Oko."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.connect(path)
        def receive():
            with connection.makefile('rb') as stream:
                for line in stream:
                    sys.stdout.buffer.write(line)
                    sys.stdout.buffer.flush()
        reader = threading.Thread(target=receive, daemon=True)
        reader.start()
        for line in sys.stdin.buffer:
            connection.sendall(line)
        connection.shutdown(socket.SHUT_WR)
        reader.join(timeout=5)


class Bridge:
    def __init__(self, work, trial, offline=False):
        self.work, self.trial = work, trial
        self.phase = None
        self.calls = []
        self.lock = threading.Lock()
        self.temp = tempfile.TemporaryDirectory(prefix='oko-bridge-')
        self.path = str(Path(self.temp.name) / 'mcp.sock')
        self.backend = None
        self.server = None
        self.thread = None
        try:
            command = None if offline else [sys.executable, str(r.ROOT / 'oko-server.py'),
                                           str(work), str(trial / 'cache')]
            self.backend = profiler.Client(Path(r.SETTINGS['oko']), work, trial / 'cache',
                                           120, command=command, live=not offline,
                                           api_key=os.environ.get('TYPESAFE_API_KEY') if not offline else None)
            self.initialized = self.backend.request('initialize', {
                'protocolVersion': '2024-11-05', 'capabilities': {},
                'clientInfo': {'name': 'oko-cache-session-benchmark', 'version': '1'}})
            self.backend.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
            bridge = self
            class Handler(socketserver.StreamRequestHandler):
                def handle(self):
                    for line in self.rfile:
                        request = json.loads(line)
                        if 'id' not in request:
                            continue
                        response = bridge.dispatch(request)
                        try:
                            self.wfile.write((json.dumps(response) + '\n').encode())
                            self.wfile.flush()
                        except (BrokenPipeError, ConnectionResetError):
                            return
            class Server(socketserver.ThreadingUnixStreamServer):
                daemon_threads = True
            self.server = Server(self.path, Handler)
            os.chmod(self.path, 0o600)
            self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
            self.thread.start()
        except BaseException:
            self.close()
            raise

    def dispatch(self, request):
        with self.lock:
            try:
                method = request['method']
                if method == 'initialize':
                    result = self.initialized
                elif method == 'ping':
                    result = {}
                elif method in ('tools/list', 'tools/call'):
                    params = request.get('params', {})
                    target = self.work / BY_ID['document-search-delay']['path']
                    before_hash = r.digest(target) if target.is_file() else None
                    result = self.backend.request(method, params)
                    if method == 'tools/call':
                        packet = result.get('structuredContent', {})
                        call = dict(phase=self.phase, serverPid=self.backend.process.pid,
                                    tool=params.get('name'), arguments=params.get('arguments'),
                                    targetSha256BeforeCall=before_hash,
                                    isError=result.get('isError', False),
                                    timings=packet.get('timings'), retrieval=packet.get('retrieval'),
                                    evidence=packet.get('results', []))
                        self.calls.append(call)
                        r.save(self.trial / 'oko-calls.json', self.calls)
                else:
                    return {'jsonrpc': '2.0', 'id': request['id'],
                            'error': {'code': -32601, 'message': 'Method not found'}}
                return {'jsonrpc': '2.0', 'id': request['id'], 'result': result}
            except Exception:
                return {'jsonrpc': '2.0', 'id': request['id'],
                        'error': {'code': -32603, 'message': 'Benchmark MCP bridge failed'}}

    def close(self):
        if self.server:
            if self.thread and self.thread.is_alive():
                self.server.shutdown()
            self.server.server_close()
        if self.backend:
            self.backend.close()
        if self.thread:
            self.thread.join(timeout=5)
        self.temp.cleanup()


def session_id(client, events):
    ids = set()
    for event in events:
        value = (event.get('thread_id') if client == 'codex' and event.get('type') == 'thread.started'
                 else event.get('sessionID') if client == 'opencode'
                 else event.get('session_id') if client == 'claude' else None)
        if value:
            ids.add(value)
    if len(ids) != 1:
        raise RuntimeError(f'Expected exactly one {client} session ID; got {len(ids)}')
    return ids.pop()


def commands(task, client, enabled, work, trial, bridge_path=None, continuation=None):
    args, env = r.args_for(task, client, enabled, work, trial)
    prompt = args.pop()
    if task['kind'] == 'edit':
        prompt += ('\nAfter saving the edit, make another Oko search for the changed behavior '
                   'and verify the returned source contains the new value.' if enabled else
                   '\nAfter saving the edit, use native tools to reread and verify the changed value.')
    proxy_command = [sys.executable, str(SCRIPT), '--proxy', bridge_path] if enabled else None
    if client == 'codex':
        args.remove('--ephemeral')
        if continuation:
            args.insert(2, 'resume')
            # Resume uses the working directory and sandbox config rather than
            # exec's --cd/--sandbox flags. Never select a session via --last.
            for flag in ('--cd', '--sandbox'):
                index = args.index(flag)
                value = args[index + 1]
                del args[index:index + 2]
                if flag == '--sandbox':
                    args += ['-c', 'sandbox_mode=' + json.dumps(value)]
            args.append(continuation)
        if enabled:
            for index, argument in enumerate(args):
                if argument.startswith('mcp_servers.oko.command='):
                    args[index] = 'mcp_servers.oko.command=' + json.dumps(proxy_command[0])
                elif argument.startswith('mcp_servers.oko.args='):
                    args[index] = 'mcp_servers.oko.args=' + json.dumps(proxy_command[1:])
    elif client == 'opencode':
        if continuation:
            args += ['--session', continuation]
        if enabled:
            config = json.loads(env['OPENCODE_CONFIG_CONTENT'])
            config['mcp']['oko']['command'] = proxy_command
            env['OPENCODE_CONFIG_CONTENT'] = json.dumps(config)
    else:
        args.remove('--no-session-persistence')
        if continuation:
            args += ['--resume', continuation]
        if enabled:
            index = args.index('--mcp-config') + 1
            config = json.loads(args[index])
            config['mcpServers']['oko'].update(command=proxy_command[0], args=proxy_command[1:])
            args[index] = json.dumps(config)
    return args + [prompt], env


def check_cache_calls(phase, calls, edited_hash):
    if not calls or any(call['isError'] or not call['timings'] for call in calls):
        raise RuntimeError('Missing or failed Oko search; cache condition was not exercised')
    first_cache = calls[0]['timings']['cache']
    if phase == 'cold' and first_cache['status'] != 'cold':
        raise RuntimeError('First search did not use a cold cache')
    if phase == 'warm' and first_cache['status'] != 'memory':
        raise RuntimeError('Warm search did not reuse the retained workspace snapshot')
    if phase == 'after-edit':
        target = BY_ID['document-search-delay']
        verified = [call for call in calls if call['targetSha256BeforeCall'] == edited_hash
                    and any(item['path'] == target['path'] and target['new'] in item['text']
                            for item in call['evidence'])]
        if not verified:
            raise RuntimeError('No post-edit Oko search returned the updated source')


def restorable_native(session):
    turns = session['turns']
    return (not session['oko'] and 0 < len(turns) < len(PHASES)
            and bool(session.get('sessionId'))
            and [turn['phase'] for turn in turns] == [phase for phase, _ in PHASES[:len(turns)]]
            and all(turn.get('complete') and turn.get('exitCode') == 0
                    and not turn.get('providerErrors')
                    and not turn.get('grade', {}).get('unexpectedEdits') for turn in turns)
            and session.get('error') == 'RuntimeError: Grading failed; inspect exact patch and answer')


def run_condition(client, enabled, output, index, saved=None):
    trial = output / f'{index:02}-{client}-{"oko" if enabled else "native"}'
    trial.mkdir(exist_ok=saved is not None)
    work = trial / 'workspace'
    bridge = None
    row = copy.deepcopy(saved) if saved else dict(client=client, oko=enabled, turns=[], complete=False, artifact=str(trial))
    if saved:
        if not restorable_native(saved):
            raise RuntimeError('Cannot restore this incomplete condition without repeating work')
        row['priorErrors'] = [row.pop('error')]
    try:
        r.checkout(work)
        baseline = r.git(work, 'rev-parse', 'HEAD')
        edit = BY_ID['document-search-delay']
        original = (work / edit['path']).read_bytes()
        edited_hash = hashlib.sha256(original.replace(edit['old'].encode(), edit['new'].encode(), 1)).hexdigest()
        if enabled:
            bridge = Bridge(work, trial)
            row['okoPid'] = bridge.backend.process.pid
        conversation = row.get('sessionId')
        completed_turns = len(row['turns'])
        for phase_index, (phase, task_id) in enumerate(PHASES):
            if phase_index < completed_turns:
                continue
            task = BY_ID[task_id]
            phase_dir = trial / phase
            phase_dir.mkdir()
            if bridge:
                bridge.phase = phase
                call_offset = len(bridge.calls)
            args, env = commands(task, client, enabled, work, trial,
                                 bridge.path if bridge else None, conversation)
            started = time.monotonic()
            with (phase_dir / 'events.jsonl').open('w') as stdout, (phase_dir / 'stderr.txt').open('w') as stderr:
                proc = subprocess.Popen(args, cwd=work, env=env, stdout=stdout,
                                        stderr=stderr, start_new_session=True)
                try:
                    proc.wait(timeout=r.SETTINGS['timeoutSeconds'])
                except BaseException:
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    proc.wait()
                    raise
            elapsed = time.monotonic() - started
            events = []
            for line in (phase_dir / 'events.jsonl').read_text().splitlines():
                try:
                    events.append(json.loads(line))
                except ValueError:
                    continue
            turn = dict(phase=phase, id=task_id, seconds=elapsed, exitCode=proc.returncode,
                        **r.parse_events(client, events))
            row['turns'].append(turn)
            if proc.returncode or not turn['complete'] or turn['providerErrors']:
                raise RuntimeError(f'{phase}: client failed; inspect phase logs')
            actual_id = session_id(client, events)
            if conversation and actual_id != conversation:
                raise RuntimeError('CLI did not resume the same conversation')
            conversation = actual_id
            row['sessionId'] = conversation
            if bool(turn['okoCalls']) != enabled:
                raise RuntimeError('Agent tool usage did not match Oko condition')
            turn['grade'] = r.grade(task, work, turn['final'])
            if r.git(work, 'rev-parse', 'HEAD') != baseline:
                raise RuntimeError('Agent changed baseline commit')
            if turn['grade'].get('unexpectedEdits'):
                raise RuntimeError('Grading failed; inspect exact patch and answer')
            if bridge:
                turn['okoSearches'] = copy.deepcopy(bridge.calls[call_offset:])
                try:
                    check_cache_calls(phase, turn['okoSearches'], edited_hash)
                except RuntimeError as error:
                    # A completed turn with missing retrieval evidence is a
                    # measured failure, not a reason to discard other clients.
                    turn['cacheCheckError'] = str(error)
            r.save(trial / 'result.json', row)
            print(f'{client} {"oko" if enabled else "native"} {phase}: {elapsed:.2f}s', flush=True)
        row['complete'] = True
    except BaseException as error:
        row['error'] = f'{type(error).__name__}: {error}'
    finally:
        if bridge:
            bridge.close()
        if (work / '.git').exists():
            (trial / 'changes.patch').write_bytes(r.git(work, 'diff', '--binary', 'HEAD'))
        r.save(trial / 'result.json', row)
        if work.exists():
            try:
                shutil.rmtree(work)
            except OSError as error:
                row['cleanupWarning'] = str(error)
                r.save(trial / 'result.json', row)
    return row


def save_report(output, report):
    r.save(output / 'report.json', report)
    lines = ['# Documenso cache-session comparison', '',
             f"Complete: {report['complete']}; failed checks: {report.get('failedChecks', 'pending')}", '',
             '| Client | Oko | Phase | Seconds | Reported agent tokens | Cache ms | Jev ranking ms | Correct | Cache check |',
             '|---|---|---|---:|---:|---:|---:|---|---|']
    for session in report['sessions']:
        for turn in session['turns']:
            calls = turn.get('okoSearches', [])
            cache_ms = sum(call['timings']['cache']['totalMs'] for call in calls) if calls else '—'
            jev_ms = sum((call.get('retrieval') or {}).get('rerankMs', 0) for call in calls) if calls else '—'
            if any((call.get('arguments') or {}).get('deep') for call in calls):
                jev_ms = f'{jev_ms} + unmeasured deep ranking'
            grade = turn.get('grade', {})
            correct = grade.get('expectedPatchMatch') if turn['phase'] == 'after-edit' else grade.get('correctTopFive')
            tokens = (turn.get('tokens') or {}).get('total', '—')
            failed_cache = turn.get('cacheCheckError') or (turn['phase'] == 'after-edit' and
                session.get('error') == 'RuntimeError: No post-edit Oko search returned the updated source')
            cache_check = 'FAIL' if failed_cache else 'pass' if calls else '—'
            lines.append(f"| {session['client']} | {session['oko']} | {turn['phase']} | "
                         f"{turn['seconds']:.2f} | {tokens} | {cache_ms} | {jev_ms} | {correct} | {cache_check} |")
    lines += ['', report['method'], '', *('- ' + text for text in report['caveats'])]
    if report.get('runNotes'):
        lines += ['', 'Run notes:', '', *('- ' + text for text in report['runNotes'])]
    (output / 'report.md').write_text('\n'.join(lines) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--clients', default='codex,opencode,claude')
    parser.add_argument('--resume', type=Path, help='Continue saved conditions without repeating model calls')
    args = parser.parse_args()
    clients = args.clients.split(',')
    if len(set(clients)) != len(clients) or any(c not in ('codex', 'opencode', 'claude') for c in clients):
        parser.error('clients must be a unique comma-separated selection')
    modes = [(client, enabled) for index, client in enumerate(clients)
             for enabled in ((False, True) if index % 2 == 0 else (True, False))]
    print(f'{len(modes)} conversations, {len(modes) * 3} turns. Execution={args.execute}', flush=True)
    for phase, task_id in PHASES:
        print(f'{phase}: {BY_ID[task_id]["question"]}')
    if not args.execute:
        return
    r.SETTINGS = json.loads((r.STATE / 'settings.json').read_text())
    expected_models = dict(codex='gpt-5.6-sol', opencode='openai/gpt-5.6-sol', claude='claude-sonnet-5')
    if r.SETTINGS.get('effort') != 'low' or any(r.SETTINGS['models'][c] != expected_models[c] for c in clients):
        raise RuntimeError('Prepare the documented Sol/Sonnet low-effort configuration first')
    for path, expected in [(r.STATE / 'baseline.tar', r.SETTINGS['archiveSha256']),
                           (r.ROOT / 'tasks.json', r.SETTINGS['tasksSha256']),
                           (Path(r.SETTINGS['oko']), r.SETTINGS['okoSha256'])]:
        if r.digest(path) != expected:
            raise RuntimeError(f'Frozen artifact changed: {path}; prepare again')
    before = r.source_state()
    if before != {'commit': r.SETTINGS['commit'], 'status': ''}:
        raise RuntimeError('Original checkout changed; no reset performed')
    for client in clients:
        version = subprocess.check_output([r.SETTINGS['clients'][client], '--version'], text=True).strip()
        if version != r.SETTINGS['versions'][client]:
            raise RuntimeError(f'{client} version changed; prepare again')
    output = args.resume.resolve() if args.resume else Path(tempfile.mkdtemp(prefix='cache-session-', dir=r.STATE))
    if not output.is_relative_to(r.STATE.resolve()):
        raise RuntimeError('Resume must be inside this benchmark artifact directory')
    report = dict(settings=r.SETTINGS, isolation=r.ISOLATION_VERSION, complete=False, sessions=[], phases=PHASES,
                  method='Six independent conversations, each resumed across three CLI invocations. '
                  'One persistent Oko process per enabled conversation; no prewarming. '
                  'Blank-slate-v1 excludes personal skills/instructions and repository agent configuration. '
                  'Phase timings include CLI startup but exclude checkout and Oko bridge initialization.',
                  caveats=['One sample per phase/condition; not a statistical speed claim. Compare Oko/native '
                           'within each phase; cold and warm questions have different difficulty.',
                           'Conversation and provider prompt caches also affect warm turns.',
                           'Token usage is reported as emitted by each CLI; resume counters may differ in scope. '
                           'Do not sum cumulative counters or compare token definitions across clients.',
                           'Periodic verification can reread contents while retaining prepared data.',
                           'Agent conversations persist in local CLI storage to allow explicit resume.'])
    if args.resume:
        previous = json.loads((output / 'report.json').read_text())
        if previous.get('isolation') != r.ISOLATION_VERSION:
            raise RuntimeError('Cannot resume a conversation from a different isolation policy; start a new run')
        if previous['settings'] != r.SETTINGS or previous['phases'] != [list(phase) for phase in PHASES]:
            raise RuntimeError('Resume settings or phases differ')
        for index, session in enumerate(previous['sessions']):
            if index >= len(modes) or (session['client'], session['oko']) != modes[index]:
                raise RuntimeError('Resume client/condition order differs')
            # Never automatically retry a failed model call. The initial runner
            # stopped on missing post-edit evidence after all turns had ended;
            # retain that failure and move on to the next independent condition.
            finished = ([turn['phase'] for turn in session['turns']] == [phase for phase, _ in PHASES]
                        and all(turn.get('complete') and turn.get('exitCode') == 0 for turn in session['turns']))
            if not session['complete'] and not (finished or restorable_native(session)):
                raise RuntimeError('Incomplete client execution cannot be retried automatically')
        report['sessions'] = previous['sessions']
        print(f"Continuing after {len(report['sessions'])} saved conditions; no calls repeated.", flush=True)
    print(f'Report: {output / "report.json"}', flush=True)
    save_report(output, report)
    try:
        for index, (client, enabled) in enumerate(modes, 1):
            if index <= len(report['sessions']):
                saved = report['sessions'][index - 1]
                if not restorable_native(saved):
                    continue
                row = run_condition(client, enabled, output, index, saved=saved)
                report['sessions'][index - 1] = row
            else:
                row = run_condition(client, enabled, output, index)
                report['sessions'].append(row)
            save_report(output, report)
            if not row['complete']:
                raise RuntimeError(f'Stopped after failed condition: {row.get("error")}')
        report['complete'] = True
    finally:
        report['failedChecks'] = sum(bool(session.get('error')) +
                                    sum(bool(turn.get('cacheCheckError')) + bool(turn.get('grade', {}).get('gradingError'))
                                        for turn in session['turns'])
                                    for session in report['sessions'])
        report['sourceUnchanged'] = r.source_state() == before
        if not report['sourceUnchanged']:
            report['complete'] = False
        save_report(output, report)
    if not report['sourceUnchanged']:
        raise RuntimeError('Original checkout changed during benchmark')


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--proxy':
        proxy(sys.argv[2])
    else:
        main()
