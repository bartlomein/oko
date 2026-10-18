#!/usr/bin/env python3
"""Cursor app benchmark: the branch suite's nine tasks in Cursor, without and with Oko.

Each session runs in the Cursor app itself, driven by
scripts/ide-harness/cursor_session.mjs over Cursor's debugging port. The
prompts and graders are the public runner's own (`runner.prompt`,
`runner.grade`), as in the Pi suite: `native` is Cursor alone; `oko` is the
project as `oko setup --client cursor` leaves it (guidance in AGENTS.md, the
prompt hook, the MCP connection) with Oko's server turned on in Cursor, and the
prompt's Oko line.

Workspaces are made once per repository and condition from the frozen archives
in benchmarks/results/public-branch/<repo>/baseline.tar, with setup's files
committed into the baseline, and reset with git before every session. Oko's
index is warmed once per workspace. Cursor is restarted on each session's
project before the session (it loads a project's MCP servers only for the folder
it starts on), with the TypeSafe key in its environment, so the hook and the server never wait on a Keychain prompt
and no key is written into a workspace.

Measured per session: wall time from sending the prompt to the end of the turn
in Cursor's transcript, the tool calls in that transcript, and the grade.
Cursor shows no token counts, so none are reported. Cursor's own model and
settings are used as they are; each session records the model shown.

  cursor_bench.py --plan                       # the schedule; no sessions
  cursor_bench.py --execute [--repeats 3] [--tasks id,id] [--resume DIR]

Results: benchmarks/results/cursor/<run>/sessions.jsonl and summary.json.
"""
import argparse
import importlib.util
import json
import os
import shutil
import statistics
import subprocess
import sys
import tarfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PROJECT = ROOT.parents[1]
ARCHIVES = PROJECT / 'benchmarks/results/public-branch'
RESULTS = PROJECT / 'benchmarks/results/cursor'
HOME = Path.home() / '.cache' / 'oko-cursor-bench'
SESSION = PROJECT / 'scripts/ide-harness/cursor_session.mjs'
APP = Path('/Applications/Cursor.app')
PORT = 9333
CONDITIONS = ('native', 'oko')

sys.path.insert(0, str(ROOT))  # the runner imports its neighbours
spec = importlib.util.spec_from_file_location('public_runner', ROOT / 'runner.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
TASKS = json.loads((ROOT / 'tasks-branch.json').read_text())['repositories']


def key():
    """The TypeSafe key: the environment, then this repository's .env."""
    if os.environ.get('TYPESAFE_API_KEY'):
        return os.environ['TYPESAFE_API_KEY']
    path = PROJECT / '.env'
    for line in path.read_text().splitlines() if path.exists() else []:
        if line.strip().startswith('TYPESAFE_API_KEY='):
            return line.split('=', 1)[1].strip().strip('"\'')
    raise SystemExit('No TypeSafe key in the environment or .env')


def schedule(repeats, wanted):
    """Every task in both conditions per repeat; which goes first alternates."""
    sessions = []
    for repeat in range(repeats):
        for index, (repo, task) in enumerate((r['name'], t) for r in TASKS for t in r['tasks']):
            if wanted and task['id'] not in wanted:
                continue
            order = CONDITIONS if (index + repeat) % 2 == 0 else CONDITIONS[::-1]
            sessions += [{'repo': repo, 'task': task['id'], 'condition': c, 'repetition': repeat + 1} for c in order]
    return sessions


def git(work, *args):
    return subprocess.run(['git', *args], cwd=work, check=True, capture_output=True).stdout


def workspace(repo, condition, oko, env):
    """The condition's checkout of one repository, built once."""
    work = HOME / f'{condition}-{repo}'
    if (work / '.git').exists():
        return work
    work.mkdir(parents=True)
    with tarfile.open(ARCHIVES / repo / 'baseline.tar') as tar:
        tar.extractall(work, filter='data')
    runner.engine.strip_customizations(work)
    git(work, 'init', '-q')
    if condition == 'oko':
        subprocess.run([str(oko), 'setup', '--client', 'cursor', '--install-dir', str(HOME / 'install'), '--quiet'],
                       cwd=work, env=env, check=True)
        # A warm index, as in the agent suite's warm condition; no Jev call.
        subprocess.run([str(HOME / 'install' / 'oko'), 'ask', '--no-jev', '--json', 'project source overview'],
                       cwd=work, env=env, check=True, capture_output=True)
    git(work, 'add', '-f', '.')
    git(work, '-c', 'user.name=Benchmark', '-c', 'user.email=benchmark@localhost', '-c', 'commit.gpgsign=false',
        'commit', '-qm', 'Frozen benchmark baseline')
    return work


def reset(work):
    git(work, 'reset', '--hard', '-q')
    git(work, 'clean', '-fdxq')


def listening():
    try:
        with urllib.request.urlopen(f'http://127.0.0.1:{PORT}/json/version', timeout=2):
            return True
    except OSError:
        return False


def start_cursor(env, folders):
    """A fresh Cursor with the key in its environment and the debugging port
    open. Cursor lists a folder under Recents, where the session script picks
    it, only when it starts on that folder; so it starts once on each."""
    for folder in folders:
        subprocess.run(['osascript', '-e', 'quit app "Cursor"'], check=False)
        for _ in range(40):
            if subprocess.run(['pgrep', '-f', f'{APP}/Contents/MacOS/Cursor'], capture_output=True).returncode:
                break
            time.sleep(0.5)
        subprocess.Popen([str(APP / 'Contents/MacOS/Cursor'), f'--remote-debugging-port={PORT}', str(folder)], env=env,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(120):
            if listening():
                break
            time.sleep(0.5)
        else:
            raise SystemExit('Cursor did not open its debugging port')
        time.sleep(6)


def task_of(repo, task_id):
    item = next(r for r in TASKS if r['name'] == repo)
    return dict(next(t for t in item['tasks'] if t['id'] == task_id), repositoryName=repo)


def run_session(session, work, timeout, env):
    task = task_of(session['repo'], session['task'])
    enabled = session['condition'] == 'oko'
    if enabled:
        task['cacheCondition'] = runner.PREFETCH
    text = runner.prompt(task, enabled)
    reset(work)
    # Cursor loads a project's MCP servers only for the folder it starts on; a
    # restart per session, outside the timed part, also keeps sessions apart.
    start_cursor(env, [work])
    command = ['node', str(SESSION), '--cwd', str(work), '--prompt', text, '--timeout', str(timeout)]
    if enabled:
        command.append('--enable-oko')
    done = subprocess.run(command, capture_output=True, text=True, timeout=timeout + 180)
    try:
        report = json.loads(done.stdout.strip().splitlines()[-1])
    except (IndexError, ValueError):
        report = {'error': (done.stderr or done.stdout)[-2000:]}
    grade = runner.grade(task, work, report.get('answer') or '') if not report.get('error') else {'passed': False}
    return {**session, **{k: v for k, v in report.items() if k != 'cwd'}, 'toolCalls': len(report.get('tools') or []),
            'grade': grade, 'passed': bool(grade.get('passed')), 'prompt': text}


def summarize(rows):
    summary = {}
    for condition in CONDITIONS:
        done = [r for r in rows if r['condition'] == condition and not r.get('error')]
        if not done:
            continue
        summary[condition] = {
            'sessions': len(done), 'errors': sum(1 for r in rows if r['condition'] == condition and r.get('error')),
            'passed': sum(r['passed'] for r in done),
            'meanSeconds': round(statistics.mean(r['seconds'] for r in done), 1),
            'medianSeconds': round(statistics.median(r['seconds'] for r in done), 1),
            'meanToolCalls': round(statistics.mean(r['toolCalls'] for r in done), 2),
            'okoCalls': sum(sum(1 for t in r.get('tools') or [] if t.endswith(':search') and 'oko' in t) for r in done),
        }
    if {'native', 'oko'} <= summary.keys():
        n, o = summary['native'], summary['oko']
        summary['change'] = {'time': f"{(o['meanSeconds'] / n['meanSeconds'] - 1) * 100:+.0f}%",
                             'toolCalls': f"{(o['meanToolCalls'] / n['meanToolCalls'] - 1) * 100:+.0f}%"}
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--plan', action='store_true')
    mode.add_argument('--execute', action='store_true')
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--tasks', help='comma-separated task ids (default: all nine)')
    parser.add_argument('--oko', type=Path, default=PROJECT / 'target/release/oko')
    parser.add_argument('--timeout', type=int, default=300)
    parser.add_argument('--resume', type=Path)
    args = parser.parse_args()
    wanted = set(args.tasks.split(',')) if args.tasks else set()
    sessions = schedule(args.repeats, wanted)
    if args.plan:
        for s in sessions:
            print(s['repetition'], s['repo'], s['task'], s['condition'])
        print(f'{len(sessions)} sessions')
        return
    env = {**os.environ, 'TYPESAFE_API_KEY': key()}
    env.pop('CLAUDECODE', None)
    out = args.resume or RESULTS / time.strftime('run-%Y%m%d-%H%M%S')
    out.mkdir(parents=True, exist_ok=True)
    log = out / 'sessions.jsonl'
    finished = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    seen = {(r['repo'], r['task'], r['condition'], r['repetition']) for r in finished}
    version = subprocess.run([str(args.oko), '--version'], capture_output=True, text=True, check=True).stdout.strip()
    works = {(r, c): workspace(r, c, args.oko, env) for r in {s['repo'] for s in sessions} for c in CONDITIONS}
    start_cursor(env, list(works.values()))
    (out / 'settings.json').write_text(json.dumps({
        'oko': version, 'cursor': json.loads((APP / 'Contents/Resources/app/package.json').read_text())['version'],
        'repeats': args.repeats, 'tasks': sorted(wanted) or 'all', 'timeoutSeconds': args.timeout,
        'workspaces': {f'{r}/{c}': str(w) for (r, c), w in works.items()}}, indent=2) + '\n')
    rows = list(finished)
    for number, session in enumerate(sessions, 1):
        if (session['repo'], session['task'], session['condition'], session['repetition']) in seen:
            continue
        row = run_session(session, works[(session['repo'], session['condition'])], args.timeout, env)
        rows.append(row)
        with log.open('a') as handle:
            handle.write(json.dumps(row) + '\n')
        print(f"[{number}/{len(sessions)}] {session['task']} {session['condition']} r{session['repetition']}: "
              f"{'PASS' if row['passed'] else 'FAIL'} {row.get('seconds')} s, {row['toolCalls']} tools"
              + (f", error: {row['error']}" if row.get('error') else ''), flush=True)
    summary = summarize(rows)
    (out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print(out)


if __name__ == '__main__':
    main()
