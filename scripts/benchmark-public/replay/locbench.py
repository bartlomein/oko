#!/usr/bin/env python3
"""Score Oko's retrieval on Loc-Bench (LocAgent, czlll/Loc-Bench_V1).

Each case is a real GitHub issue, a repository at a fixed commit, and the
functions the merged fix edited. The issue text goes to Oko's `search` tool and
the ranked code it returns is compared with those functions. No agent sessions:
this measures retrieval alone, with the metrics the localization papers report.

Cases are split once, by a hash of their id, into `dev` and `heldout`. Tune
against `dev`. Run `heldout` only for a number that will be reported.

  locbench.py --fetch                      # download the cases (no model calls)
  locbench.py --split dev --limit 30       # keyword ranking only, free
  locbench.py --split dev --limit 30 --jev # with Jev: paid calls

Everything lands under benchmarks/results/locbench/, which Git ignores.
"""
import argparse
import ast
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import replay  # noqa: E402  (api_key, profiler, packet_of, JEV_MODEL)

STATE = replay.PROJECT / 'benchmarks/results/locbench'
CASES = STATE / 'cases.jsonl'
ROWS = 'https://datasets-server.huggingface.co/rows?dataset=czlll%2FLoc-Bench_V1&config=default&split=test'
# Oko rejects questions above 4096 bytes; a few issues are far longer.
QUESTION_BYTES = 4000
FILE_K = (1, 3, 5)
FUNCTION_K = (5, 10)


def fetch():
    STATE.mkdir(parents=True, exist_ok=True)
    cases, offset = [], 0
    while True:
        with urllib.request.urlopen(f'{ROWS}&offset={offset}&length=100', timeout=60) as response:
            page = json.load(response)
        for item in page['rows']:
            row = item['row']
            cases.append({key: row[key] for key in (
                'instance_id', 'repo', 'base_commit', 'problem_statement', 'category', 'edit_functions')})
        offset += len(page['rows'])
        if not page['rows'] or offset >= page['num_rows_total']:
            break
    CASES.write_text(''.join(json.dumps(case) + '\n' for case in cases))
    print(f'{len(cases)} cases -> {CASES}')


def split_of(instance_id):
    return 'dev' if hashlib.sha256(instance_id.encode()).digest()[0] % 2 == 0 else 'heldout'


def load(split, limit, repos):
    cases = [json.loads(line) for line in CASES.read_text().splitlines()]
    cases = [case for case in cases if split_of(case['instance_id']) == split]
    if repos:
        cases = [case for case in cases if case['repo'] in repos]
    # A stable order that mixes repositories, so a small --limit is not one project.
    cases.sort(key=lambda case: hashlib.sha256(case['instance_id'].encode()).hexdigest())
    return cases[:limit] if limit else cases


def git(*args, cwd=None):
    return subprocess.run(['git', *args], cwd=cwd, check=True, capture_output=True, text=True).stdout


def checkout(case, target):
    """The repository at the case's commit, from a blob-less mirror kept between runs."""
    mirror = STATE / 'repos' / (case['repo'].replace('/', '__') + '.git')
    if not mirror.exists():
        mirror.parent.mkdir(parents=True, exist_ok=True)
        git('clone', '--quiet', '--bare', '--filter=blob:none', f"https://github.com/{case['repo']}.git", str(mirror))
    try:
        git('cat-file', '-e', case['base_commit'] + '^{commit}', cwd=mirror)
    except subprocess.CalledProcessError:
        git('fetch', '--quiet', 'origin', case['base_commit'], cwd=mirror)
    git('worktree', 'add', '--quiet', '--detach', '--force', str(target), case['base_commit'], cwd=mirror)


def release(case, target):
    mirror = STATE / 'repos' / (case['repo'].replace('/', '__') + '.git')
    subprocess.run(['git', 'worktree', 'remove', '--force', str(target)], cwd=mirror, capture_output=True)
    shutil.rmtree(target, ignore_errors=True)
    subprocess.run(['git', 'worktree', 'prune'], cwd=mirror, capture_output=True)


def function_ranges(workspace, edit_functions):
    """{'path:Qualified.name': (path, start, end)} for the functions the fix edited."""
    wanted, found = {}, {}
    for name in edit_functions:
        path, _, qualified = name.partition(':')
        wanted.setdefault(path, set()).add(qualified)
    for path, names in wanted.items():
        try:
            tree = ast.parse((workspace / path).read_text(encoding='utf-8', errors='replace'))
        except (OSError, SyntaxError, ValueError):
            continue

        def visit(node, prefix):
            for child in ast.iter_child_nodes(node):
                if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                    qualified = f'{prefix}{child.name}'
                    if qualified in names:
                        start = min([child.lineno] + [d.lineno for d in child.decorator_list])
                        found[f'{path}:{qualified}'] = (path, start, child.end_lineno)
                    visit(child, qualified + '.')
        visit(tree, '')
    return found


def ranked(packet):
    """Oko's order: the excerpts it showed, then every judged candidate by score."""
    retrieval = packet.get('retrieval') or {}
    shown = [(e['path'], e['startLine'], e['endLine']) for kind in ('results', 'related')
             for e in packet.get(kind) or []]
    rest = sorted(retrieval.get('candidates') or [], key=lambda c: -(c.get('score') or 0))
    order, seen = [], set()
    for span in shown + [(c['path'], c['startLine'], c['endLine']) for c in rest]:
        if span not in seen:
            seen.add(span)
            order.append(span)
    return order, len(shown)


def score(order, targets, files):
    """Acc@k is strict, as in the LocAgent paper: every target must be in the top k."""
    row = {}
    file_order = list(dict.fromkeys(path for path, _, _ in order))
    for k in FILE_K:
        top = set(file_order[:k])
        row[f'fileAcc@{k}'] = files <= top
        row[f'fileRecall@{k}'] = len(files & top) / len(files)
    for k in FUNCTION_K:
        hit = {name for name, (path, start, end) in targets.items()
               if any(p == path and s <= end and start <= e for p, s, e in order[:k])}
        row[f'functionAcc@{k}'] = bool(targets) and len(hit) == len(targets)
        row[f'functionRecall@{k}'] = len(hit) / len(targets) if targets else None
    return row


def run(case, binary, live, timeout):
    row = {key: case[key] for key in ('instance_id', 'repo', 'category')}
    question = case['problem_statement'].encode()[:QUESTION_BYTES].decode(errors='ignore')
    row['questionTrimmed'] = len(question.encode()) < len(case['problem_statement'].encode())
    workspace = STATE / 'work' / case['instance_id']
    module = replay.profiler()
    started = time.monotonic()
    try:
        checkout(case, workspace)
        files = {name.partition(':')[0] for name in case['edit_functions']}
        targets = function_ranges(workspace, case['edit_functions'])
        row.update(targetFiles=len(files), targetFunctions=len(case['edit_functions']),
                   targetsResolved=len(targets))
        with tempfile.TemporaryDirectory(prefix='oko-locbench-cache-') as cache:
            client = module.Client(Path(binary), workspace, Path(cache), timeout, live=live,
                                   api_key=replay.api_key() if live else None,
                                   model=replay.JEV_MODEL if live else None)
            try:
                client.initialize()
                response = client.request('tools/call', {'name': 'search', 'arguments': {
                    'question': question, 'intent': 'implementation'}})
                packet = replay.packet_of(client, response)
            finally:
                client.close()
        order, shown = ranked(packet)
        retrieval = packet.get('retrieval') or {}
        row.update(score(order, targets, files), ranking=packet.get('ranking'), shown=shown,
                   candidates=len(order), okoMs=(packet.get('timings') or {}).get('totalMs'),
                   jevMs=retrieval.get('rerankMs'),
                   # Where a miss happened: was the file even among the judged candidates?
                   fileShortlisted=files <= {path for path, _, _ in order},
                   top=[f'{p}:{s}-{e}' for p, s, e in order[:5]], targets=sorted(case['edit_functions']))
    except Exception as error:  # A failed case is a result, not a reason to stop.
        row['error'] = f'{type(error).__name__}: {error}'[:300]
    finally:
        release(case, workspace)
    row['seconds'] = round(time.monotonic() - started, 1)
    return row


def summarize(rows):
    scored = [row for row in rows if 'error' not in row]
    out = {'cases': len(rows), 'scored': len(scored), 'errors': len(rows) - len(scored)}
    for key in [f'fileAcc@{k}' for k in FILE_K] + [f'functionAcc@{k}' for k in FUNCTION_K]:
        values = [row[key] for row in scored if row.get(key) is not None]
        out[key] = round(100 * sum(values) / len(values), 1) if values else None
    for key in ('fileRecall@5', 'functionRecall@10'):
        values = [row[key] for row in scored if row.get(key) is not None]
        out[key] = round(100 * sum(values) / len(values), 1) if values else None
    out['fileShortlisted'] = round(100 * sum(bool(r.get('fileShortlisted')) for r in scored) / len(scored), 1) if scored else None
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--fetch', action='store_true', help='Download the cases; no model calls')
    parser.add_argument('--split', choices=('dev', 'heldout'), default='dev')
    parser.add_argument('--limit', type=int, default=0)
    parser.add_argument('--repos', help='Comma-separated owner/name filter')
    parser.add_argument('--jev', action='store_true', help='Rank with Jev: paid calls')
    parser.add_argument('--binary', default=str(replay.PROJECT / 'target/release/oko'))
    parser.add_argument('--label', default=None)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.fetch:
        return fetch()
    if not CASES.exists():
        parser.error('Run --fetch first')
    if args.split == 'heldout':
        print('HELD-OUT split: run this only for a number you will report.', file=sys.stderr)
    cases = load(args.split, args.limit, set(args.repos.split(',')) if args.repos else None)
    label = args.label or ('jev' if args.jev else 'keywords')
    out = STATE / f'{args.split}-{label}-{time.strftime("%Y%m%d-%H%M%S")}.jsonl'
    rows = []
    with out.open('w') as handle:
        for index, case in enumerate(cases, 1):
            row = run(case, args.binary, args.jev, args.timeout)
            rows.append(row)
            handle.write(json.dumps(row) + '\n')
            handle.flush()
            mark = row.get('error') or f"file@5={row['fileAcc@5']} function@10={row['functionAcc@10']}"
            print(f"{index}/{len(cases)} {case['instance_id']} {row['seconds']}s {mark}", flush=True)
    summary = summarize(rows)
    (out.with_suffix('.summary.json')).write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print(out)


if __name__ == '__main__':
    main()
