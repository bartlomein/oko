#!/usr/bin/env python3
"""Run Claude Code on ContextBench issues, with and without Oko, and score
the trajectories with the benchmark's own evaluator.

ContextBench (arXiv 2602.05892, github.com/EuniAI/ContextBench, Apache-2.0)
scores which files and line ranges an agent looked at while resolving an
issue against human-annotated gold context: coverage, precision and F1 at
file, symbol and line level, how early the gold was reached (AUC-coverage)
and how much was re-read (redundancy). The agent runs on this machine with
the user's own Claude Code login; nothing here needs an API key except Oko's
Jev key for the `oko` condition.

    run.py --fetch                      # instances from the evaluator's parquet, one mirror per repo
    run.py --condition native --limit 10 --label pilot
    run.py --condition oko --limit 10 --label pilot
    run.py --score runs/pilot-native runs/pilot-oko

Per instance: a git worktree at the base commit, one `claude -p` session
with the same tool set the public agent suite allows (Read, Glob, Grep,
Edit, Write; no Bash), events saved as `events.jsonl`, the final diff as
`patch.diff`, the trajectory as `traj.json` (see extract.py), and timings,
tokens and tool counts in `meta.json`. The `oko` condition adds the Oko MCP
server and the guidance `oko setup` installs, exactly as the README suite's
guided condition does.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / 'replay'))
import extract  # noqa: E402
import replay  # noqa: E402  (api_key: the environment, then the project's .env)

PROJECT = HERE.parents[2]
STATE = PROJECT / 'benchmarks/results/contextbench'
EVALUATOR = STATE / 'evaluator'
VENV_PYTHON = STATE / '.venv/bin/python'
INSTANCES = STATE / 'instances.jsonl'
GUIDANCE = PROJECT / 'src/guidance.md'
CLAUDE_TOOLS = 'Read,Glob,Grep,Edit,Write'

PROMPT = """You are working in a checkout of {repo} at the root of the current directory. Resolve the issue below by editing the source files that need to change. Do not install packages or run tests; there is no shell. Read only what you need.

When you are done, end your reply with a block that lists every file and line range that is essential to understanding and making this fix, in exactly this form (paths relative to the repository root, inclusive line numbers as they are now, several ranges separated by commas):

<PATCH_CONTEXT>
File: path/to/file.py
Lines: 10-42, 80-95
File: path/to/other.py
Lines: 1-20
</PATCH_CONTEXT>

Issue:

{issue}
"""


def load_instances(limit=0, only=None):
    rows = [json.loads(line) for line in INSTANCES.read_text().splitlines() if line.strip()]
    if only:
        rows = [row for row in rows if row['instance_id'] in only or row['original_inst_id'] in only]
    # A stable order that mixes repositories and languages, so a small --limit is not all Django.
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


def gold_path(path):
    """Gold paths carry the annotators' container prefix, `/workspace/<owner__repo__x>/`."""
    parts = path.split('/')
    if len(parts) > 2 and parts[1] == 'workspace':
        return '/'.join(parts[3:])
    return path.lstrip('/')


def fetch():
    parquet = EVALUATOR / 'data/contextbench_verified.parquet'
    if not parquet.is_file():
        sys.exit(f'Clone github.com/EuniAI/ContextBench to {EVALUATOR} first')
    code = (
        "import json, sys, pyarrow.parquet as pq\n"
        "def gold_path(path):\n"
        "    parts = path.split('/')\n"
        "    return '/'.join(parts[3:]) if len(parts) > 2 and parts[1] == 'workspace' else path.lstrip('/')\n"
        f"rows = pq.read_table({str(parquet)!r}).to_pylist()\n"
        f"with open({str(INSTANCES)!r}, 'w') as out:\n"
        "    for r in rows:\n"
        "        gc = r['gold_context']; gc = json.loads(gc) if isinstance(gc, str) else gc\n"
        "        out.write(json.dumps({k: r[k] for k in ('instance_id', 'original_inst_id', 'repo', 'repo_url', 'language', 'base_commit', 'source', 'problem_statement')} | {'gold_files': sorted({gold_path(g['file']) for g in gc})}) + '\\n')\n"
        "print(len(rows), 'instances')\n"
    )
    subprocess.run([str(VENV_PYTHON), '-c', code], check=True)
    rows = load_instances()
    repos = sorted({row['repo'] for row in rows})
    for index, repo in enumerate(repos, 1):
        started = time.monotonic()
        try:
            mirror(repo)
            print(f'{index}/{len(repos)} {repo} {time.monotonic() - started:.0f}s', flush=True)
        except subprocess.CalledProcessError as error:
            print(f'{index}/{len(repos)} {repo} FAILED: {error.stderr[-200:]}', flush=True)


def claude_command(work, trial, condition, binary, model, effort):
    enabled = condition == 'oko'
    command = [str(Path(binary).resolve()), 'mcp', '--root', str(work)]
    mcp = {'mcpServers': {'oko': {'type': 'stdio', 'command': command[0], 'args': command[1:]}} if enabled else {}}
    settings = {'disableAllHooks': True, 'autoMemoryEnabled': False, 'claudeMdExcludes': ['/**'],
                'pluginConfigs': {'agents-md@builtin': {'options': {'instructionFiles': 'managed-only'}}},
                'permissions': {'deny': ['Read(**/.env)', 'Read(**/.env.*)']}}
    args = ['claude', '-p', '--output-format', 'stream-json', '--verbose', '--restricted',
            '--tools', CLAUDE_TOOLS, '--allowedTools', CLAUDE_TOOLS + (',mcp__oko__search' if enabled else ''),
            '--permission-mode', 'dontAsk', '--permission-prompts', 'none',
            '--strict-mcp-config', '--mcp-config', json.dumps(mcp),
            '--disable-slash-commands', '--no-session-persistence', '--no-chrome',
            '--effort', effort, '--setting-sources', '', '--settings', json.dumps(settings),
            '--debug-file', str(trial / 'claude-debug.log')]
    if model:
        args += ['--model', model]
    if enabled:
        args += ['--append-system-prompt', GUIDANCE.read_text()]
    env = dict(os.environ)
    env.update({'OKO_CACHE_DIR': str(trial / 'cache'), 'OKO_METRICS_FILE': str(trial / 'oko-metrics.jsonl'),
                'OKO_NO_WATCH': '1'})
    if enabled:
        key = replay.api_key()
        if not key:
            raise RuntimeError('No TYPESAFE_API_KEY in the environment or the project .env')
        env['TYPESAFE_API_KEY'] = key
    return args, env


def usage_of(events):
    """Tokens and tool calls from Claude's stream-json events."""
    tools, oko_searches, usage, cost, turns = 0, 0, {}, None, 0
    for event in events:
        if event.get('type') == 'assistant':
            turns += 1
            for part in (event.get('message') or {}).get('content') or []:
                if isinstance(part, dict) and part.get('type') == 'tool_use':
                    tools += 1
                    oko_searches += part.get('name') == 'mcp__oko__search'
        if event.get('type') == 'result':
            usage = event.get('usage') or {}
            cost = event.get('total_cost_usd')
    total = sum(int(usage.get(k) or 0) for k in ('input_tokens', 'cache_creation_input_tokens', 'cache_read_input_tokens', 'output_tokens'))
    return {'toolCalls': tools, 'okoSearches': oko_searches, 'modelTurns': turns, 'tokens': total, 'usage': usage, 'costUsd': cost}


def run_one(row, args, out_dir):
    trial = out_dir / row['instance_id']
    if (trial / 'traj.json').is_file() and not args.rerun:
        return json.loads((trial / 'meta.json').read_text())
    trial.mkdir(parents=True, exist_ok=True)
    work = STATE / 'work' / row['instance_id']
    meta = {'instance_id': row['instance_id'], 'repo': row['repo'], 'language': row['language'], 'source': row['source'],
            'condition': args.condition, 'model': args.model, 'effort': args.effort}
    started = time.monotonic()
    try:
        checkout(row, work)
        prompt = PROMPT.format(repo=row['repo'], issue=row['problem_statement'][:args.issue_bytes])
        command, env = claude_command(work, trial, args.condition, args.binary, args.model, args.effort)
        with (trial / 'events.jsonl').open('w') as stdout, (trial / 'stderr.txt').open('w') as stderr:
            try:
                completed = subprocess.run(command + [prompt], cwd=work, env=env, stdin=subprocess.DEVNULL,
                                           stdout=stdout, stderr=stderr, timeout=args.timeout)
                meta['exitCode'] = completed.returncode
            except subprocess.TimeoutExpired:
                meta['exitCode'] = 'timeout'
        git('add', '-A', cwd=work)
        patch = git('diff', '--cached', cwd=work)
        (trial / 'patch.diff').write_text(patch)
        events = []
        for line in (trial / 'events.jsonl').read_text().splitlines():
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
        meta.update(usage_of(events))
        traj = extract.extract(trial / 'events.jsonl', work, row['instance_id'], patch)
        (trial / 'traj.json').write_text(json.dumps(traj))
        meta['steps'] = len(traj['traj_data']['pred_steps'])
        meta['declared'] = traj['traj_data']['declared_source']
        meta['filesViewed'] = len({f for s in traj['traj_data']['pred_steps'] for f in s['files']})
        meta['goldFilesViewed'] = len(set(row['gold_files']) & {f for s in traj['traj_data']['pred_steps'] for f in s['files']})
        meta['goldFiles'] = len(row['gold_files'])
    except Exception as error:  # A failed instance is a result, not a reason to stop.
        meta['error'] = f'{type(error).__name__}: {error}'[:300]
    finally:
        release(row, work)
    meta['seconds'] = round(time.monotonic() - started, 1)
    (trial / 'meta.json').write_text(json.dumps(meta, indent=2))
    return meta


def score(run_dirs, out_path=None):
    """Score run directories with the benchmark's evaluator; print a table."""
    for run_dir in run_dirs:
        run_dir = Path(run_dir).resolve()
        pred = run_dir / 'pred.jsonl'
        trajs = sorted(run_dir.glob('*/traj.json'))
        with pred.open('w') as out:
            for traj in trajs:
                out.write(traj.read_text().strip() + '\n')
        results = run_dir / 'results.jsonl'
        subprocess.run([str(VENV_PYTHON), '-m', 'contextbench.evaluate',
                        '--gold', str(EVALUATOR / 'data/contextbench_verified.parquet'),
                        '--pred', str(pred), '--cache', str(STATE / 'repos-eval'), '--out', str(results)],
                       cwd=EVALUATOR, check=True)
        summarize(run_dir)


def summarize(run_dir):
    run_dir = Path(run_dir)
    rows = [json.loads(l) for l in (run_dir / 'results.jsonl').read_text().splitlines() if l.strip()]
    metas = {m['instance_id']: m for m in (json.loads(p.read_text()) for p in run_dir.glob('*/meta.json'))}
    mean = lambda vals: sum(vals) / len(vals) if vals else float('nan')

    def pick(row, *keys):
        value = row
        for key in keys:
            value = value.get(key) if isinstance(value, dict) else None
        return value if isinstance(value, (int, float)) else None

    print(f'\n{run_dir.name}: {len(rows)} scored, {sum("error" in r for r in rows)} evaluator errors, '
          f'{sum("error" in m for m in metas.values())} run errors')
    if rows:
        sample = next(r for r in rows if 'error' not in r)
        print('  result keys:', sorted(sample.keys()))
    for label, keys in (('file coverage', ('final', 'file', 'coverage')), ('file precision', ('final', 'file', 'precision')),
                        ('line coverage', ('final', 'line', 'coverage')), ('line precision', ('final', 'line', 'precision')),
                        ('line F1', ('final', 'line', 'f1')), ('AUC line', ('trajectory', 'auc_coverage', 'line')),
                        ('redundancy line', ('trajectory', 'redundancy', 'line'))):
        vals = [pick(r, *keys) for r in rows]
        vals = [v for v in vals if v is not None]
        if vals:
            print(f'  {label:18s} {mean(vals):.3f}  (n={len(vals)})')
    good = [m for m in metas.values() if 'error' not in m]
    if good:
        print(f'  {"seconds":18s} {mean([m["seconds"] for m in good]):.0f}')
        print(f'  {"tokens":18s} {mean([m["tokens"] for m in good]):.0f}')
        print(f'  {"tool calls":18s} {mean([m["toolCalls"] for m in good]):.1f}  (oko searches {mean([m["okoSearches"] for m in good]):.1f})')
        print(f'  {"steps (views)":18s} {mean([m["steps"] for m in good]):.1f}')
        print(f'  {"gold files viewed":18s} {mean([m["goldFilesViewed"] / m["goldFiles"] for m in good if m["goldFiles"]]):.3f}')
        print(f'  {"declared context":18s} {sum(m["declared"] == "declared" for m in good)}/{len(good)}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--fetch', action='store_true')
    parser.add_argument('--condition', choices=('native', 'oko'), default='native')
    parser.add_argument('--binary', default=str(PROJECT / 'target/release/oko'))
    parser.add_argument('--model', default=None, help="Claude model; default is the login's default")
    parser.add_argument('--effort', default='medium')
    parser.add_argument('--limit', type=int, default=0)
    parser.add_argument('--instances', help='Comma-separated instance ids')
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('--issue-bytes', type=int, default=12000)
    parser.add_argument('--label', default=None)
    parser.add_argument('--rerun', action='store_true')
    parser.add_argument('--score', nargs='+', help='Run directories to score with the evaluator')
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    if args.fetch:
        return fetch()
    if args.score:
        return score(args.score)
    rows = load_instances(args.limit, set(args.instances.split(',')) if args.instances else None)
    label = args.label or time.strftime('%Y%m%d-%H%M%S')
    out_dir = STATE / 'runs' / f'{label}-{args.condition}'
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / 'settings.json').write_text(json.dumps({k: v for k, v in vars(args).items() if k != 'score'}, indent=2))
    if args.dry_run:
        for row in rows:
            print(row['instance_id'], row['repo'], row['language'], len(row['gold_files']), 'gold files')
        return
    for index, row in enumerate(rows, 1):
        meta = run_one(row, args, out_dir)
        print(f'{index}/{len(rows)} {row["instance_id"][:60]} {meta.get("seconds")}s '
              + (f'ERROR {meta["error"]}' if 'error' in meta else
                 f'tools={meta["toolCalls"]} tokens={meta["tokens"]} gold viewed {meta["goldFilesViewed"]}/{meta["goldFiles"]} {meta["declared"]}'),
              flush=True)
    print(out_dir)


if __name__ == '__main__':
    main()
