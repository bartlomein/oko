#!/usr/bin/env python3
"""Replay real agent sessions with consecutive calls batched into one call.

Agents fire runs of 2-8 Oko calls in a row. This takes the corpus, groups
calls by session, splits each session into runs of adjacent calls (same
repository, positions in sequence), and for every run of two or more calls
sends one `questions` call to the build under test. It records what the
batching buys: calls that disappear, wall time against the sum of separate
calls, bytes against the sum of separate answers, Jev requests, and whether
the identifiers the separate answers declared are still declared.

  sessions.py --corpus plans/corpus/calls.jsonl --binary target/release/oko \
      --separate benchmarks/results/corpus/<label>-nojev-<stamp>.jsonl --label p2b [--jev]

`--separate` is a corpus.py result file for the same build: its rows give
the separate answers' bytes, times and declared identifiers per call.
"""
import argparse
import json
import sys
import tempfile
import time
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402
import replay  # noqa: E402

MAX_BATCH = 8


def runs_of(calls):
    """Consecutive calls of one session, in order of position, that one
    `questions` call can carry: same directory and same intent (the batch is
    sent with a single intent)."""
    calls = sorted(calls, key=lambda c: c['pos'])
    runs, current = [], [calls[0]]
    for previous, call in zip(calls, calls[1:]):
        if (call['pos'] == previous['pos'] + 1
                and call.get('directory') == previous.get('directory')
                and call.get('intent') == previous.get('intent')):
            current.append(call)
        else:
            runs.append(current)
            current = [call]
    runs.append(current)
    return runs


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--corpus', required=True)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--separate', required=True, help='corpus.py rows for the separate calls')
    parser.add_argument('--label', required=True)
    parser.add_argument('--out', default=str(replay.PROJECT / 'benchmarks/results/corpus'))
    parser.add_argument('--jev', action='store_true')
    parser.add_argument('--only', help='Comma-separated repo names')
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--env', action='append', default=[], help='KEY=VALUE passed to the Oko process')
    args = parser.parse_args()
    calls = [json.loads(line) for line in Path(args.corpus).read_text().splitlines() if line.strip()]
    separate = {(r['session'], r['idx']): r for r in (json.loads(l) for l in Path(args.separate).read_text().splitlines())}
    by_session = defaultdict(list)
    for call in calls:
        if call['repo'] in corpus.DEFAULT_REPOS and corpus.DEFAULT_REPOS[call['repo']].exists():
            if not args.only or call['repo'] in args.only.split(','):
                by_session[(call['repo'], call['session'])].append(call)
    batches = []
    for (repo, session), session_calls in by_session.items():
        for run in runs_of(session_calls):
            if len(run) >= 2:
                pieces = [run[start:start + MAX_BATCH] for start in range(0, len(run), MAX_BATCH)]
                # A leftover single call joins the previous batch when it has room,
                # else it is not a batch.
                if len(pieces) > 1 and len(pieces[-1]) == 1:
                    last = pieces.pop()
                    if len(pieces[-1]) < MAX_BATCH:
                        pieces[-1].extend(last)
                batches.extend((repo, piece) for piece in pieces if len(piece) >= 2)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / f'{args.label}-sessions-{"jev" if args.jev else "nojev"}-{time.strftime("%Y%m%d-%H%M%S")}.jsonl'
    rows = []
    by_repo = defaultdict(list)
    for repo, run in batches:
        by_repo[repo].append(run)
    with out.open('w') as handle:
        for repo in sorted(by_repo):
            with tempfile.TemporaryDirectory(prefix='oko-sessions-cache-') as cache:
                with replay.oko_client(args.binary, corpus.DEFAULT_REPOS[repo], cache, args.timeout, args.jev,
                                       replay.env_pairs(args.env)) as client:
                    client.request('tools/call', {'name': 'search', 'arguments': {'question': 'warm up'}})
                    for run in by_repo[repo]:
                        questions = [c['question'] for c in run]
                        arguments = {'questions': questions}
                        if run[0].get('directory'):
                            arguments['directory'] = run[0]['directory']
                        if run[0].get('intent'):
                            arguments['intent'] = run[0]['intent']
                        sep_rows = [separate.get((c['session'], c['idx'])) for c in run]
                        row = {'repo': repo, 'session': run[0]['session'], 'idxs': [c['idx'] for c in run], 'n': len(run)}
                        started = time.monotonic()
                        try:
                            response = client.request('tools/call', {'name': 'search', 'arguments': arguments})
                            packet = replay.packet_of(client, response)
                            text = ''.join(b.get('text', '') for b in response.get('content', []))
                            retrieval = packet.get('retrieval') or {}
                            names = []
                            for c in run:
                                names.extend(n for n in corpus.identifiers(c['question']) if n not in names)
                            hits = corpus.declaration_hits(packet, names)
                            sep_hits = set()
                            for r in sep_rows:
                                if r and 'error' not in r:
                                    sep_hits.update(r.get('identifierHits') or [])
                            row.update(
                                error=bool(response.get('isError')),
                                wallMs=round(1000 * (time.monotonic() - started)),
                                okoMs=(packet.get('timings') or {}).get('totalMs'),
                                textBytes=len(text),
                                results=len(packet.get('results') or []),
                                jevCalls=len(retrieval.get('jevCalls') or []),
                                tags=sorted({r.get('tag') for r in packet.get('results') or [] if r.get('tag')}),
                                emptyQuestions=sum(1 for q in retrieval.get('questions') or [] if q.get('results') == 0 and q.get('pins') == 0),
                                batchHits=sorted(hits), separateHits=sorted(sep_hits),
                                separateBytes=sum(r['textBytes'] for r in sep_rows if r and 'error' not in r),
                                separateMs=sum((r.get('okoMs') or 0) for r in sep_rows if r and 'error' not in r),
                                separateJevCalls=sum((r.get('jevCalls') or 0) for r in sep_rows if r and 'error' not in r),
                                separateOk=sum(1 for r in sep_rows if r and 'error' not in r))
                        except Exception as error:  # A failed batch is a result, not a reason to stop.
                            row['error'] = f'{type(error).__name__}: {error}'[:300]
                        rows.append(row)
                        handle.write(json.dumps(row) + '\n')
                        handle.flush()
                        print(f"  {repo} n={row['n']} {row.get('error') or f'{row['textBytes']}B vs {row.get('separateBytes')}B, {row['okoMs']}ms vs {row.get('separateMs')}ms, jev {row['jevCalls']} vs {row.get('separateJevCalls')}, hits {len(row['batchHits'])}/{len(row['separateHits'])}'}", flush=True)
    ok = [r for r in rows if not r.get('error') and r.get('separateOk') == r['n']]
    calls_saved = sum(r['n'] - 1 for r in rows if not r.get('error'))
    summary = {
        'label': args.label, 'jev': args.jev, 'batches': len(rows), 'errors': sum(1 for r in rows if r.get('error')),
        'callsBefore': sum(r['n'] for r in rows), 'callsAfter': sum(1 for r in rows if not r.get('error')), 'callsSaved': calls_saved,
        'comparable': len(ok),
        'bytesBatched': sum(r['textBytes'] for r in ok), 'bytesSeparate': sum(r['separateBytes'] for r in ok),
        'msBatched': sum(r['okoMs'] or 0 for r in ok), 'msSeparate': sum(r['separateMs'] for r in ok),
        'jevBatched': sum(r['jevCalls'] for r in ok), 'jevSeparate': sum(r['separateJevCalls'] for r in ok),
        'separateHitsTotal': sum(len(r['separateHits']) for r in ok),
        'separateHitsKept': sum(len(set(r['separateHits']) & set(r['batchHits'])) for r in ok),
        'batchOnlyHits': sum(len(set(r['batchHits']) - set(r['separateHits'])) for r in ok),
        'emptyQuestions': sum(r.get('emptyQuestions', 0) for r in ok),
        'sizes': dict(sorted(Counter(r['n'] for r in rows).items())),
    }
    out.with_suffix('.summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print(out)


if __name__ == '__main__':
    main()
