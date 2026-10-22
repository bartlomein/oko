"""Rescore the saved Oko rankings offline: Recall@k and MRR from top_files, BCY with report_bcy_budget_curve.

No Oko binary, no Jev, no network. Run from the root of an agent-retrieval-bench checkout, after
downloading the v2 positive subsets, with this folder (details files beside this script) anywhere:

    python /path/to/details/rescore.py --corpus-root data/corpus
"""
import argparse
import json
import tempfile
from pathlib import Path

from agent_retrieval_bench.bcy_curve import report_bcy_budget_curve

HERE = Path(__file__).resolve().parent
RUNS = [
    ('Oko 0.9.1 run 1', HERE / 'oko-0.9.1-run1_details.jsonl'),
    ('Oko 0.9.1 run 2', HERE / 'oko-0.9.1-run2_details.jsonl'),
    ('Oko 0.9.1 run 3', HERE / 'oko-0.9.1-run3_details.jsonl'),
    ('Oko 0.5.0 run 1', HERE / 'oko-0.5.0-run1_details.jsonl'),
    ('Oko 0.5.0 run 2', HERE / 'oko-0.5.0-run2_details.jsonl'),
    ('Oko 0.5.0 run 3', HERE / 'oko-0.5.0-run3_details.jsonl'),
]


def rank_metrics(gold, top, ranked):
    out = {f'Recall@{k}': sum(path in top[:k] for path in gold) / len(gold) for k in (5, 10, 20)}
    out['MRR'] = next((1 / rank for rank, path in enumerate(ranked, 1) if path in gold), 0.0)
    return out


def mean(rows, key):
    return sum(row[key] for row in rows) / len(rows)


def corpus_manifest(corpus_root, path):
    """One manifest row per released chunk file, in the shape report_bcy_budget_curve reads."""
    rows = []
    for chunks in sorted(corpus_root.glob('v2_*/*/*.chunks.jsonl')):
        repo = chunks.parent.name.replace('__', '/', 1)
        rows.append({'repo': repo, 'base_commit': chunks.name.removesuffix('.chunks.jsonl'),
                     'chunks_path': str(chunks.resolve()), 'status': 'ok'})
    path.write_text(''.join(json.dumps(row) + '\n' for row in rows))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--corpus-root', type=Path, default=Path('data/corpus'))
    parser.add_argument('--out-dir', type=Path, default=Path('data/reports/community/oko'))
    args = parser.parse_args()

    print('| Run | Split | Samples | Recall@5 | Recall@20 | MRR | Stored metrics match |')
    print('| --- | --- | ---: | ---: | ---: | ---: | --- |')
    for label, path in RUNS:
        details = [json.loads(line) for line in path.read_text().splitlines()]
        assert len({d['sample_id'] for d in details}) == len(details) == 345, path
        for split in ('all', 'dev', 'heldout'):
            rows = [d for d in details if split == 'all' or d['split'] == split]
            scored = [rank_metrics(d['gold_files'], d['top_files'], d['ranked_files']) for d in rows]
            match = all(abs(s[k] - d['metrics'][k]) < 1e-9 for s, d in zip(scored, rows) for k in s)
            print(f"| {label} | {split} | {len(rows)} | {mean(scored, 'Recall@5'):.4f} | "
                  f"{mean(scored, 'Recall@20'):.4f} | {mean(scored, 'MRR'):.4f} | {match} |")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temp:
        manifest = Path(temp) / 'corpus_manifest.jsonl'
        corpus_manifest(args.corpus_root, manifest)
        report = report_bcy_budget_curve(
            corpus_manifest_path=manifest,
            out_path=args.out_dir / 'bcy_budget_curve.json',
            markdown_out_path=args.out_dir / 'bcy_budget_curve.md',
            runs=[(label, 'oko', path) for label, path in RUNS],
        )
    print()
    print('| Run | BCY@4k | BCY@8k | BCY@16k | BCY@32k |')
    print('| --- | ---: | ---: | ---: | ---: |')
    for run in report['runs']:
        overall = run['overall']
        print(f"| {run['label']} | " + ' | '.join(f"{overall[f'BCY@{b}']:.4f}" for b in (4000, 8000, 16000, 32000)) + ' |')


if __name__ == '__main__':
    main()
