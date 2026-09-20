#!/usr/bin/env python3
"""Public-repository pilot: 12 tasks, 108 sessions. Plan-only unless --execute."""
import argparse
import difflib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent
PROJECT = ROOT.parents[1]
STATE = PROJECT / 'benchmarks/results/public'
FIXTURE = ROOT / 'tasks.json'
REPOSITORIES = json.loads(FIXTURE.read_text())['repositories']
CLIENTS = ('codex', 'opencode', 'claude')
CONDITIONS = ('native', 'cold', 'warm')
CACHE_POLICY = 'cold-vs-prebuilt-disk-v1'
SPEC = importlib.util.spec_from_file_location('public_engine', ROOT.parent / 'benchmark-twenty/runner.py')
engine = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(engine)
engine.ROOT = ROOT
original_args = engine.args_for
original_grade = engine.grade
original_prompt = engine.prompt


def save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix('.tmp')
    temp.write_text(json.dumps(data, indent=2) + '\n')
    temp.replace(path)


def digest(path):
    return engine.digest(path)


def implementation_digest():
    paths = sorted(ROOT.glob('*.py')) + [ROOT.parent / 'benchmark-twenty' / f for f in ('runner.py', 'oko-server.py')]
    return hashlib.sha256(''.join(digest(p) for p in paths).encode()).hexdigest()


def state(repo):
    return {'commit': engine.git(repo, 'rev-parse', 'HEAD').decode().strip(), 'status': engine.git(repo, 'status', '--porcelain').decode()}


def validate(task, work):
    # Do not pass provider credentials to post-edit code execution.
    env = {k: v for k, v in os.environ.items() if k in ('PATH', 'HOME', 'TMPDIR', 'SYSTEMROOT', 'RUSTUP_HOME', 'CARGO_HOME')}
    env['PYTHONDONTWRITEBYTECODE'] = '1'
    start = time.monotonic()
    try:
        result = subprocess.run([sys.executable, str(ROOT / 'validate.py'), task['id'], str(work)], env=env,
                                capture_output=True, text=True, timeout=75)
        return {'passed': result.returncode == 0, 'seconds': time.monotonic() - start,
                'exitCode': result.returncode, 'output': (result.stdout + result.stderr)[-12000:]}
    except subprocess.TimeoutExpired:
        return {'passed': False, 'seconds': time.monotonic() - start, 'output': 'Validation timed out'}


def reference(task, work):
    path = work / task['path']
    text = path.read_text()
    for change in task['replacements']:
        if change['old'] not in text:
            raise ValueError('Stale reference patch: ' + task['id'])
        text = text.replace(change['old'], change['new'])
    path.write_text(text)


def fixture_check(repo, task):
    targets = task['expected'] if task['kind'] == 'search' else [task]
    for target in targets:
        path = repo / target['path']
        if digest(path) != target['sha256']:
            raise ValueError('Stale fixture: ' + str(path))
        if task['kind'] == 'search' and not 1 <= target['startLine'] <= target['endLine'] <= len(path.read_text().splitlines()):
            raise ValueError('Invalid anchor: ' + task['id'])
    if task['kind'] == 'search':
        return {'id': task['id'], 'anchorsVerified': True}
    with tempfile.TemporaryDirectory(prefix='oko-contract-') as temp:
        work = Path(temp)
        files = [task['path']]
        if task['id'].startswith('httpx-'):
            files.append('httpx/_types.py')
        for name in files:
            dest = work / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(repo / name, dest)
        before = validate(task, work)
        reference(task, work)
        after = validate(task, work)
        if before['passed'] or not after['passed']:
            raise RuntimeError('Fixture must fail before and pass after: ' + task['id'] + '\n' + json.dumps({'before': before, 'after': after}))
        return {'id': task['id'], 'baseline': before, 'reference': after}


def prepare(args):
    clients = {c: shutil.which(c) for c in CLIENTS}
    if not all(clients.values()):
        raise RuntimeError('Install the three clients before preparing')
    if not shutil.which('node') or not shutil.which('rustc') or not shutil.which('rg'):
        raise RuntimeError('Node 22+ with type stripping, rustc, and rg are required')
    versions = {c: subprocess.check_output([exe, '--version'], text=True).strip() for c, exe in clients.items()}
    for item in REPOSITORIES:
        repo = (args.repositories / item['name']).expanduser().resolve()
        initial = state(repo)
        if initial != {'commit': item['commit'], 'status': ''}:
            raise RuntimeError(f'{repo} must be clean at the pinned commit; no reset performed')
        checks = [fixture_check(repo, t) for t in item['tasks']]
        dest = STATE / item['name']
        dest.mkdir(parents=True, exist_ok=True, mode=0o700)
        archive = dest / 'baseline.tar'
        subprocess.run(['git', 'archive', '--format=tar', '--output', str(archive), item['commit']], cwd=repo, check=True)
        settings = dict(repository=str(repo), commit=item['commit'], clients=clients, versions=versions,
                        models={'codex': args.codex_model, 'opencode': args.opencode_model, 'claude': args.claude_model},
                        effort=args.effort, timeoutSeconds=args.timeout, oko=str(args.oko.resolve()),
                        rg=shutil.which('rg'), jevModel='jev-1.13.0', archiveSha256=digest(archive),
                        okoSha256=digest(args.oko), tasksSha256=digest(FIXTURE), implementationSha256=implementation_digest(),
                        isolation=engine.ISOLATION_VERSION)
        if state(repo) != initial:
            raise RuntimeError('Source changed during preparation')
        save(dest / 'settings.json', settings)
        save(dest / 'preflight.json', {'checks': checks, 'sourceUnchanged': True})
        print(f"Prepared {item['name']}: 2 searches, 2 edits; executable edit checks fail before/pass after", flush=True)


def plan(repos, clients):
    result = []
    # Six permutations repeated evenly: each condition occupies each position
    # four times per client over the twelve tasks.
    import itertools
    orders = list(itertools.permutations(CONDITIONS))
    for task_index in range(4):
        for repo_index, item in enumerate(repos):
            offset = (task_index + repo_index) % len(clients)
            task_number = task_index * len(repos) + repo_index
            for c in clients[offset:] + clients[:offset]:
                conditions = orders[(task_number + CLIENTS.index(c)) % len(orders)]
                for condition in conditions:
                    result.append((item['name'], item['tasks'][task_index], c, condition))
    return result


def prewarm(work, trial, settings):
    """Build only the local index, with no task question or provider calls."""
    cache = trial / 'cache'
    if cache.exists():
        raise RuntimeError('Warm-up requires a fresh per-session cache')
    env = {k: v for k, v in os.environ.items() if not k.startswith(('TYPESAFE_', 'OKO_'))}
    env.update(OKO_CACHE_DIR=str(cache), OKO_NO_CACHE='0', OKO_RIPGREP=settings['rg'], TYPESAFE_API_KEY='')
    start = time.monotonic()
    proc = subprocess.run([settings['oko'], 'ask', '--no-jev', '--json', 'project source overview'],
                          cwd=work, env=env, capture_output=True, text=True, timeout=120)
    elapsed = time.monotonic() - start
    if proc.returncode:
        raise RuntimeError('Offline index warm-up failed: ' + proc.stderr[-2000:])
    metadata = json.loads(proc.stdout)['cache']
    if metadata['status'] != 'cold':
        raise RuntimeError('Warm-up did not begin with a fresh index')
    result = {'seconds': elapsed, 'cache': metadata, 'providerCalls': 0,
              'method': 'Task-independent lexical query; prebuilt disk index, fresh MCP process'}
    save(trial / 'warmup.json', result)
    return result


def args_for(task, client, enabled, work, trial):
    if task.get('cacheCondition') == 'warm':
        # Shared engine starts its agent timer after args_for returns.
        prewarm(work, trial, engine.SETTINGS)
    args, env = original_args(task, client, enabled, work, trial)
    env['OKO_PUBLIC_BENCH_REPO'] = task['repositoryName']
    return args, env


def cache_observations(row):
    def packet(value):
        if isinstance(value, str):
            try:
                return packet(json.loads(value))
            except (ValueError, RecursionError):
                return None
        if isinstance(value, dict):
            timing = value.get('timings')
            if isinstance(timing, dict) and isinstance(timing.get('cache'), dict):
                return timing['cache']
            for child in value.values():
                found = packet(child)
                if found is not None:
                    return found
        elif isinstance(value, list):
            for child in value:
                found = packet(child)
                if found is not None:
                    return found
        return None
    observations = []
    for tool in row.get('tools', []):
        name = tool.get('name') or tool.get('tool')
        if tool.get('server') == 'oko' or name in ('oko_search', 'mcp__oko__search'):
            # Startup preparation, not the memory hit after it, shows the session's cache state.
            for event in tool.get('okoPrewarm') or []:
                if isinstance(event.get('cache'), dict):
                    observations.append(event['cache'])
            found = packet({k: v for k, v in tool.items() if k != 'okoPrewarm'})
            if found is not None:
                observations.append(found)
    # Claude's tool_use events contain only inputs. Match their tool_result IDs
    # in the raw user events to capture Oko's returned cache metadata.
    if row.get('client') == 'claude':
        ids = {t.get('id') for t in row.get('tools', []) if t.get('name') == 'mcp__oko__search'}
        for line in (Path(row['artifact']) / 'events.jsonl').read_text().splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            for block in event.get('message', {}).get('content', []) if event.get('type') == 'user' else []:
                if isinstance(block, dict) and block.get('type') == 'tool_result' and block.get('tool_use_id') in ids:
                    found = packet(block)
                    if found is not None:
                        observations.append(found)
    return observations


def check_cache(condition, observations):
    if condition == 'native':
        return not observations
    if not observations:
        return False
    first = observations[0]
    return (first.get('status') == 'cold' if condition == 'cold'
            else first.get('status') == 'disk' and first.get('rebuiltFiles') == 0 and first.get('reusedFiles', 0) > 0)


def prompt(task, enabled):
    text = original_prompt(task, enabled)
    if task['kind'] == 'edit':
        text = text.replace('this fixture evaluates retrieval and a bounded patch without application dependencies',
                            'the runner will independently execute focused behavior checks after your session')
        text += '\nKeep the change local to the implementation described. Do not add tests; the runner supplies independent checks.'
    return text


def within_edit_scope(baseline, actual, allowed):
    start, end = allowed
    changes = difflib.SequenceMatcher(None, baseline.splitlines(), actual.splitlines(), autojunk=False).get_opcodes()
    return all(tag == 'equal' or (start - 1 <= a <= b <= end) for tag, a, b, _, _ in changes)


def grade(task, work, final):
    if task['kind'] == 'search':
        result = original_grade(task, work, final)
        if result.get('gradingError'):
            return {**result, 'passed': False}
        import re
        answers = json.loads(re.sub(r'^```(?:json)?\s*|\s*```$', '', final.strip()))['results']
        # Require every anchor, including both locations on tracing tasks.
        covered = [any(a['path'] == e['path'] and a['startLine'] <= e['startLine'] and a['endLine'] >= e['endLine'] for a in answers) for e in task['expected']]
        return {**result, 'anchorsCovered': covered, 'passed': all(covered)}
    changed = engine.git(work, 'diff', 'HEAD', '--name-only').decode().splitlines()
    extras = engine.git(work, 'ls-files', '--others').decode().splitlines()
    path = work / task['path']
    unexpected = [p for p in changed + extras if p != task['path']]
    if not path.is_file() or path.is_symlink() or unexpected:
        return {'passed': False, 'unexpectedFiles': unexpected, 'reason': 'Target missing/symlink or unrelated edits'}
    baseline = engine.git(work, 'show', 'HEAD:' + task['path']).decode()
    if not within_edit_scope(baseline, path.read_text(), task['allowedLines']):
        return {'passed': False, 'reason': 'Changes outside the requested implementation; review required'}
    result = validate(task, work)
    save(work.parent / 'validation.json', result)
    return {'passed': result['passed'] and task['path'] in changed, 'behaviorTestsPassed': result['passed'],
            'validationSeconds': result['seconds'], 'unexpectedFiles': unexpected,
            'productValidation': 'Focused module contract checks, not full application tests'}


engine.args_for = args_for
engine.prompt = prompt
engine.grade = grade


def token_breakdown(row):
    t = row.get('tokens') or {}
    if not t or t.get('total') is None:
        return None
    if row['client'] == 'codex':
        return dict(total=t['total'], uncachedInput=t['input']-t['cachedInput'], cacheRead=t['cachedInput'], cacheWrite=0, output=t['output'])
    if row['client'] == 'claude':
        return dict(total=t['total'], uncachedInput=t['input'], cacheRead=t['cacheRead'], cacheWrite=t['cacheWrite'], output=t['output'])
    steps = t['steps']
    return dict(total=t['total'], uncachedInput=sum(s['input'] for s in steps), cacheRead=sum(s['cache']['read'] for s in steps),
                cacheWrite=sum(s['cache']['write'] for s in steps), output=sum(s['output']+s.get('reasoning',0) for s in steps))


def report(output, data):
    save(output / 'report.json', data)
    lines = ['# Public repository pilot', '', f"Sessions: {len(data['runs'])}/{len(data['plan'])}. Complete: {data['complete']}.", '',
             '| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens |', '|---|---|---:|---:|---:|']
    for c in CLIENTS:
        for condition in CONDITIONS:
            rows = [r for r in data['runs'] if r['client']==c and r['condition']==condition]
            if not rows:
                continue
            tokens = [token_breakdown(r) for r in rows]
            total = sum(t['total'] for t in tokens) if all(t is not None for t in tokens) else 'unavailable'
            passed = sum(r.get('grade',{}).get('passed',False) and not r.get('error') for r in rows)
            lines.append(f"| {c} | {condition} | {passed}/{len(rows)} | {statistics.median(r['seconds'] for r in rows):.2f} | {total} |")
    lines += ['', 'Each condition has the same tasks. Failed attempts remain in the timing table; inspect success rates before claiming a speed win.',
              'Fresh checkout and conversation per session. Cold = empty index; warm = prebuilt disk index with a fresh MCP process. Offline warm-up time is recorded separately and excluded from agent latency. Skills/custom instructions disabled.',
              'Token totals include provider-cached input and exclude Jev; they are not cost estimates. Full token breakdowns and per-task timings are in report.json.',
              'One repetition per task; 12 tasks across 3 repositories. Focused module checks are not full application correctness. No provider cache clearing.',
              'Warm does not measure a persistent in-memory MCP server or a cached Jev answer. Provider requests still run normally.',
              'Models differ across clients; compare with/without Oko within each client. All attempts, failures, and timeouts are retained.']
    warmups = [r['warmup']['seconds'] for r in data['runs'] if r.get('warmup')]
    if warmups:
        lines += ['', f'Offline warm-up: median {statistics.median(warmups):.2f}s across {len(warmups)} sessions (excluded from agent timing; no provider calls).']
    lines += ['', '| Repository | Task | Client | Without Oko | Cold Oko | Warm Oko | All passed |', '|---|---|---|---:|---:|---:|---|']
    for name, task_id, client in dict.fromkeys((x['repository'], x['id'], x['client']) for x in data['runs']):
        pair = [next((x for x in data['runs'] if x['repository']==name and x['id']==task_id and x['client']==client and x['condition']==condition), None) for condition in CONDITIONS]
        times = [f"{x['seconds']:.2f}s" if x else 'pending' for x in pair]
        passed = all(x and not x.get('error') and x.get('grade',{}).get('passed') for x in pair)
        lines.append(f'| {name} | {task_id} | {client} | {times[0]} | {times[1]} | {times[2]} | {passed} |')
    (output / 'report.md').write_text('\n'.join(lines)+'\n')


def execute(args, schedule):
    settings = {}
    for name in dict.fromkeys(p[0] for p in schedule):
        folder = STATE / name
        s = json.loads((folder/'settings.json').read_text())
        for path, key in [(folder/'baseline.tar','archiveSha256'), (FIXTURE,'tasksSha256'), (Path(s['oko']),'okoSha256')]:
            if digest(path) != s[key]:
                raise RuntimeError('Frozen artifact changed; prepare again: '+str(path))
        if s['implementationSha256'] != implementation_digest() or s['isolation'] != engine.ISOLATION_VERSION:
            raise RuntimeError('Runner changed; prepare again')
        if state(Path(s['repository'])) != {'commit':s['commit'],'status':''}:
            raise RuntimeError('Source changed; no reset performed')
        for c in args.clients:
            if subprocess.check_output([s['clients'][c],'--version'],text=True).strip()!=s['versions'][c]:
                raise RuntimeError('CLI version changed; prepare again')
        settings[name]=s
    ids = [{'repository':n,'task':t['id'],'client':c,'condition':condition} for n,t,c,condition in schedule]
    if args.resume:
        output=args.resume.resolve()
        if output.parent != STATE.resolve() or not output.name.startswith('results-'):
            raise ValueError('Resume must identify a public benchmark results directory')
        data=json.loads((output/'report.json').read_text())
        if data['plan']!=ids or data['settings']!=settings or data.get('cachePolicy')!=CACHE_POLICY:
            raise ValueError('Resume plan/settings changed')
    else:
        output=Path(tempfile.mkdtemp(prefix='results-',dir=STATE))
        data={'plan':ids,'settings':settings,'runs':[],'complete':False,'isolation':engine.ISOLATION_VERSION,'cachePolicy':CACHE_POLICY}
    print('Results: '+str(output),flush=True)
    report(output,data)
    try:
        for i,(name,task,client,condition) in enumerate(schedule,1):
            if i<=len(data['runs']):
                continue
            engine.STATE=STATE/name
            engine.SETTINGS=settings[name]
            on = condition != 'native'
            trials=engine.STATE/output.name/condition
            trials.mkdir(parents=True,exist_ok=True,mode=0o700)
            trial=trials/f"{i:03}-{task['id']}-{client}-{'oko' if on else 'native'}"
            if trial.exists():
                # Never silently repeat a possibly billed interrupted session.
                raise RuntimeError('Existing unrecorded session requires inspection: '+str(trial))
            print(f'START {i}/{len(schedule)} {name} {task["id"]} {client} condition={condition}',flush=True)
            row=engine.run_one({**task,'repositoryName':name,'cacheCondition':condition},client,on,trials,i)
            row['repository']=name
            row['condition']=condition
            row['cacheObservations']=cache_observations(row)
            row['cacheConditionVerified']=check_cache(condition,row['cacheObservations'])
            if condition == 'warm':
                row['warmup']=json.loads((Path(row['artifact'])/'warmup.json').read_text())
            if not row['cacheConditionVerified']:
                row['cacheCheckError']='Observed Oko cache did not match assigned condition'
                row.setdefault('error', row['cacheCheckError'])
                row['errorType']='infrastructure'
            row['tokenBreakdown']=token_breakdown(row)
            save(Path(row['artifact'])/'result.json',row)
            data['runs'].append(row)
            report(output,data)
            if row.get('error') and row.get('errorType')!='answer':
                raise RuntimeError(row['error'])
    finally:
        data['sourceUnchanged']=all(state(Path(s['repository']))=={'commit':s['commit'],'status':''} for s in settings.values())
        data['artifactsUnchanged']=all(digest(STATE/n/'baseline.tar')==s['archiveSha256'] and digest(s['oko'])==s['okoSha256'] for n,s in settings.items()) and digest(FIXTURE)==next(iter(settings.values()))['tasksSha256']
        data['complete']=len(data['runs'])==len(schedule) and data['sourceUnchanged'] and data['artifactsUnchanged']
        report(output,data)
    print(output)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    action=p.add_mutually_exclusive_group()
    action.add_argument('--prepare',action='store_true',help='Freeze clones/binary/models and verify edit graders; no paid calls')
    action.add_argument('--execute',action='store_true',help='Start paid model/Jev sessions')
    p.add_argument('--repositories',type=Path,default=Path.home()/'dev')
    p.add_argument('--clients',default=','.join(CLIENTS))
    p.add_argument('--resume',type=Path)
    p.add_argument('--oko',type=Path,default=PROJECT/'target/release/oko')
    p.add_argument('--codex-model',default='gpt-5.6-sol')
    p.add_argument('--opencode-model',default='openai/gpt-5.6-sol')
    p.add_argument('--claude-model',default='claude-sonnet-5')
    p.add_argument('--effort',choices=('low','medium','high'),default='low')
    p.add_argument('--timeout',type=int,default=180)
    args=p.parse_args();args.clients=args.clients.split(',')
    if not args.clients or len(set(args.clients))!=len(args.clients) or any(c not in CLIENTS for c in args.clients):p.error('Invalid clients')
    if args.timeout<1:p.error('Timeout must be positive')
    if args.resume and not args.execute:p.error('--resume requires --execute')
    if args.prepare:
        prepare(args);return
    schedule=plan(REPOSITORIES,args.clients)
    if args.execute:
        execute(args,schedule);return
    print(f'{len(schedule)} sessions; 2 searches + 2 edits per repository. No model calls.')
    for i,(name,t,c,condition) in enumerate(schedule,1):print(f'{i:03} {name} {t["kind"]} {t["id"]} {c} {condition}')


if __name__=='__main__':
    main()
