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
# `tuned`: an experiment for Cursor's model, which re-checks attached context at
# length. A build given with --tuned-oko (the hook's header and size for
# Cursor), a .cursor/rules file, and a prompt line that does not ask for a
# search when the answer is already there.
TUNED_RULE = """---
description: Oko code search
alwaysApply: true
---
When an "Oko answer for this prompt" block is in your context, it is current repository text with exact
line numbers. If it answers the question, answer from it directly, without re-reading or re-checking
those files. Use the oko `search` tool for anything it does not show, before grep or file reads.
"""
# `tuned2`: tuned, plus a line of code locations above the answer (in the
# build given with --tuned2-oko) and a rule that Oko's search can be called
# without first fetching its schema, a round Cursor's model otherwise spends.
TUNED2_RULE = TUNED_RULE + """Oko's tool is `search` on the oko MCP server. Call it directly with arguments
{"question": "<what to find>"} (optionally "symbols": "NameOne, NameTwo"); it needs no schema lookup first.
"""
# `pointer` and `lean`: Oko's answer without the costly follow-ups. Both use
# the build given with --exp-oko and a neutral prompt line (no "search with
# Oko first"). `pointer` gives only where the code is (OKO_CURSOR_PREFETCH,
# set in Cursor's environment) for Cursor's own fast reads; `lean` gives the
# excerpts. Either rule sends follow-ups to Cursor's own grep and reads, not
# to Oko's MCP tool, which costs a schema lookup and a round each time.
POINTER_RULE = """---
description: Oko code search
alwaysApply: true
---
When an "Oko answer for this prompt" block lists code locations, they are exact and current: open
those lines with your read tool first and answer from them. For anything else, use your own grep and
file reads.
"""
LEAN_RULE = """---
description: Oko code search
alwaysApply: true
---
When an "Oko answer for this prompt" block is in your context, it is current repository text with exact
line numbers. If it answers the question, answer from it directly, without re-reading or re-checking
those files. For anything it does not show, use your own grep and file reads.
"""
NEUTRAL = ("Oko's answer to this prompt may already be in your context; use it when it answers the question. "
           'Use native search/read tools for anything else.')
# `tuned3`: tuned's build (no locations line) with tuned2's rule.
VARIANTS = {'tuned': (TUNED_RULE, 'install-tuned'), 'tuned2': (TUNED2_RULE, 'install-tuned2'),
            'tuned3': (TUNED2_RULE, 'install-tuned3'), 'pointer': (POINTER_RULE, 'install-exp'),
            'lean': (LEAN_RULE, 'install-exp')}
CURSOR_ENV = {'pointer': {'OKO_CURSOR_PREFETCH': 'pointer'}}
# `release`: the project exactly as `oko setup --client cursor` from the --oko
# build leaves it (its header, cap and rule), with the neutral prompt line.
OKO_CONDITIONS = ('oko', 'tuned', 'tuned2', 'tuned3', 'pointer', 'lean', 'release')
SEARCH_FIRST = ('Use Oko MCP search first to locate the relevant code, phrasing the search from the request. '
                'Use its evidence when sufficient; native follow-up reads/searches are allowed.')
ANSWER_FIRST = ("Oko's answer to this prompt may already be in your context; when it answers the question, answer "
                'from it. Use Oko MCP search for any part it does not show; native follow-up reads/searches are allowed.')

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


def schedule(repeats, wanted, conditions=CONDITIONS):
    """Every task in every condition per repeat; which goes first rotates."""
    sessions = []
    for repeat in range(repeats):
        for index, (repo, task) in enumerate((r['name'], t) for r in TASKS for t in r['tasks']):
            if wanted and task['id'] not in wanted:
                continue
            shift = (index + repeat) % len(conditions)
            order = conditions[shift:] + conditions[:shift]
            sessions += [{'repo': repo, 'task': task['id'], 'condition': c, 'repetition': repeat + 1} for c in order]
    return sessions


def git(work, *args):
    return subprocess.run(['git', *args], cwd=work, check=True, capture_output=True).stdout


def workspace(repo, condition, oko, env, tuned=None, tuned2=None, exp=None):
    """The condition's checkout of one repository, built once."""
    work = HOME / f'{condition}-{repo}'
    if (work / '.git').exists():
        return work
    work.mkdir(parents=True)
    with tarfile.open(ARCHIVES / repo / 'baseline.tar') as tar:
        tar.extractall(work, filter='data')
    runner.engine.strip_customizations(work)
    git(work, 'init', '-q')
    if condition in OKO_CONDITIONS:
        build = {'oko': oko, 'tuned': tuned, 'tuned2': tuned2, 'tuned3': tuned, 'pointer': exp, 'lean': exp,
                 'release': oko}[condition]
        install = HOME / (VARIANTS[condition][1] if condition in VARIANTS
                          else 'install-release' if condition == 'release' else 'install')
        # Setup saves a key it finds in the environment to the Keychain, which
        # waits on a macOS prompt; a key in the project's .env it only uses.
        # The .env is removed before the baseline commit and the sessions.
        quiet_env = {k: v for k, v in env.items() if k != 'TYPESAFE_API_KEY'}
        (work / '.env').write_text(f"TYPESAFE_API_KEY={env['TYPESAFE_API_KEY']}\n")
        try:
            subprocess.run([str(build), 'setup', '--client', 'cursor', '--install-dir', str(install), '--quiet'],
                           cwd=work, env=quiet_env, check=True, timeout=300)
            # A warm index, as in the agent suite's warm condition; no Jev call.
            subprocess.run([str(install / 'oko'), 'ask', '--no-jev', '--json', 'project source overview'],
                           cwd=work, env=quiet_env, check=True, capture_output=True, timeout=300)
        finally:
            (work / '.env').unlink(missing_ok=True)
        if condition in VARIANTS:
            (work / '.cursor/rules').mkdir(parents=True, exist_ok=True)
            (work / '.cursor/rules/oko.mdc').write_text(VARIANTS[condition][0])
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


def size_window(width=1600, height=1000):
    """Cursor reopens at the size it last closed with, and its debugging port
    cannot resize a window; a narrow one hides the sidebar and the chips the
    session script uses. So the saved size is set before Cursor starts."""
    path = Path.home() / 'Library/Application Support/Cursor/User/globalStorage/storage.json'
    if not path.exists():
        return
    state = json.loads(path.read_text())
    window = state.setdefault('windowsState', {}).setdefault('lastActiveWindow', {})
    window.setdefault('uiState', {}).update(x=40, y=40, width=width, height=height)
    path.write_text(json.dumps(state, indent=4))


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
        size_window()
        subprocess.Popen([str(APP / 'Contents/MacOS/Cursor'), f'--remote-debugging-port={PORT}', str(folder)], env=env,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(120):
            if listening():
                break
            time.sleep(0.5)
        else:
            raise SystemExit('Cursor did not open its debugging port')
        time.sleep(6)


CURSOR_LOGS = Path.home() / 'Library/Application Support/Cursor/logs'


def trace_of(since):
    """Cursor's request trace for the session that began after `since`: time to
    the first token, and when each thinking step ended, in seconds from the
    request. Cursor keeps logs for only its last few launches, so it is read
    right after the session."""
    import datetime
    import re
    best = None
    for path in CURSOR_LOGS.glob('*/*/output_*/cursor.requestTraces.log'):
        if path.stat().st_mtime < since:
            continue
        events = []
        for line in path.read_text(errors='replace').splitlines():
            match = re.match(r'(\S+) (\S+).*?name="([^"]+)"', line)
            if match and match.group(2) == 'span_completed' or (match and match.group(3) == 'agent.request'):
                when = datetime.datetime.fromisoformat(match.group(1).replace('Z', '+00:00')).timestamp()
                events.append((when, match.group(3)))
        starts = [t for t, name in events if name == 'agent.request' and t >= since - 5]
        if not starts:
            continue
        start = starts[0]
        ttft = next((t - start for t, name in events if name == 'client.ttft' and t >= start), None)
        thinking = sorted({round(t - start, 1) for t, name in events
                           if name == 'AgentResponseAdapter.thinkingCompleted' and t >= start})
        if best is None or start < best['start']:
            best = {'start': start, 'ttftSeconds': round(ttft, 2) if ttft is not None else None,
                    'thinkingEndsSeconds': thinking}
    if best:
        best.pop('start')
    return best


def task_of(repo, task_id):
    item = next(r for r in TASKS if r['name'] == repo)
    return dict(next(t for t in item['tasks'] if t['id'] == task_id), repositoryName=repo)


def run_session(session, work, timeout, env, out, effort=None, tuned_prompt=True):
    task = task_of(session['repo'], session['task'])
    enabled = session['condition'] in OKO_CONDITIONS
    if enabled:
        task['cacheCondition'] = runner.PREFETCH
    text = runner.prompt(task, enabled)
    if session['condition'] == 'tuned' and tuned_prompt:
        if SEARCH_FIRST not in text:
            raise RuntimeError('Benchmark prompt changed; update the tuned condition')
        text = text.replace(SEARCH_FIRST, ANSWER_FIRST)
    if session['condition'] in ('pointer', 'lean', 'release'):
        if SEARCH_FIRST not in text:
            raise RuntimeError('Benchmark prompt changed; update the pointer and lean conditions')
        text = text.replace(SEARCH_FIRST, NEUTRAL)
    reset(work)
    # Cursor loads a project's MCP servers only for the folder it starts on; a
    # restart per session, outside the timed part, also keeps sessions apart.
    start_cursor({**env, **CURSOR_ENV.get(session['condition'], {})}, [work])
    shot = out / f"{session['task']}-{session['condition']}-r{session['repetition']}.png"
    command = ['node', str(SESSION), '--cwd', str(work), '--prompt', text, '--timeout', str(timeout), '--screenshot', str(shot)]
    if enabled:
        command.append('--enable-oko')
    if effort:
        command += ['--effort', effort]
    began = time.time()
    done = subprocess.run(command, capture_output=True, text=True, timeout=timeout + 180)
    try:
        report = json.loads(done.stdout.strip().splitlines()[-1])
    except (IndexError, ValueError):
        report = {'error': (done.stderr or done.stdout)[-2000:]}
    grade = runner.grade(task, work, report.get('answer') or '') if not report.get('error') else {'passed': False}
    report['trace'] = trace_of(began)
    # For matching Cursor's usage export (tokens per request) to this session.
    report['startedAt'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(began))
    report['endedAt'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    return {**session, **{k: v for k, v in report.items() if k != 'cwd'}, 'toolCalls': len(report.get('tools') or []),
            'grade': grade, 'passed': bool(grade.get('passed')), 'prompt': text}


def summarize(rows):
    summary = {}
    for condition in ('native',) + OKO_CONDITIONS:
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
    parser.add_argument('--conditions', default=','.join(CONDITIONS), help='comma-separated: native, oko, tuned')
    parser.add_argument('--tuned-oko', type=Path, help='the Oko build for the tuned condition')
    parser.add_argument('--tuned2-oko', type=Path, help='the Oko build for the tuned2 condition')
    parser.add_argument('--exp-oko', type=Path, help='the Oko build for the pointer and lean conditions')
    parser.add_argument('--tuned-keeps-prompt', action='store_true',
                        help="tuned keeps the standard prompt's Oko line: only Oko's own changes differ")
    parser.add_argument('--effort', choices=('Low', 'Medium', 'High', 'Extra High'),
                        help="set the model's reasoning effort in Cursor before each session (it stays set)")
    args = parser.parse_args()
    wanted = set(args.tasks.split(',')) if args.tasks else set()
    conditions = tuple(args.conditions.split(','))
    if (not set(conditions) <= {'native', *OKO_CONDITIONS}
            or ({'pointer', 'lean'} & set(conditions) and not args.exp_oko)
            or ({'tuned', 'tuned3'} & set(conditions) and not args.tuned_oko)
            or ('tuned2' in conditions and not args.tuned2_oko)):
        parser.error('Conditions are native, oko, tuned and tuned2; tuned needs --tuned-oko, tuned2 --tuned2-oko')
    sessions = schedule(args.repeats, wanted, conditions)
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
    # A session the harness failed to run is run again; its row is kept aside.
    failed = [row for row in finished if row.get('error')]
    finished = [row for row in finished if not row.get('error')]
    if failed:
        with (out / 'harness-errors.jsonl').open('a') as handle:
            handle.writelines(json.dumps(row) + '\n' for row in failed)
        log.write_text(''.join(json.dumps(row) + '\n' for row in finished))
    seen = {(r['repo'], r['task'], r['condition'], r['repetition']) for r in finished}
    version = subprocess.run([str(args.oko), '--version'], capture_output=True, text=True, check=True).stdout.strip()
    works = {(r, c): workspace(r, c, args.oko, env, args.tuned_oko, args.tuned2_oko, args.exp_oko)
             for r in {s['repo'] for s in sessions} for c in conditions}
    start_cursor(env, list(works.values()))
    (out / 'settings.json').write_text(json.dumps({
        'oko': version, 'cursor': json.loads((APP / 'Contents/Resources/app/package.json').read_text())['version'],
        'repeats': args.repeats, 'tasks': sorted(wanted) or 'all', 'timeoutSeconds': args.timeout,
        'workspaces': {f'{r}/{c}': str(w) for (r, c), w in works.items()}}, indent=2) + '\n')
    rows = list(finished)
    for number, session in enumerate(sessions, 1):
        if (session['repo'], session['task'], session['condition'], session['repetition']) in seen:
            continue
        row = run_session(session, works[(session['repo'], session['condition'])], args.timeout, env, out, args.effort,
                          not args.tuned_keeps_prompt)
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
