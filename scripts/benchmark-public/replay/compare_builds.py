#!/usr/bin/env python3
"""Replay the same Oko calls through two builds and diff every answer.

For changes meant to leave answers alone (refactors, speedups): each recorded
agent session is replayed in order through one fresh `oko mcp --no-jev`
process per build, as the agent made the calls, and the answer texts are
compared. Free and local; no Jev calls.

  compare_builds.py --before /tmp/oko-old --after target/release/oko \\
      --transcripts 'benchmarks/results/sense-bench/results-run*/oko-dev/*/run-*/transcript.json' \\
      [--corpus plans/corpus/calls.jsonl] [--repos name=path ...] [--workers 3]

Sources: Claude Code transcripts (their Oko `search` calls, one session per
transcript, repository taken from the path: .../<repo>/run-N/transcript.json)
and a replay corpus (one session per repository). Differing answers are
written in full under benchmarks/results/compare-builds/<time>/; the exit
status is 1 when any answer differs.

Two things depend on timing, not on the build. The index line's `watched`,
`rescanned` or `built now` is ignored. Parsing a file has a time limit, so on
a saturated machine the `parsed for symbols` count can differ by a file or
two: rerun a session that differs only there alone, or with fewer workers.
"""
import argparse
import glob
import json
import re
import sys
import tempfile
import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402
import replay  # noqa: E402
import transcripts  # noqa: E402

WATCH_STATE = re.compile(r', (?:watched|rescanned|built now)(?= ·|\n|$)', re.M)


def comparable(text):
    return WATCH_STATE.sub('', text)


def play(binary, root, calls):
    """Every call's answer text from one fresh server, in order."""
    out = []
    with tempfile.TemporaryDirectory(prefix='oko-compare-') as cache:
        with replay.oko_client(binary, root, cache, 600, False) as client:
            for arguments in calls:
                try:
                    response = client.request('tools/call', {'name': 'search', 'arguments': arguments})
                    text = ''.join(b.get('text', '') for b in response.get('content', []))
                    out.append(('error ' if response.get('isError') else '') + text)
                except Exception as error:  # A failed call is an answer to compare.
                    out.append(f'EXCEPTION {type(error).__name__}: {error}')
    return out


def sessions(args, repos):
    """(label, workspace, calls) for every transcript and corpus repository."""
    found = []
    for pattern in args.transcripts:
        for path in sorted(glob.glob(pattern)):
            repo = Path(path).parent.parent.name
            calls = transcripts.calls_of(path)
            if calls and repo in repos:
                found.append((path, repos[repo], calls))
    if args.corpus:
        by_repo = defaultdict(list)
        for line in Path(args.corpus).read_text().splitlines():
            if not line.strip():
                continue
            call = json.loads(line)
            arguments = {'question': call['question']}
            for key in ('intent', 'deep', 'directory'):
                if call.get(key):
                    arguments[key] = call[key]
            by_repo[call['repo']].append(arguments)
        for repo, calls in sorted(by_repo.items()):
            if repo in repos:
                found.append((f'corpus:{repo}', repos[repo], calls))
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--before', required=True, help='Oko binary to compare against')
    parser.add_argument('--after', required=True, help='Oko binary under test')
    parser.add_argument('--transcripts', action='append', default=[], help='Transcript glob, repeated')
    parser.add_argument('--corpus', help='Replay corpus (JSON lines with repo and question)')
    parser.add_argument('--repos', action='append', default=[], help='name=path, repeated')
    parser.add_argument('--workers', type=int, default=3, help='Sessions replayed at once')
    args = parser.parse_args()
    repos = {name: path for name, path in corpus.DEFAULT_REPOS.items() if path.exists()}
    for item in args.repos:
        name, _, path = item.partition('=')
        repos[name] = Path(path)
    jobs = sessions(args, repos)
    if not jobs:
        raise SystemExit('No sessions: pass --transcripts or --corpus for repositories that exist here.')
    out = replay.PROJECT / 'benchmarks/results/compare-builds' / time.strftime('%Y%m%d-%H%M%S')

    def job(label, root, calls):
        before, after = play(args.before, root, calls), play(args.after, root, calls)
        diffs = [{'call': i, 'arguments': calls[i], 'before': a, 'after': b}
                 for i, (a, b) in enumerate(zip(before, after)) if comparable(a) != comparable(b)]
        if diffs:
            out.mkdir(parents=True, exist_ok=True)
            (out / (re.sub(r'[^\w.-]+', '_', label)[-150:] + '.json')).write_text(json.dumps(diffs, indent=1))
        return label, len(calls), diffs

    total_calls = total_diffs = 0
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        for label, n, diffs in pool.map(lambda j: job(*j), jobs):
            total_calls += n
            total_diffs += len(diffs)
            print(f'{label}: {n} calls, {len(diffs)} differ', flush=True)
    print(f'{len(jobs)} sessions, {total_calls} calls, {total_diffs} differ'
          + (f'; differences in {out}' if total_diffs else ''))
    sys.exit(1 if total_diffs else 0)


if __name__ == '__main__':
    main()
