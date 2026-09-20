#!/usr/bin/env python3
"""Replay the questions agents really asked Oko and score what each build returns.

Agent sessions are fast only when one Oko call returns all the code a task needs;
otherwise the agent keeps searching and the session is no better than native
search. This measures that directly, per build, in minutes and without agent
sessions: the share of each task's expected locations covered by one response,
plus response size and Oko/Jev time.

It cannot show how an agent reacts (turns, trust, final answers). Use the smoke
or branch suite for that. Jev is not deterministic, so each build's numbers carry
some noise; compare means over many questions, not single rows.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import statistics
import sys
import tarfile
import tempfile

# Kept in a subfolder: the runner fingerprints every script beside it, so a new
# file there would invalidate frozen suites and block resuming a paid run.
ROOT = Path(__file__).resolve().parents[1]
PROJECT = ROOT.parents[1]
FIXTURE = ROOT / 'tasks-branch.json'
BRANCH_STATE = PROJECT / 'benchmarks/results/public-branch'
STATE = PROJECT / 'benchmarks/results/public-replay'
JEV_MODEL = 'jev-1.13.0'


def load_tasks(fixture=FIXTURE):
    """Task id -> (repository name, task), with every task's expected locations."""
    tasks = {}
    for item in json.loads(Path(fixture).read_text())['repositories']:
        for task in item['tasks']:
            tasks[task['id']] = (item['name'], task)
    return tasks


def anchors(task):
    """The code one response should contain: search anchors, or the lines an edit must change."""
    if task['kind'] == 'search':
        return [(e['path'], e['startLine'], e['endLine']) for e in task['expected']]
    ranges = task.get('allowedRanges') or [task['allowedLines']]
    return [(task['path'], start, end) for start, end in ranges]


def tool_arguments(tool):
    """Oko arguments as recorded by Codex (arguments), Claude (input), or OpenCode (state.input)."""
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
    return value if isinstance(value, dict) and isinstance(value.get('question'), str) else None


def portable_directory(directory):
    """Agents sometimes pass the session's absolute workspace path; keep only the part inside it."""
    if not directory or directory in ('.', './'):
        return None
    if os.path.isabs(directory):
        marker = '/workspace'
        if marker not in directory:
            return None
        directory = directory.split(marker, 1)[1].strip('/')
    return directory or None


def collect_questions(reports, tasks):
    """Distinct (question, intent, directory) per task, in first-seen order.

    Deep searches are replayed as normal ones: one Jev call per question keeps
    the cost known, and the normal path is what these builds change.
    """
    found = {task: {} for task in tasks}
    for report in reports:
        for run in json.loads(Path(report).read_text())['runs']:
            if run.get('id') not in found:
                continue
            for tool in run.get('tools', []):
                args = tool_arguments(tool)
                if args is None:
                    continue
                query = {'question': args['question']}
                if args.get('intent'):
                    query['intent'] = args['intent']
                directory = portable_directory(args.get('directory'))
                if directory:
                    query['directory'] = directory
                found[run['id']].setdefault(json.dumps(query, sort_keys=True), query)
    return {task: list(queries.values()) for task, queries in found.items()}


def spans(packet):
    return [(e['path'], e['startLine'], e['endLine'], bool(e.get('truncated')))
            for key in ('results', 'related') for e in packet.get(key) or []]


# Runners-up below this relevance are judged irrelevant and are not named to the agent.
LISTED_FLOOR = 0.2


def score(packet, expected, directory=None):
    """How much of the expected code one response contains, and where the rest went.

    Builds that record every candidate's judgment let each missed location be
    placed: `listed` (named to the agent by path as a lower-rated candidate),
    `rejected` (shortlisted but judged irrelevant), or `notShortlisted` (keyword
    search never offered it to the ranker). Each needs a different fix.
    """
    prefix = (directory.strip('/') + '/') if directory and directory not in ('.', './') else ''
    returned = [(prefix + path, start, end, partial) for path, start, end, partial in spans(packet)]
    covered = [any(path == p and start <= s and end >= e for p, start, end, _ in returned)
               for path, s, e in expected]
    result = {'covered': sum(covered), 'expected': len(expected), 'full': all(covered),
              'excerpts': len(returned), 'lines': sum(end - start + 1 for _, start, end, _ in returned),
              'results': len(packet.get('results') or [])}
    candidates = (packet.get('retrieval') or {}).get('candidates')
    if candidates is not None:
        missed = {'listed': 0, 'rejected': 0, 'notShortlisted': 0}
        for (path, start, end), hit in zip(expected, covered):
            if hit:
                continue
            # A candidate is a chunk; overlapping the location is enough to lead the agent there.
            scores = [c.get('score') for c in candidates
                      if prefix + c['path'] == path and c['startLine'] <= end and start <= c['endLine']]
            if not scores:
                missed['notShortlisted'] += 1
            elif any(s is None or s >= LISTED_FLOOR for s in scores):
                missed['listed'] += 1
            else:
                missed['rejected'] += 1
        result['missed'] = missed
    return result


def profiler():
    spec = importlib.util.spec_from_file_location('cache_profiler', ROOT.parent / 'profile-cache.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def api_key():
    """The environment, then the project's .env; otherwise Oko uses its saved credential."""
    if os.environ.get('TYPESAFE_API_KEY'):
        return os.environ['TYPESAFE_API_KEY']
    path = PROJECT / '.env'
    if path.exists():
        for line in path.read_text().splitlines():
            match = re.match(r'^\s*(?:export\s+)?TYPESAFE_API_KEY\s*=\s*(.*)$', line)
            if match:
                value = match.group(1).strip()
                if value[:1] in ('"', "'", '`'):
                    return value[1:value.find(value[0], 1)]
                return value.split('#', 1)[0].strip()
    return None


def packet_of(client, response):
    """Older builds return the packet as structured content; newer ones record it by file."""
    if response.get('isError'):
        text = ''.join(block.get('text', '') for block in response.get('content', []))
        structured = response.get('structuredContent') or {}
        raise RuntimeError(structured.get('error') or text or 'search failed')
    if 'structuredContent' in response:
        return response['structuredContent']
    return client.last_metrics()


def replay_build(label, binary, workspaces, questions, tasks, live, timeout, progress):
    module = profiler()
    rows = []
    for repo, workspace in workspaces.items():
        selected = [(task, query) for task, queries in questions.items()
                    if tasks[task][0] == repo for query in queries]
        if not selected:
            continue
        with tempfile.TemporaryDirectory(prefix='oko-replay-cache-') as cache:
            client = module.Client(Path(binary), workspace, Path(cache), timeout, live=live,
                                   api_key=api_key() if live else None,
                                   model=JEV_MODEL if live else None)
            try:
                client.initialize()
                for index, (task, query) in enumerate(selected):
                    row = {'build': label, 'repository': repo, 'task': task,
                           'kind': tasks[task][1]['kind'], **query}
                    try:
                        response = client.request('tools/call', {'name': 'search', 'arguments': query})
                        packet = packet_of(client, response)
                        retrieval = packet.get('retrieval') or {}
                        timings = packet.get('timings') or {}
                        text = ''.join(b.get('text', '') for b in response.get('content', []))
                        row.update(score(packet, anchors(tasks[task][1]), query.get('directory')),
                                   ranking=packet.get('ranking'),
                                   # What the agent pays for: every copy the client forwards.
                                   responseBytes=len(json.dumps({k: v for k, v in response.items()
                                                                 if k in ('content', 'structuredContent')})),
                                   textBytes=len(text),
                                   okoMs=timings.get('totalMs'),
                                   # The first search in a process also builds the index.
                                   cacheMs=(timings.get('cache') or {}).get('totalMs'),
                                   firstInProcess=index == 0,
                                   jevMs=retrieval.get('rerankMs'),
                                   jevCalls=len(retrieval.get('jevCalls') or []),
                                   jevInputTokens=sum((call.get('usage') or {}).get('inputTokens') or 0
                                                      for call in retrieval.get('jevCalls') or []))
                    except Exception as error:  # A failed query is a result, not a reason to stop.
                        row['error'] = f'{type(error).__name__}: {error}'[:300]
                    rows.append(row)
                    progress(row)
            finally:
                client.close()
    return rows


def mean(values):
    values = [v for v in values if v is not None]
    return statistics.mean(values) if values else None


def median(values):
    values = [v for v in values if v is not None]
    return statistics.median(values) if values else None


def summarize(rows, builds):
    """Per build and per task; then the same questions compared between builds."""
    def block(selected):
        good = [r for r in selected if 'error' not in r]
        return {'questions': len(selected), 'errors': len(selected) - len(good),
                'anchorsCovered': mean([r['covered'] / r['expected'] for r in good]),
                'fullyCovered': mean([float(r['full']) for r in good]),
                'empty': mean([float(r['excerpts'] == 0) for r in good]),
                'excerpts': mean([r['excerpts'] for r in good]),
                'lines': mean([r['lines'] for r in good]),
                'responseBytes': mean([r['responseBytes'] for r in good]),
                'okoLocalMs': median([r['okoMs'] - (r['jevMs'] or 0) for r in good
                                      if r.get('okoMs') is not None and not r['firstInProcess']]),
                'jevMs': median([r['jevMs'] for r in good]),
                'fallbacks': sum(r.get('ranking') == 'lexical-fallback' for r in good),
                'missed': ({key: sum(r['missed'][key] for r in good if 'missed' in r)
                            for key in ('listed', 'rejected', 'notShortlisted')}
                           if any('missed' in r for r in good) else None)}
    summary = {'builds': {}, 'tasks': {}, 'paired': {}}
    for build in builds:
        mine = [r for r in rows if r['build'] == build]
        summary['builds'][build] = block(mine)
        for task in dict.fromkeys(r['task'] for r in mine):
            summary['tasks'].setdefault(task, {})[build] = block([r for r in mine if r['task'] == task])
    first = builds[0]
    key = lambda r: (r['task'], r['question'], r.get('intent'), r.get('directory'))
    base = {key(r): r for r in rows if r['build'] == first and 'error' not in r}
    for build in builds[1:]:
        pairs = [(base[key(r)], r) for r in rows if r['build'] == build and 'error' not in r and key(r) in base]
        summary['paired'][build] = {
            'baseline': first, 'pairs': len(pairs),
            'gainedFullCoverage': sum(b['full'] and not a['full'] for a, b in pairs),
            'lostFullCoverage': sum(a['full'] and not b['full'] for a, b in pairs),
            'moreAnchors': sum(b['covered'] > a['covered'] for a, b in pairs),
            'fewerAnchors': sum(b['covered'] < a['covered'] for a, b in pairs)}
    return summary


def render(summary, builds, note):
    def pct(value):
        return 'n/a' if value is None else f'{100 * value:.0f}%'

    def num(value, digits=0):
        return 'n/a' if value is None else f'{value:.{digits}f}'
    lines = ['# Oko retrieval replay', '', note, '',
             '| Build | Questions | Expected code covered | Responses covering everything | Empty | Excerpts | Lines | Response bytes | Oko local ms | Jev ms |',
             '|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|']
    for build in builds:
        b = summary['builds'][build]
        lines.append(f"| {build} | {b['questions']} | {pct(b['anchorsCovered'])} | {pct(b['fullyCovered'])} | {pct(b['empty'])} | "
                     f"{num(b['excerpts'], 1)} | {num(b['lines'])} | {num(b['responseBytes'])} | {num(b['okoLocalMs'])} | {num(b['jevMs'])} |")
    lines += ['', '"Expected code covered" is the mean share of a task\'s expected locations inside one response. '
              '"Covering everything" is the share of responses after which the agent needs no further search. '
              'Oko local time excludes Jev and each process\'s first, index-building search.', '',
              '## Per task', '',
              '| Task | ' + ' | '.join(f'{b}: covered / everything / lines' for b in builds) + ' |',
              '|---|' + '---:|' * len(builds)]
    for task, per_build in summary['tasks'].items():
        cells = []
        for build in builds:
            b = per_build.get(build)
            cells.append('n/a' if not b else f"{pct(b['anchorsCovered'])} / {pct(b['fullyCovered'])} / {num(b['lines'])}")
        lines.append(f'| {task} | ' + ' | '.join(cells) + ' |')
    placed = {build: summary['builds'][build]['missed'] for build in builds if summary['builds'][build]['missed']}
    if placed:
        lines += ['', '## Where the missed code went', '',
                  '| Build | Named as a lower-rated candidate | Shortlisted, judged irrelevant | Never shortlisted |',
                  '|---|---:|---:|---:|']
        for build, missed in placed.items():
            lines.append(f"| {build} | {missed['listed']} | {missed['rejected']} | {missed['notShortlisted']} |")
        lines += ['', 'Counts are expected locations missing from the excerpts. "Named" ones cost the agent one read; '
                  '"judged irrelevant" points at the ranker or the question; "never shortlisted" points at keyword retrieval. '
                  'Only builds that record every candidate appear here.']
    if summary['paired']:
        lines += ['', '## Same question, different build', '',
                  '| Build | Baseline | Pairs | Gained full coverage | Lost full coverage | More anchors | Fewer anchors |',
                  '|---|---|---:|---:|---:|---:|---:|']
        for build, p in summary['paired'].items():
            lines.append(f"| {build} | {p['baseline']} | {p['pairs']} | {p['gainedFullCoverage']} | {p['lostFullCoverage']} | {p['moreAnchors']} | {p['fewerAnchors']} |")
        lines += ['', 'Jev is not deterministic: some gains and losses occur between two runs of the same build. '
                  'Read the difference between gained and lost, not either number alone.']
    errors = sum(b['errors'] for b in summary['builds'].values())
    fallbacks = sum(b['fallbacks'] for b in summary['builds'].values())
    if errors or fallbacks:
        lines += ['', f'Failed queries: {errors}. Keyword fallbacks (Jev slow or unavailable): {fallbacks}. Both are excluded from or distort coverage; inspect replay.json.']
    return '\n'.join(lines) + '\n'


def default_builds():
    """The builds frozen by the branch or smoke suite's last --prepare."""
    for folder in (BRANCH_STATE, PROJECT / 'benchmarks/results/public-smoke'):
        for settings in sorted(folder.glob('*/settings.json'), key=lambda p: p.stat().st_mtime, reverse=True):
            builds = json.loads(settings.read_text()).get('builds')
            if builds:
                return [(label, build['path']) for label, build in builds.items()]
    return []


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--build', action='append', metavar='LABEL=PATH',
                        help='Oko binary to replay; repeat to compare. The first is the baseline. '
                             'Default: the builds frozen by the last branch/smoke --prepare.')
    parser.add_argument('--report', action='append', type=Path,
                        help='report.json to take questions from; default: every branch-suite run')
    parser.add_argument('--tasks', help='Comma-separated task ids; default all')
    parser.add_argument('--limit', type=int, help='At most this many questions per task')
    parser.add_argument('--no-jev', action='store_true',
                        help='Free and offline: keyword ranking only. Checks packaging, not relevance.')
    parser.add_argument('--execute', action='store_true', help='Make the paid Jev calls')
    parser.add_argument('--timeout', type=float, default=60)
    args = parser.parse_args(argv)

    builds = [tuple(item.split('=', 1)) for item in args.build] if args.build else default_builds()
    if not builds or any(len(b) != 2 or not Path(b[1]).is_file() for b in builds):
        parser.error('No usable builds; pass --build LABEL=PATH or prepare the branch/smoke suite')
    if len({label for label, _ in builds}) != len(builds):
        parser.error('Build labels must differ')
    tasks = load_tasks()
    if args.tasks:
        wanted = args.tasks.split(',')
        if not set(wanted) <= set(tasks):
            parser.error('Unknown task; choose from: ' + ', '.join(tasks))
        tasks = {task: tasks[task] for task in wanted}
    reports = args.report or sorted(BRANCH_STATE.glob('results-*/report.json'))
    if not reports:
        parser.error('No branch-suite report.json found; pass --report')
    questions = collect_questions(reports, tasks)
    if args.limit:
        questions = {task: queries[:args.limit] for task, queries in questions.items()}
    total = sum(len(q) for q in questions.values())
    calls = 0 if args.no_jev else total * len(builds)
    print(f"{total} distinct questions from {len(reports)} report(s); {len(builds)} build(s): "
          + ', '.join(f'{label}' for label, _ in builds))
    for task, queries in questions.items():
        print(f'  {len(queries):3} {task}')
    print(f'Jev calls: about {calls}' + (' (keyword ranking only)' if args.no_jev else
                                         ' (about 11k Jev input tokens each; an empty result adds a recovery call)'))
    if not args.execute and not args.no_jev:
        print('Nothing sent. Add --execute for the paid replay, or --no-jev for a free packaging check.')
        return 0

    STATE.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix='results-', dir=STATE))
    rows = []
    with tempfile.TemporaryDirectory(prefix='oko-replay-') as scratch:
        workspaces = {}
        for repo in dict.fromkeys(tasks[task][0] for task in questions if questions[task]):
            workspace = Path(scratch) / repo
            workspace.mkdir()
            with tarfile.open(BRANCH_STATE / repo / 'baseline.tar') as archive:
                archive.extractall(workspace, filter='data')
            workspaces[repo] = workspace
        done = [0]

        def progress(row):
            done[0] += 1
            state = row.get('error') or f"{row['covered']}/{row['expected']} anchors, {row['lines']} lines"
            print(f"[{done[0]}/{total * len(builds)}] {row['build']} {row['task']}: {state}", flush=True)
        for label, binary in builds:
            rows += replay_build(label, binary, workspaces, questions, tasks, not args.no_jev,
                                 args.timeout, progress)
    labels = [label for label, _ in builds]
    summary = summarize(rows, labels)
    note = (f"{total} real agent questions, replayed against each build"
            + (' with keyword ranking only (--no-jev): this checks packaging, not relevance.' if args.no_jev
               else f' with Jev ({JEV_MODEL}).')
            + ' No agent sessions: this measures what Oko returns, not how an agent reacts.')
    (output / 'replay.json').write_text(json.dumps(
        {'builds': dict(builds), 'reports': [str(r) for r in reports], 'noJev': args.no_jev,
         'summary': summary, 'rows': rows}, indent=2) + '\n')
    (output / 'replay.md').write_text(render(summary, labels, note))
    print('\n' + render(summary, labels, note))
    print(output)
    return 0


if __name__ == '__main__':
    sys.exit(main())
