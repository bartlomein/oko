#!/usr/bin/env python3
"""Score Oko's retrieval on SWE-Explore (arXiv 2606.07297, SWE-Explore-Bench/SWE-Explore-Bench).

Each instance is a real issue and a repository at a fixed commit. The answer is
the code successful agents actually read while fixing it, as line regions. An
explorer returns five ranked regions; the benchmark's own scorer (pinned to a
commit) computes line-level precision and recall, file and region hits, and
ranking at line budgets. No agent sessions.

  swe_explore.py --fetch                      # benchmark, scorer, issue text and commits; free
  swe_explore.py --mirror                     # bare mirrors of the 203 repositories; free, long
  swe_explore.py --limit 20                   # keyword ranking only, free
  swe_explore.py --jev                        # with Jev: paid calls
  swe_explore.py --explorer bm25              # the benchmark's own baseline on the same inputs

Everything lands under benchmarks/results/swe-explore, which Git ignores. The
benchmark data is CC BY-NC-ND: run and report, do not redistribute.
"""
import argparse
import hashlib
import importlib.util
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

STATE = replay.PROJECT / 'benchmarks/results/swe-explore'
BENCH = STATE / 'bench.final.public.jsonl'
JOINED = STATE / 'instances.jsonl'
EVALUATOR = STATE / 'evaluator'
EVALUATOR_REPO = 'https://github.com/Qiushao-E/SWE-Explore-Bench'
# Pinned at --fetch time; recorded in evaluator-commit.txt beside the results.
BENCH_URL = 'https://huggingface.co/datasets/SWE-Explore-Bench/SWE-Explore-Bench/resolve/main/bench.final.public.jsonl'
SOURCES = {
    'verified': 'princeton-nlp/SWE-bench_Verified',
    'multilingual': 'SWE-bench/SWE-bench_Multilingual',
    'pro': 'ScaleAI/SWE-bench_Pro',
}
TOP_K = 5
# Oko rejects questions above 4096 bytes; many issues are longer.
QUESTION_BYTES = 4000
METRICS = ('precision', 'recall', 'f1_score', 'hit_file_rate', 'hit_region_rate', 'noise_file_rate',
           'context_efficiency', 'ndcg_at_500', 'first_useful_hit', 'recall_at_500')


def rows_of(hub):
    """Every row of a Hugging Face dataset's test split, through the rows API."""
    url = 'https://datasets-server.huggingface.co/rows?dataset=' + hub.replace('/', '%2F') + '&config=default&split=test'
    rows, offset = [], 0
    while True:
        with urllib.request.urlopen(f'{url}&offset={offset}&length=100', timeout=120) as response:
            page = json.load(response)
        rows += [item['row'] for item in page['rows']]
        offset += len(page['rows'])
        if not page['rows'] or offset >= page['num_rows_total']:
            break
    return rows


def fetch():
    STATE.mkdir(parents=True, exist_ok=True)
    if not BENCH.exists():
        urllib.request.urlretrieve(BENCH_URL, BENCH)
    if not EVALUATOR.exists():
        subprocess.run(['git', 'clone', '--quiet', EVALUATOR_REPO, str(EVALUATOR)], check=True)
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=EVALUATOR, text=True).strip()
    (STATE / 'evaluator-commit.txt').write_text(commit + '\n')
    bench = [json.loads(line) for line in BENCH.read_text().splitlines()]
    # The benchmark file has no issue text or commit; they come from its sources.
    lookup = {}
    for name, hub in SOURCES.items():
        for row in rows_of(hub):
            key = row['instance_id'].removeprefix('instance_')
            lookup[key] = (name, row['repo'], row['base_commit'], row['problem_statement'])
    joined, missing = [], []
    for item in bench:
        found = lookup.get(item['instance_id'])
        if not found:
            missing.append(item['instance_id'])
            continue
        source, repo, commit, issue = found
        joined.append({'instance_id': item['instance_id'], 'dataset': item['dataset'], 'source': source,
                       'repo': repo, 'base_commit': commit, 'problem_statement': issue})
    JOINED.write_text(''.join(json.dumps(row) + '\n' for row in joined))
    print(f'{len(joined)} of {len(bench)} instances joined; evaluator at {commit[:12]}')
    if missing:
        (STATE / 'missing.json').write_text(json.dumps(missing, indent=1))
        print(f'{len(missing)} without a source row (see missing.json): ' + ', '.join(missing[:5]))


def instances(limit=0, only=None):
    rows = [json.loads(line) for line in JOINED.read_text().splitlines()]
    if only:
        rows = [row for row in rows if row['instance_id'] in only]
    # A stable order that mixes repositories and sources, so a small --limit is not all Django.
    rows.sort(key=lambda row: hashlib.sha256(row['instance_id'].encode()).hexdigest())
    return rows[:limit] if limit else rows


def git(*args, cwd=None):
    return subprocess.run(['git', *args], cwd=cwd, check=True, capture_output=True, text=True).stdout


def mirror(repo):
    """A blob-less bare clone shared by every commit of the repository."""
    path = STATE / 'repos' / (repo.replace('/', '__') + '.git')
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        git('clone', '--quiet', '--bare', '--filter=blob:none', f'https://github.com/{repo}.git', str(path))
    return path


def mirror_all(rows):
    repos = sorted({row['repo'] for row in rows})
    for index, repo in enumerate(repos, 1):
        started = time.monotonic()
        try:
            mirror(repo)
            print(f'{index}/{len(repos)} {repo} {time.monotonic() - started:.0f}s', flush=True)
        except subprocess.CalledProcessError as error:
            print(f'{index}/{len(repos)} {repo} FAILED: {error.stderr[-200:]}', flush=True)


def checkout(row, target):
    source = mirror(row['repo'])
    try:
        git('cat-file', '-e', row['base_commit'] + '^{commit}', cwd=source)
    except subprocess.CalledProcessError:
        git('fetch', '--quiet', 'origin', row['base_commit'], cwd=source)
    git('worktree', 'add', '--quiet', '--detach', '--force', str(target), row['base_commit'], cwd=source)


def release(row, target):
    source = STATE / 'repos' / (row['repo'].replace('/', '__') + '.git')
    subprocess.run(['git', 'worktree', 'remove', '--force', str(target)], cwd=source, capture_output=True)
    shutil.rmtree(target, ignore_errors=True)
    subprocess.run(['git', 'worktree', 'prune'], cwd=source, capture_output=True)


def scorer():
    spec = importlib.util.spec_from_file_location('swe_explore_eval', EVALUATOR / 'eval.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.ExploreEvaluator(BENCH)


def gt_paths(gt):
    paths = set(gt.get('read_core_files') or []) | set(gt.get('modified_core_files') or []) | set(gt.get('main_files') or [])
    for region in gt.get('read_core_regions') or []:
        paths.add(region['path'])
    for regions in (gt.get('read_optional_regions_map') or {}).values():
        for region in regions:
            paths.add(region['path'])
    for files in (gt.get('read_optional_files_map') or {}).values():
        paths.update(files)
    return paths


def line_counts(workspace, gt):
    """The scorer resolves `end = -1` and negative starts from these."""
    counts = {}
    for rel in gt_paths(gt):
        path = workspace / rel
        if path.is_file():
            try:
                counts[rel] = len(path.read_text(errors='ignore').splitlines())
            except OSError:
                pass
    return counts


def oko_regions(packet, top_k):
    """Oko's excerpts first, then every judged candidate by score, as regions."""
    retrieval = packet.get('retrieval') or {}
    shown = [(e['path'], e['startLine'], e['endLine']) for kind in ('results', 'related')
             for e in packet.get(kind) or []]
    rest = sorted(retrieval.get('candidates') or [], key=lambda c: -(c.get('score') or 0))
    regions = []
    for span in shown + [(c['path'], c['startLine'], c['endLine']) for c in rest]:
        if span not in regions:
            regions.append(span)
    return regions[:top_k]


def run_oko(row, workspace, binary, live, timeout, intent):
    question = row['problem_statement'].encode()[:QUESTION_BYTES].decode(errors='ignore')
    module = replay.profiler()
    with tempfile.TemporaryDirectory(prefix='oko-swex-cache-') as cache:
        client = module.Client(Path(binary), workspace, Path(cache), timeout, live=live,
                               api_key=replay.api_key() if live else None,
                               model=replay.JEV_MODEL if live else None)
        try:
            client.initialize()
            response = client.request('tools/call', {'name': 'search', 'arguments': {
                'question': question, 'intent': intent}})
            packet = replay.packet_of(client, response)
        finally:
            client.close()
    return oko_regions(packet, TOP_K), packet


def run_baseline(name, workspace, row):
    """The benchmark's own local explorers, on the same checkout and issue."""
    sys.path.insert(0, str(EVALUATOR))
    # The explorers import a CLI framework for their own command line; not needed here.
    if 'typer' not in sys.modules:
        stub = type(sys)('typer')
        stub.Typer = lambda **kw: type('T', (), {'command': lambda self, *a, **k: (lambda f: f)})()
        stub.Argument = stub.Option = lambda *a, **k: None
        stub.echo = print
        sys.modules['typer'] = stub
    if name == 'bm25':
        from explorers.bm25 import BM25Explorer as Explorer
    else:
        from explorers.rag_tfidf import TFIDFExplorer as Explorer
    explorer = Explorer(workspace)
    results = explorer.explore(instance_id=row['instance_id'], query=row['problem_statement'], top_k=TOP_K)
    return [(r.path, r.start, r.end) for res in results for r in res.regions][:TOP_K], None


def run(row, args, evaluator):
    out = {key: row[key] for key in ('instance_id', 'dataset', 'repo')}
    started = time.monotonic()
    workspace = STATE / 'work' / row['instance_id']
    try:
        checkout(row, workspace)
        gt = evaluator.bench_data_dict[row['instance_id']]['ground_truth']
        if args.explorer == 'oko':
            regions, packet = run_oko(row, workspace, args.binary, args.jev, args.timeout, args.intent)
            retrieval = (packet.get('retrieval') or {})
            out.update(ranking=packet.get('ranking'), okoMs=(packet.get('timings') or {}).get('totalMs'),
                       jevMs=retrieval.get('rerankMs'), shown=len(packet.get('results') or []))
        else:
            regions, _ = run_baseline(args.explorer, workspace, row)
        evaluator._current_instance_id = row['instance_id']
        evaluator._current_file_line_counts = line_counts(workspace, gt)
        out['regions'] = [{'path': p, 'start': s, 'end': e} for p, s, e in regions]
        out['metrics'] = {m: getattr(evaluator, f'evaluate_{m}')(regions, gt) for m in METRICS}
    except Exception as error:  # A failed instance is a result, not a reason to stop.
        out['error'] = f'{type(error).__name__}: {error}'[:300]
    finally:
        release(row, workspace)
    out['seconds'] = round(time.monotonic() - started, 1)
    return out


def summarize(rows):
    out = {}
    groups = {'all': rows, **{d: [r for r in rows if r['dataset'] == d] for d in ('verified', 'pro', 'multilingual')}}
    for name, group in groups.items():
        if not group:
            continue
        scored = [r for r in group if 'error' not in r]
        # An errored instance scores zero: it returned nothing.
        out[name] = {'instances': len(group), 'errors': len(group) - len(scored),
                     **{m: round(sum(r['metrics'][m] for r in scored) / len(group), 3) for m in METRICS}}
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--fetch', action='store_true')
    parser.add_argument('--mirror', action='store_true', help='Clone every repository mirror now (otherwise on demand)')
    parser.add_argument('--explorer', choices=('oko', 'bm25', 'tfidf'), default='oko')
    parser.add_argument('--limit', type=int, default=0)
    parser.add_argument('--instances', help='Comma-separated instance ids')
    parser.add_argument('--jev', action='store_true', help='Rank with Jev: paid calls')
    parser.add_argument('--intent', choices=('implementation', 'explanation', 'general'), default='implementation')
    parser.add_argument('--binary', default=str(replay.PROJECT / 'target/release/oko'))
    parser.add_argument('--label', default=None)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.fetch:
        return fetch()
    if not JOINED.exists():
        parser.error('Run --fetch first')
    rows = instances(args.limit, set(args.instances.split(',')) if args.instances else None)
    if args.mirror:
        return mirror_all(instances())
    if shutil.disk_usage(STATE).free < 10e9:
        parser.error('Less than 10 GB free')
    evaluator = scorer()
    label = args.label or (args.explorer if args.explorer != 'oko' else ('jev' if args.jev else 'keywords'))
    out = STATE / f'{label}-{time.strftime("%Y%m%d-%H%M%S")}.jsonl'
    results = []
    with out.open('w') as handle:
        for index, row in enumerate(rows, 1):
            result = run(row, args, evaluator)
            results.append(result)
            handle.write(json.dumps(result) + '\n')
            handle.flush()
            mark = result.get('error') or f"file={result['metrics']['hit_file_rate']:.2f} region={result['metrics']['hit_region_rate']:.2f} recall={result['metrics']['recall']:.2f}"
            print(f"{index}/{len(rows)} {row['dataset']} {row['instance_id']} {result['seconds']}s {mark}", flush=True)
    summary = {'label': label, 'explorer': args.explorer, 'intent': args.intent, 'binary': args.binary,
               'evaluator': (STATE / 'evaluator-commit.txt').read_text().strip(), 'results': summarize(results)}
    out.with_suffix('.summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary['results'], indent=2))
    print(out)


if __name__ == '__main__':
    main()
