#!/usr/bin/env python3
"""Score Oko's retrieval on Agent Retrieval Bench (arXiv 2607.24882, eyuansu71/agent_retrieval_bench).

Each sample is a repository at a fixed commit, a signal from a coding workflow
(a failing test's output, a PR description, a review comment, an edit), and the
files a developer needs next. The signal goes to Oko's `search` tool and the
ranked code it returns is scored by the benchmark's own metric code, so the
numbers are computed exactly as the published baselines were. No agent sessions.

Samples are split once, by a hash of their id, into `dev` and `heldout`. Tune
against `dev`. Run `heldout` only for a number that will be reported.

  arb.py --fetch                              # data (1.5 GB) and the evaluator; no model calls
  arb.py --split dev --limit 40               # keyword ranking only, free
  arb.py --split dev --limit 40 --jev         # with Jev: paid calls
  arb.py --split dev --tasks trace2code --jev

Everything lands under benchmarks/results/arb/, which Git ignores.
"""
import argparse
import hashlib
import json
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

STATE = replay.PROJECT / 'benchmarks/results/arb'
DATA = STATE / 'data'
EVALUATOR = STATE / 'evaluator'
EVALUATOR_REPO = 'https://github.com/eyuansu62/agent-retrieval-bench'
# The metric code these results are computed with.
EVALUATOR_COMMIT = '07014c986f3deadb1548c62b32c0ffbe6a81465d'
HUB = 'https://huggingface.co/datasets/eyuansu71/agent_retrieval_bench/resolve/main/releases'
TASKS = ('code2test', 'comment2context', 'trace2code', 'edit2ripple')
# Oko rejects questions above 4096 bytes; some logs and PR texts are longer.
QUESTION_BYTES = 4000
BUDGET = 8_000


def fetch():
    DATA.mkdir(parents=True, exist_ok=True)
    for task in TASKS:
        name = f'v2_{task}'
        if (DATA / 'benchmark' / name / 'samples.jsonl').exists():
            print(f'{name}: already unpacked')
            continue
        archive = DATA / f'{name}.tar.zst'
        url = f'{HUB}/{name}/agent_retrieval_bench_{name}.tar.zst'
        print(f'{name}: downloading', flush=True)
        urllib.request.urlretrieve(url, archive)
        with urllib.request.urlopen(url + '.sha256', timeout=60) as response:
            expected = response.read().decode().split()[0]
        with archive.open('rb') as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != expected:
                raise RuntimeError(f'{name}: checksum mismatch')
        unpack = subprocess.Popen(['zstd', '-dc', str(archive)], stdout=subprocess.PIPE)
        subprocess.run(['tar', '-x', '-C', str(DATA)], stdin=unpack.stdout, check=True)
        if unpack.wait():
            raise RuntimeError(f'{name}: zstd failed')
        archive.unlink()
    if not EVALUATOR.exists():
        subprocess.run(['git', 'clone', '--quiet', EVALUATOR_REPO, str(EVALUATOR)], check=True)
    subprocess.run(['git', 'checkout', '--quiet', EVALUATOR_COMMIT], cwd=EVALUATOR, check=True)
    print(f'{len(load(None, 0, TASKS))} samples; evaluator at {EVALUATOR_COMMIT[:12]}')


def metrics_module():
    sys.path.insert(0, str(EVALUATOR / 'src'))
    from agent_retrieval_bench import baseline
    return baseline


def split_of(sample_id):
    return 'dev' if hashlib.sha256(sample_id.encode()).digest()[0] % 2 == 0 else 'heldout'


def load(split, limit, tasks):
    samples = []
    for task in tasks:
        path = DATA / 'benchmark' / f'v2_{task}' / 'samples.jsonl'
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        rows = [row for row in rows if split is None or split_of(row['id']) == split]
        # A stable order that mixes repositories, so a small --limit is not one project.
        rows.sort(key=lambda row: hashlib.sha256(row['id'].encode()).hexdigest())
        samples += rows[:limit] if limit else rows
    return samples


def question_of(sample):
    """The workflow signal as plain text: every string the query holds, shortest first.

    The dataset stores fields alphabetically, which puts a long diff or log ahead
    of the title or intent. Shortest first means the size limit cuts the bulk.
    """
    parts = []

    def walk(value):
        if isinstance(value, str):
            if value.strip():
                parts.append(value.strip())
        elif isinstance(value, dict):
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    walk(sample['query'])
    text = '\n\n'.join(sorted(parts, key=len))
    trimmed = text.encode()[:QUESTION_BYTES].decode(errors='ignore')
    return trimmed, len(trimmed.encode()) < len(text.encode())


def build_workspace(sample, target):
    """The repository at the sample's commit, from the whole-file rows of the released corpus.

    Oko then searches exactly the files the published baselines ranked.
    """
    corpus = DATA / 'corpus' / f"v2_{sample['task_type']}"
    chunks = corpus / sample['repo'].replace('/', '__') / f"{sample['base_commit']}.chunks.jsonl"
    files = 0
    with chunks.open() as stream:
        for line in stream:
            row = json.loads(line)
            if row['kind'] != 'file':
                continue
            path = Path(row['path'])
            if path.is_absolute() or '..' in path.parts:
                raise ValueError('Corpus path escapes the workspace: ' + row['path'])
            dest = target / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(row['text'] + '\n')
            files += 1
    return files


def ranked(packet, workspace):
    """Oko's order as benchmark chunks: the excerpts it showed, then every judged candidate by score."""
    retrieval = packet.get('retrieval') or {}
    shown = [(e['path'], e['startLine'], e['endLine']) for kind in ('results', 'related')
             for e in packet.get(kind) or []]
    rest = sorted(retrieval.get('candidates') or [], key=lambda c: -(c.get('score') or 0))
    chunks, seen, lines = [], set(), {}
    for span in shown + [(c['path'], c['startLine'], c['endLine']) for c in rest]:
        if span in seen:
            continue
        seen.add(span)
        path, start, end = span
        if path not in lines:
            try:
                lines[path] = (workspace / path).read_text(errors='replace').splitlines()
            except OSError:
                lines[path] = []
        chunks.append({'path': path, 'start_line': start, 'end_line': end,
                       'text': '\n'.join(lines[path][start - 1:end])})
    return chunks, len(shown)


def run(sample, binary, live, timeout, intent, baseline, extra_env=()):
    row = {'id': sample['id'], 'task': sample['task_type'], 'repo': sample['repo']}
    question, row['questionTrimmed'] = question_of(sample)
    gold = baseline.target_gold_files(sample)
    started = time.monotonic()
    module = replay.profiler()
    with tempfile.TemporaryDirectory(prefix='oko-arb-') as temp:
        workspace, cache = Path(temp) / 'workspace', Path(temp) / 'cache'
        try:
            row['files'] = build_workspace(sample, workspace)
            # The client drops inherited OKO_ settings; experiment switches go in through `env`.
            command = ['env', *extra_env, str(binary), 'mcp', *([] if live else ['--no-jev']), '--root', str(workspace)]
            client = module.Client(Path(binary), workspace, cache, timeout, live=live,
                                   api_key=replay.api_key() if live else None,
                                   model=replay.JEV_MODEL if live else None, command=command if extra_env else None)
            try:
                client.initialize()
                response = client.request('tools/call', {'name': 'search', 'arguments': {
                    'question': question, 'intent': intent}})
                packet = replay.packet_of(client, response)
            finally:
                client.close()
            chunks, shown = ranked(packet, workspace)
            retrieval = packet.get('retrieval') or {}
            scores = baseline.sample_metrics(gold, chunks, BUDGET, baseline.hard_negative_files(sample))
            paths = baseline.unique_ranked_paths(chunks)
            row.update(metrics=scores, gold=gold, ranking=packet.get('ranking'), shown=shown,
                       rankedFiles=len(paths), top=paths[:5],
                       # Enough to re-order the list offline without another paid run.
                       shownSpans=[[e['path'], e['startLine'], e['endLine']] for kind in ('results', 'related')
                                   for e in packet.get(kind) or []],
                       candidates=retrieval.get('candidates') or [],
                       # A miss is either never shortlisted, or shortlisted and ranked low.
                       goldShortlisted=sum(path in paths for path in gold) / len(gold) if gold else None,
                       okoMs=(packet.get('timings') or {}).get('totalMs'), jevMs=retrieval.get('rerankMs'))
        except Exception as error:  # A failed sample is a result, not a reason to stop.
            row['error'] = f'{type(error).__name__}: {error}'[:300]
    row['seconds'] = round(time.monotonic() - started, 1)
    return row


HEADLINE = ('Recall@5', 'Recall@10', 'Recall@20', 'MRR', 'gold_coverage@8k')


def summarize(rows):
    out = {}
    groups = {'all': rows, **{task: [r for r in rows if r['task'] == task] for task in TASKS}}
    for name, group in groups.items():
        scored = [r for r in group if 'error' not in r]
        if not group:
            continue
        # An errored sample scores zero: it returned nothing a developer could use.
        out[name] = {'samples': len(group), 'errors': len(group) - len(scored),
                     **{key: round(sum(r['metrics'][key] for r in scored) / len(group), 3) for key in HEADLINE},
                     'goldShortlisted': round(sum(r['goldShortlisted'] or 0 for r in scored) / len(group), 3),
                     'medianRankedFiles': sorted(r['rankedFiles'] for r in scored)[len(scored) // 2] if scored else None}
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--fetch', action='store_true', help='Download data and evaluator; no model calls')
    parser.add_argument('--split', choices=('dev', 'heldout', 'all'), default='dev',
                        help='all = both halves in one pass; it uses up the held-out half')
    parser.add_argument('--tasks', default=','.join(TASKS), help='Comma-separated: ' + ','.join(TASKS))
    parser.add_argument('--limit', type=int, default=0, help='Samples per task')
    parser.add_argument('--jev', action='store_true', help='Rank with Jev: paid calls')
    parser.add_argument('--intent', choices=('implementation', 'explanation', 'general'), default='implementation')
    parser.add_argument('--binary', default=str(replay.PROJECT / 'target/release/oko'))
    parser.add_argument('--label', default=None)
    parser.add_argument('--env', action='append', default=[], metavar='KEY=VALUE', help='Extra environment for Oko, e.g. an experiment switch')
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.fetch:
        return fetch()
    tasks = args.tasks.split(',')
    if any(task not in TASKS for task in tasks):
        parser.error('Unknown task')
    if not EVALUATOR.exists() or not all((DATA / 'benchmark' / f'v2_{t}' / 'samples.jsonl').exists() for t in tasks):
        parser.error('Run --fetch first')
    if args.split in ('heldout', 'all'):
        print('HELD-OUT split: run this only for a number you will report.', file=sys.stderr)
    if shutil.disk_usage(STATE).free < 5e9:
        parser.error('Less than 5 GB free')
    baseline = metrics_module()
    samples = load(None if args.split == 'all' else args.split, args.limit, tasks)
    label = args.label or ('jev' if args.jev else 'keywords')
    out = STATE / f'{args.split}-{label}-{time.strftime("%Y%m%d-%H%M%S")}.jsonl'
    rows = []
    with out.open('w') as handle:
        for index, sample in enumerate(samples, 1):
            row = run(sample, args.binary, args.jev, args.timeout, args.intent, baseline, tuple(args.env))
            rows.append(row)
            handle.write(json.dumps(row) + '\n')
            handle.flush()
            mark = row.get('error') or f"R@5={row['metrics']['Recall@5']:.2f} MRR={row['metrics']['MRR']:.2f}"
            print(f"{index}/{len(samples)} {sample['task_type']} {sample['repo']} {row['seconds']}s {mark}", flush=True)
    summary = {'split': args.split, 'label': label, 'intent': args.intent, 'binary': args.binary, 'env': args.env,
               'evaluator': EVALUATOR_COMMIT, 'results': summarize(rows)}
    out.with_suffix('.summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary['results'], indent=2))
    print(out)


if __name__ == '__main__':
    main()
