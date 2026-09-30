#!/usr/bin/env python3
"""Replay the first Oko call of recorded agent sessions with the recorded Jev verdicts.

A recorded session holds the question an agent asked and the score Jev gave
every candidate. Replaying that question through another build, against a local
stand-in for Jev that returns the same scores, shows what that build would have
put in the agent's first answer. It is free, needs no network, and is
deterministic, so it measures changes to how Oko builds an answer (excerpt
choice, siblings, windows) exactly.

It is only exact while chunk boundaries and previews match the recorded build:
a candidate whose range the recording does not know gets a low default score.
The report counts those lookups; changes to chunking or ranking still need a
real Jev replay (replay.py --execute).

Usage:
  verdicts.py --report .../results-on3ibomb/report.json \
      --build v060=/path/to/oko --build new=/path/to/oko [--tasks a,b] [--call N]
"""
import argparse
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, str(Path(__file__).resolve().parent))
import replay  # noqa: E402

OUTPUT = replay.PROJECT / 'benchmarks/results/public-replay'
DEFAULT_SCORE = 0.02


def oko_arguments(tool):
    """The arguments an agent sent to Oko, as recorded by Codex, Claude or OpenCode."""
    name = str(tool.get('name') or tool.get('tool'))
    if not ('oko' in name or tool.get('server') == 'oko'):
        return None
    value = tool.get('arguments') or tool.get('input')
    if value is None:
        state = tool.get('state')
        if isinstance(state, str):
            try:
                state = json.loads(state)
            except ValueError:
                state = None
        value = (state or {}).get('input')
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except ValueError:
            return None
    if not isinstance(value, dict):
        return None
    arguments = {key: value[key] for key in ('question', 'questions', 'symbols', 'mode', 'intent')
                 if value.get(key) not in (None, '', [])}
    directory = replay.portable_directory(value.get('directory'))
    if directory:
        arguments['directory'] = directory
    return arguments or None


def recorded_calls(artifact):
    path = Path(artifact) / 'oko-metrics.jsonl'
    if not path.exists():
        return []
    lines = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    return [line for line in lines if line.get('event') != 'prewarm']


def sessions(reports, tasks, call):
    """(report, run, arguments, recorded call) for every Oko session of the chosen tasks."""
    for report in reports:
        for run in json.loads(Path(report).read_text())['runs']:
            if not run.get('oko') or run.get('id') not in tasks:
                continue
            calls = recorded_calls(run['artifact'])
            tools = [a for a in (oko_arguments(t) for t in run.get('tools') or []) if a]
            if len(calls) <= call or len(tools) <= call:
                continue
            yield report, run, tools[call], calls[call]


def verdicts(call, arguments):
    """Recorded scores as (path, start, end, score) lists, keyed by question.

    A several-question call records each question's candidates one after the
    other, each list best first, so a rise in score starts the next question's
    list. When that split does not give one list per question, every question
    gets the merged list (the stand-in then reports it as ambiguous).
    """
    rows = [(c['path'], c['startLine'], c['endLine'], c.get('score') or 0.0)
            for c in (call.get('retrieval') or {}).get('candidates') or []]
    questions = ([arguments['question']] if arguments.get('question') else []) + list(arguments.get('questions') or [])
    if len(questions) < 2:
        return {None: rows}
    runs = [[]]
    for row in rows:
        if runs[-1] and row[3] > runs[-1][-1][3]:
            runs.append([])
        runs[-1].append(row)
    if len(runs) != len(questions):
        return {None: rows}
    by_question = {question.strip(): run for question, run in zip(questions, runs)}
    by_question[None] = rows
    return by_question


class StandInJev:
    """A local HTTP server that answers Jev ranking requests with recorded scores."""

    def __init__(self):
        self.scores = {None: []}
        self.requests = []
        stand_in = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get('content-length', 0))))
                answers, rows = {}, []
                question = str(body['state'].get('question', '')).strip()
                scores = stand_in.scores.get(question)
                if scores is None:
                    scores = stand_in.scores[None]
                for index, candidate in enumerate(body['state']['candidates']):
                    score, match = stand_in.lookup(scores, candidate.get('source', ''))
                    answers[f'candidate_{index + 1}'] = {'type': 'noul', 'noul': score}
                    rows.append({'source': candidate.get('source'), 'textBytes': len(candidate.get('text', '')),
                                 'text': candidate.get('text', ''), 'score': score, 'match': match})
                stand_in.requests.append({'phase': self.headers.get('x-oko-phase', ''), 'candidates': rows})
                reply = json.dumps({'answers': answers, 'usage': {'input_tokens': 0, 'output_tokens': 0}}).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(reply)))
                self.end_headers()
                self.wfile.write(reply)

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f'http://127.0.0.1:{self.server.server_address[1]}'

    @staticmethod
    def lookup(scores, source):
        path, _, lines = source.rpartition(':')
        start, _, end = lines.partition('-')
        try:
            start, end = int(start), int(end)
        except ValueError:
            return DEFAULT_SCORE, 'unknown'
        for match, test in (('exact', lambda s, e: (s, e) == (start, end)),
                            ('end', lambda s, e: e == end), ('start', lambda s, e: s == start)):
            for p, s, e, score in scores:
                if p == path and test(s, e):
                    return score, match
        return DEFAULT_SCORE, 'default'

    def close(self):
        self.server.shutdown()


def environment(stand_in, extra):
    # Never the real service: a dummy key, the stand-in's address, and a dead
    # proxy for anything that would try another host.
    env = {'TYPESAFE_BASE_URL': stand_in.url, 'HTTPS_PROXY': 'http://127.0.0.1:9',
           'https_proxy': 'http://127.0.0.1:9', 'ALL_PROXY': 'http://127.0.0.1:9',
           'NO_PROXY': '127.0.0.1,localhost', 'no_proxy': '127.0.0.1,localhost'}
    env.update(extra or {})
    return env


def anchor_evidence(requests, expected):
    """For each expected location: the shortlist position and preview size of the
    first normal-request candidate that overlaps it, as the stand-in saw them,
    and whether any of the location's lines is in that preview: what Jev can
    see of it. Previews number their lines `N: text`."""
    found = []
    for path, start, end in expected:
        hit = None
        for request in requests:
            if request['phase'] != 'normal':
                continue
            for position, row in enumerate(request['candidates']):
                p, _, lines = (row['source'] or '').rpartition(':')
                s, _, e = lines.partition('-')
                if p == path and s.isdigit() and e.isdigit() and int(s) <= end and start <= int(e):
                    shown = {int(line.split(':', 1)[0]) for line in row['text'].split('\n')
                             if line.split(':', 1)[0].isdigit()}
                    hit = {'position': position, 'textBytes': row['textBytes'], 'score': row['score'],
                           'visible': any(start <= n <= end for n in shown)}
                    break
            if hit:
                break
        found.append(hit)
    return found


def replay_session(binary, workspace, cache, stand_in, arguments, call, expected, timeout, extra):
    """One fresh MCP process per session: session memory must not carry over."""
    stand_in.scores = verdicts(call, arguments)
    stand_in.requests = []
    client = replay.profiler().Client(Path(binary), Path(workspace), Path(cache), timeout, live=True,
                                      api_key='dummy-stand-in', model=replay.JEV_MODEL,
                                      extra_env=environment(stand_in, extra))
    try:
        client.initialize()
        response = client.request('tools/call', {'name': 'search', 'arguments': arguments})
        packet = replay.packet_of(client, response)
    finally:
        client.close()
    row = replay.score(packet, expected, arguments.get('directory'))
    shown = [(e['path'], e['startLine'], e['endLine']) for e in packet.get('results') or []]
    recorded = [(e['path'], e['startLine'], e['endLine']) for e in call.get('results') or []]
    lookups = [c['match'] for r in stand_in.requests for c in r['candidates']]
    normal = [c['textBytes'] for r in stand_in.requests if r['phase'] == 'normal' for c in r['candidates']]
    evidence = anchor_evidence(stand_in.requests, expected)
    row.update(shown=shown, recordedShown=recorded, reproduced=shown == recorded,
               textBytes=len(''.join(b.get('text', '') for b in response.get('content', []))),
               defaultLookups=sum(m in ('default', 'unknown') for m in lookups), lookups=len(lookups),
               previewBytesMean=(sum(normal) / len(normal)) if normal else None,
               anchors=evidence)
    return row


def summarize(rows, labels):
    out = {}
    for label in labels:
        mine = [r for r in rows if r['build'] == label and 'error' not in r]
        per_task = {}
        for row in mine:
            task = per_task.setdefault(row['task'], {'sessions': 0, 'full': 0, 'covered': 0, 'expected': 0,
                                                      'reproduced': 0, 'textBytes': 0})
            task['sessions'] += 1
            task['full'] += row['full']
            task['covered'] += row['covered']
            task['expected'] += row['expected']
            task['reproduced'] += row['reproduced']
            task['textBytes'] += row['textBytes']
        anchors = [a for r in mine for a in r.get('anchors') or []]
        seen = [a for a in anchors if a]
        out[label] = {'sessions': len(mine), 'errors': sum(r['build'] == label and 'error' in r for r in rows),
                      'anchors': len(anchors), 'anchorsJudged': len(seen),
                      'anchorsVisible': sum(a.get('visible', False) for a in seen),
                      'anchorPreviewBytesMedian': sorted(a['textBytes'] for a in seen)[len(seen) // 2] if seen else None,
                      'full': sum(r['full'] for r in mine),
                      'reproduced': sum(r['reproduced'] for r in mine),
                      'defaultLookups': sum(r['defaultLookups'] for r in mine),
                      'lookups': sum(r['lookups'] for r in mine),
                      'textBytes': sum(r['textBytes'] for r in mine),
                      'tasks': per_task}
    return out


def render(summary, labels, reports):
    lines = ['# First answers replayed with recorded Jev verdicts', '',
             'Reports: ' + ', '.join(str(r) for r in reports), '',
             '| Build | Sessions | First answer has every expected location | Same excerpts as recorded | '
             'Scores not in the recording | Answer bytes | Expected lines in the Jev preview | '
             'Median preview bytes of those candidates |',
             '|---|---:|---:|---:|---:|---:|---:|---:|']
    for label in labels:
        s = summary[label]
        lines.append(f"| {label} | {s['sessions']} | {s['full']} | {s['reproduced']} | "
                     f"{s['defaultLookups']} of {s['lookups']} | {s['textBytes']} | "
                     f"{s['anchorsVisible']} of {s['anchorsJudged']} | {s['anchorPreviewBytesMedian']} |")
    lines += ['', '"Same excerpts as recorded" compares with what the recorded build showed; it is the '
              'check that the stand-in reproduces the recording when the build is the recorded one.', '',
              '| Task | ' + ' | '.join(f'{label}: every location / sessions (anchors covered)' for label in labels) + ' |',
              '|---|' + '---:|' * len(labels)]
    tasks = dict.fromkeys(t for label in labels for t in summary[label]['tasks'])
    for task in tasks:
        cells = []
        for label in labels:
            t = summary[label]['tasks'].get(task)
            cells.append('–' if not t else f"{t['full']} / {t['sessions']} ({t['covered']}/{t['expected']})")
        lines.append(f'| {task} | ' + ' | '.join(cells) + ' |')
    return '\n'.join(lines) + '\n'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--report', action='append', type=Path, required=True,
                        help='Suite report.json whose Oko sessions to replay; repeatable')
    parser.add_argument('--build', action='append', required=True, metavar='LABEL=PATH[,KEY=VALUE...]',
                        help='Oko binary; repeat to compare. KEY=VALUE pairs set environment switches.')
    parser.add_argument('--tasks', help='Comma-separated task ids; default all')
    parser.add_argument('--call', type=int, default=0, help='Which Oko call of each session (0 = first)')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args(argv)

    builds, build_env = [], {}
    for item in args.build:
        label, _, rest = item.partition('=')
        path, *switches = rest.split(',')
        if not Path(path).is_file():
            parser.error(f'No binary at {path}')
        builds.append((label, path))
        build_env[label] = replay.env_pairs(switches)
    tasks = replay.load_tasks()
    if args.tasks:
        wanted = args.tasks.split(',')
        if not set(wanted) <= set(tasks):
            parser.error('Unknown task; choose from: ' + ', '.join(tasks))
        tasks = {task: tasks[task] for task in wanted}
    selected = list(sessions(args.report, tasks, args.call))
    print(f'{len(selected)} sessions from {len(args.report)} report(s); builds: '
          + ', '.join(label for label, _ in builds), flush=True)

    OUTPUT.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix='verdicts-', dir=OUTPUT))
    stand_in = StandInJev()
    rows = []
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix='oko-verdicts-') as scratch:
            workspaces = {}
            for repo in dict.fromkeys(tasks[run['id']][0] for _, run, _, _ in selected):
                workspace = Path(scratch) / 'workspaces' / repo
                workspace.mkdir(parents=True)
                with tarfile.open(replay.BRANCH_STATE / repo / 'baseline.tar') as archive:
                    archive.extractall(workspace, filter='data')
                workspaces[repo] = workspace
            for label, binary in builds:
                for index, (report, run, arguments, call) in enumerate(selected):
                    repo, task = tasks[run['id']]
                    # One index cache per build and repository, reused across sessions.
                    cache = Path(scratch) / 'cache' / label / repo
                    cache.mkdir(parents=True, exist_ok=True)
                    row = {'build': label, 'task': run['id'], 'client': run['client'],
                           'condition': run['condition'], 'session': os.path.basename(run['artifact']),
                           'report': str(report), 'arguments': arguments}
                    try:
                        row.update(replay_session(binary, workspaces[repo], cache, stand_in, arguments, call,
                                                  replay.anchors(task), args.timeout, build_env[label]))
                        state = f"{row['covered']}/{row['expected']}" + ('' if row['reproduced'] else ' (differs)')
                    except Exception as error:  # A failed session is a result, not a reason to stop.
                        row['error'] = f'{type(error).__name__}: {error}'[:300]
                        state = row['error']
                    rows.append(row)
                    print(f"[{label} {index + 1}/{len(selected)}] {run['id']} {row['session'][:3]}: {state}",
                          flush=True)
    finally:
        stand_in.close()
    labels = [label for label, _ in builds]
    summary = summarize(rows, labels)
    (output / 'verdicts.json').write_text(json.dumps(
        {'builds': dict(builds), 'env': build_env, 'reports': [str(r) for r in args.report],
         'call': args.call, 'summary': summary, 'rows': rows}, indent=1) + '\n')
    text = render(summary, labels, args.report)
    (output / 'verdicts.md').write_text(text)
    print('\n' + text)
    print(f'{output}  ({time.monotonic() - started:.0f} s)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
