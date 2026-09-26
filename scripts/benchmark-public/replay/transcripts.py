#!/usr/bin/env python3
"""Replay the Oko calls of recorded agent transcripts through one server session.

Each Sense-bench run leaves a Claude Code transcript. This takes the Oko
`search` calls of each transcript, in order, sends them to one fresh Oko MCP
process (one process per transcript, as the agent had), and records the bytes
each answer returns. Comparing two builds on the same transcripts shows how
much text a change saves over a whole session, without paying for a run.

  transcripts.py --binary /tmp/oko-x --repo discourse \\
      --runs benchmarks/results/sense-bench/results-run{4,5,6,7}/oko-dev/discourse \\
      --label memory [--gold]

`--gold` also counts, per transcript, how many of the Sense bench's discourse
dependents and specs appear as `path:line` anywhere in the session's answers.
"""
import argparse
import json
import re
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402
import replay  # noqa: E402

GOLD = ['lib/cooked_post_processor.rb', 'lib/pretty_text.rb', 'app/models/topic_link.rb', 'file_store/s3_store.rb',
        'lib/s3_inventory.rb', 'pull_hotlinked_images.rb', 'regular/convert_video.rb', 'lib/email/styles.rb',
        'backup_restore/creator.rb', 'lib/site_icon_manager.rb', 'app/helpers/application_helper.rb',
        'app/controllers/metadata_controller.rb', 'app/models/user_profile.rb', 'services/user_merger.rb',
        'lib/plugin/instance.rb', 'lib/site_setting_extension.rb', 'spec/models/upload_spec.rb',
        'spec/lib/cooked_post_processor_spec.rb', 'spec/requests/uploads_controller_spec.rb']


def calls_of(transcript):
    """The Oko search inputs of one transcript, in order."""
    calls = []
    for line in Path(transcript).read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        for block in (event.get('message') or {}).get('content') or []:
            if isinstance(block, dict) and block.get('type') == 'tool_use' and 'oko' in (block.get('name') or ''):
                arguments = dict(block.get('input') or {})
                # The bench mounts repositories at /repos/<name>; the replay root is the checkout.
                if isinstance(arguments.get('directory'), str):
                    arguments['directory'] = re.sub(r'^/repos/[^/]+/?', '', arguments['directory']) or '.'
                calls.append(arguments)
    return calls


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--repo', required=True)
    parser.add_argument('--runs', nargs='+', required=True, help='Directories holding run-N/transcript.json')
    parser.add_argument('--label', required=True)
    parser.add_argument('--gold', action='store_true')
    parser.add_argument('--timeout', type=float, default=300)
    args = parser.parse_args()
    module = replay.profiler()
    rows = []
    for directory in args.runs:
        for transcript in sorted(Path(directory).glob('run-*/transcript.json')):
            calls = calls_of(transcript)
            if not calls:
                continue
            sizes, errors, seen_text = [], 0, ''
            started = time.monotonic()
            with tempfile.TemporaryDirectory(prefix='oko-transcripts-') as cache:
                client = module.Client(Path(args.binary), corpus.DEFAULT_REPOS[args.repo], Path(cache), args.timeout,
                                       live=False, api_key=None, model=None)
                client.initialize()
                for arguments in calls:
                    try:
                        response = client.request('tools/call', {'name': 'search', 'arguments': arguments})
                        text = ''.join(b.get('text', '') for b in response.get('content', []))
                        errors += bool(response.get('isError'))
                    except Exception as error:  # A failed call is a result, not a reason to stop.
                        text, errors = f'ERROR {error}', errors + 1
                    sizes.append(len(text))
                    seen_text += text + '\n'
                client.close()
            row = {'transcript': str(transcript), 'calls': len(calls), 'bytes': sum(sizes), 'errors': errors,
                   'maxBytes': max(sizes), 'seconds': round(time.monotonic() - started, 1)}
            if args.gold:
                row['goldCited'] = sum(re.search(re.escape(g) + r':\d+', seen_text) is not None for g in GOLD)
            rows.append(row)
            print(f"  {transcript.parent.parent.parent.parent.name}/{transcript.parent.name}: {row['calls']} calls, "
                  f"{row['bytes'] // 1000} kB, max {row['maxBytes'] // 1000} kB, errors {errors}"
                  + (f", gold {row['goldCited']}/19" if args.gold else ''), flush=True)
    total = sum(r['bytes'] for r in rows)
    summary = {'label': args.label, 'transcripts': len(rows), 'calls': sum(r['calls'] for r in rows),
               'kB': total // 1000, 'kBPerTranscript': round(total / max(1, len(rows)) / 1000, 1),
               'errors': sum(r['errors'] for r in rows)}
    if args.gold:
        summary['goldCitedMean'] = round(sum(r['goldCited'] for r in rows) / max(1, len(rows)), 2)
    out = replay.PROJECT / 'benchmarks/results/transcripts'
    out.mkdir(parents=True, exist_ok=True)
    path = out / f'{args.label}-{args.repo}-{time.strftime("%Y%m%d-%H%M%S")}.json'
    path.write_text(json.dumps({'summary': summary, 'rows': rows}, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print(path)


if __name__ == '__main__':
    main()
