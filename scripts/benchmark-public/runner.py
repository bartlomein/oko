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
import contextlib
import fcntl
from measurements import token_breakdown, measurements

ROOT = Path(__file__).resolve().parent
PROJECT = ROOT.parents[1]
STATE = PROJECT / 'benchmarks/results/public'
FIXTURE = ROOT / 'tasks.json'
REPOSITORIES = json.loads(FIXTURE.read_text())['repositories']
CLIENTS = ('codex', 'opencode', 'claude')
CONDITIONS = ('native', 'cold', 'warm')
ENGINE_CONDITIONS = {'native': 'native', 'cold': 'oko-cold', 'warm': 'oko-warm'}
CACHE_POLICY = 'cold-vs-prebuilt-disk-v1'
SUITE = 'legacy'
# Frozen per-commit builds and the validator dependency are shared by every
# suite that compares builds, so a smoke run never rebuilds what branch built.
BUILD_STATE = PROJECT / 'benchmarks/results/public-branch'
# Minutes instead of hours: tasks that separated builds in full runs (a missed
# anchor behind a short excerpt, a partly covered trace), one that Oko clearly
# helps, and one edit as a control. Too few tasks for any claim; use the branch
# suite before reporting results.
SMOKE_TASKS = ('astro-image-probe-authorization', 'astro-action-key-guards',
               'ripgrep-capture-expansion', 'ripgrep-capture-hyphen')


def compares_builds():
    return SUITE in ('branch', 'smoke')
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
    paths = sorted(ROOT.glob('*.py')) + [ROOT.parent / 'benchmark-twenty' / f for f in ('runner.py', 'oko-server.py', 'isolation-smoke.py')] + [ROOT.parent / 'benchmark_observability.py']
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
        files = [task['path'], *task.get('dependencies', [])]
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
    if compares_builds():
        from builds import prepare_builds
        builds = prepare_builds(PROJECT, BUILD_STATE, args.baseline_ref, args.current_ref)
        args.oko = Path(builds['current']['path'])
        from isolation_check import check_isolation
        isolation = check_isolation(ROOT, STATE)
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
        if compares_builds():
            settings.update(builds=builds, suite=SUITE, repeats=args.repeats,
                            cachePolicy=args.cache_policy, isolationCheck=isolation,
                            tasks=[t['id'] for t in item['tasks']],
                            validatorLibrarySha256=digest(BUILD_STATE/'libmemchr.rlib'),
                            runtimeVersions={name: subprocess.check_output([name, '--version'], text=True).strip()
                                             for name in ('node', 'rustc')}, pythonVersion=sys.version)
        if state(repo) != initial:
            raise RuntimeError('Source changed during preparation')
        save(dest / 'settings.json', settings)
        save(dest / 'preflight.json', {'checks': checks, 'sourceUnchanged': True})
        print(f"Prepared {item['name']}: {len(item['tasks'])} tasks; executable edit checks fail before/pass after", flush=True)
    save(STATE/'plan.json',{
        'suite':SUITE,'repeats':args.repeats,'cachePolicy':CACHE_POLICY,
        'memoryCanarySessions':2*len(args.clients) if SUITE=='branch' else 0,
        'sessions':[{'repository':name,'task':task['id'],'client':client,
                     'condition':condition,'repetition':task['repetition']}
                    for name,task,client,condition in plan(REPOSITORIES,args.clients,args.repeats)]})


def plan(repos, clients, repeats=1):
    result = []
    # Rotate each task/client order across repeats: at three repeats every
    # condition occupies every position once. Base permutations vary by task.
    import itertools
    orders = list(itertools.permutations(CONDITIONS))
    for repeat in range(repeats):
        for task_index in range(max(len(r['tasks']) for r in repos)):
            for repo_index, item in enumerate(repos):
                if task_index >= len(item['tasks']):
                    continue
                offset = (task_index + repo_index) % len(clients)
                task_number = task_index * len(repos) + repo_index
                for c in clients[offset:] + clients[:offset]:
                    conditions = orders[(task_number + CLIENTS.index(c)) % len(orders)]
                    shift = repeat % len(conditions)
                    conditions = conditions[shift:] + conditions[:shift]
                    for condition in conditions:
                        task = dict(item['tasks'][task_index], repetition=repeat + 1)
                        result.append((item['name'], task, c, condition))
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


def args_for(task, client, condition, work, trial):
    # Pin the selected executable per session; never read mutable repo-wide settings.
    save(trial/'settings.json', engine.SETTINGS)
    args, env = original_args(task, client, condition, work, trial)
    env['OKO_PUBLIC_BENCH_REPO'] = task['repositoryName']
    if compares_builds() and client == 'opencode':
        # Separate both conversation storage and configuration. Link login state only.
        original_data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home()/'.local/share')))
        for name in ('DATA', 'STATE', 'CACHE'):
            folder = trial/'harness-config'/name.lower()
            folder.mkdir(parents=True, exist_ok=True)
            env['XDG_'+name+'_HOME'] = str(folder)
        auth = original_data/'opencode/auth.json'
        if auth.is_file():
            target = Path(env['XDG_DATA_HOME'])/'opencode/auth.json'
            target.parent.mkdir(parents=True, exist_ok=True)
            target.symlink_to(auth.resolve())
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
    return (first.get('status') == 'cold' if ENGINE_CONDITIONS[condition] == 'oko-cold'
            else first.get('status') == 'disk' and first.get('rebuiltFiles') == 0 and first.get('reusedFiles', 0) > 0)


def prompt(task, enabled):
    if task.get('memoryCanary'):
        return task['question']
    text = original_prompt(task, enabled)
    if task['kind'] == 'edit':
        text = text.replace('this fixture evaluates retrieval and a bounded patch without application dependencies',
                            'the runner will independently execute focused behavior checks after your session')
        text += '\nKeep the change local to the implementation described. Do not add tests; the runner supplies independent checks.'
    return text


def within_edit_scope(baseline, actual, allowed):
    ranges = [allowed] if isinstance(allowed[0], int) else allowed
    changes = difflib.SequenceMatcher(None, baseline.splitlines(), actual.splitlines(), autojunk=False).get_opcodes()
    return all(tag == 'equal' or any(start - 1 <= a <= b <= end for start, end in ranges) for tag, a, b, _, _ in changes)


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
    if not within_edit_scope(baseline, path.read_text(), task.get('allowedRanges', task['allowedLines'])):
        return {'passed': False, 'reason': 'Changes outside the requested implementation; review required'}
    result = validate(task, work)
    save(work.parent / 'validation.json', result)
    return {'passed': result['passed'] and task['path'] in changed, 'behaviorTestsPassed': result['passed'],
            'validationSeconds': result['seconds'], 'unexpectedFiles': unexpected,
            'productValidation': 'Focused module contract checks, not full application tests'}


engine.args_for = args_for
engine.prewarm = lambda work, trial: prewarm(work, trial, engine.SETTINGS)
engine.prompt = prompt
engine.grade = grade


def report(output, data):
    from reporting import render
    save(output / 'report.json', data)
    (output / 'report.md').write_text(render(data, CLIENTS, CONDITIONS))


def verify_settings(args, schedule):
    settings = {}
    for name in dict.fromkeys(p[0] for p in schedule):
        folder = STATE / name
        s = json.loads((folder/'settings.json').read_text())
        for path, key in [(folder/'baseline.tar','archiveSha256'), (FIXTURE,'tasksSha256'), (Path(s['oko']),'okoSha256')]:
            if digest(path) != s[key]:
                raise RuntimeError('Frozen artifact changed; prepare again: '+str(path))
        if s['implementationSha256'] != implementation_digest() or s['isolation'] != engine.ISOLATION_VERSION:
            raise RuntimeError('Runner changed; prepare again')
        if compares_builds():
            if s['repeats'] != args.repeats or s['cachePolicy'] != args.cache_policy:
                raise RuntimeError('Frozen repetitions/cache policy changed; prepare again')
            selected = [t['id'] for item in REPOSITORIES if item['name'] == name for t in item['tasks']]
            # Only the smoke suite's task list can vary between preparations.
            if SUITE == 'smoke' and (s.get('suite') != SUITE or s.get('tasks') != selected):
                raise RuntimeError('Frozen suite/task selection changed; prepare again')
            for build in s['builds'].values():
                if digest(Path(build['path'])) != build['sha256']:
                    raise RuntimeError('Frozen build changed; prepare again')
            if digest(BUILD_STATE/'libmemchr.rlib') != s['validatorLibrarySha256']:
                raise RuntimeError('Validator dependency changed; prepare again')
            if not s['isolationCheck'].get('passed'):
                raise RuntimeError('Isolation preflight required')
            if s['pythonVersion'] != sys.version or any(subprocess.check_output([name,'--version'],text=True).strip()!=version for name,version in s['runtimeVersions'].items()):
                raise RuntimeError('Validation runtime changed; prepare again')
        if state(Path(s['repository'])) != {'commit':s['commit'],'status':''}:
            raise RuntimeError('Source changed; no reset performed')
        for c in args.clients:
            if subprocess.check_output([s['clients'][c],'--version'],text=True).strip()!=s['versions'][c]:
                raise RuntimeError('CLI version changed; prepare again')
        settings[name]=s
    return settings


def execute(args, schedule):
    settings = verify_settings(args, schedule)
    ids = [{'repository':n,'task':t['id'],'client':c,'condition':condition,'repetition':t.get('repetition',1)} for n,t,c,condition in schedule]
    if args.resume:
        output=args.resume.resolve()
        if output.parent != STATE.resolve() or not output.name.startswith('results-'):
            raise ValueError('Resume must identify a public benchmark results directory')
        data=json.loads((output/'report.json').read_text())
        if data['plan']!=ids or data['settings']!=settings or data.get('cachePolicy')!=CACHE_POLICY:
            raise ValueError('Resume plan/settings changed')
        if any(r.get('error') and r.get('errorType')!='answer' for r in data['runs']):
            raise RuntimeError('Saved infrastructure failure requires inspection before resuming')
    else:
        output=Path(tempfile.mkdtemp(prefix='results-',dir=STATE))
        data={'plan':ids,'settings':settings,'runs':[],'complete':False,'isolation':engine.ISOLATION_VERSION,'cachePolicy':CACHE_POLICY,'repeats':getattr(args,'repeats',1),'suite':SUITE}
    print('Results: '+str(output),flush=True)
    report(output,data)
    if SUITE == 'branch' and not data.get('memoryCanary',{}).get('complete'):
        if (output/'memory-canary').exists():
            raise RuntimeError('Incomplete memory canary exists; inspect before restarting paid calls')
        from isolation_check import memory_canary
        # Pass this module without importing a second mutable runner instance.
        data['memoryCanary']=memory_canary(sys.modules[__name__],next(iter(settings.values())),output,args.clients)
        report(output,data)
    try:
        for i,(name,task,client,condition) in enumerate(schedule,1):
            if i<=len(data['runs']):
                continue
            engine.STATE=STATE/name
            engine.SETTINGS=dict(settings[name])
            if compares_builds():
                selected = settings[name]['builds']['current' if condition=='native' else condition]
                engine.SETTINGS.update(oko=selected['path'],okoSha256=selected['sha256'])
            engine_condition = ENGINE_CONDITIONS[condition]
            trials=engine.STATE/output.name/condition
            trials.mkdir(parents=True,exist_ok=True,mode=0o700)
            trial=trials/f"{i:03}-{task['id']}-{client}-{engine_condition}"
            if trial.exists():
                # Never silently repeat a possibly billed interrupted session.
                raise RuntimeError('Existing unrecorded session requires inspection: '+str(trial))
            print(f'START {i}/{len(schedule)} {name} {task["id"]} {client} condition={condition}',flush=True)
            row=engine.run_one({**task,'repositoryName':name,'cacheCondition':condition},client,engine_condition,trials,i)
            row['repository']=name
            row['condition']=condition
            row['repetition']=task.get('repetition',1)
            if compares_builds():
                row['build']=None if condition=='native' else selected
            row['cacheObservations']=cache_observations(row)
            row['cacheConditionVerified']=check_cache(condition,row['cacheObservations'])
            if engine_condition == 'oko-warm':
                warmup=Path(row['artifact'])/'warmup.json'
                if warmup.exists():row['warmup']=json.loads(warmup.read_text())
            if not row['cacheConditionVerified']:
                row['cacheCheckError']='Observed Oko cache did not match assigned condition'
                row.setdefault('error', row['cacheCheckError'])
                row['errorType']='infrastructure'
            row['tokenBreakdown']=token_breakdown(row)
            row['measurements']=measurements(row)
            save(Path(row['artifact'])/'result.json',row)
            data['runs'].append(row)
            report(output,data)
            if row.get('error') and row.get('errorType')!='answer':
                raise RuntimeError(row['error'])
    finally:
        data['sourceUnchanged']=all(state(Path(s['repository']))=={'commit':s['commit'],'status':''} for s in settings.values())
        data['artifactsUnchanged']=all(digest(STATE/n/'baseline.tar')==s['archiveSha256'] and digest(s['oko'])==s['okoSha256'] for n,s in settings.items()) and digest(FIXTURE)==next(iter(settings.values()))['tasksSha256']
        if compares_builds():
            data['artifactsUnchanged'] &= all(digest(Path(b['path']))==b['sha256'] for s in settings.values() for b in s['builds'].values())
        data['complete']=len(data['runs'])==len(schedule) and data['sourceUnchanged'] and data['artifactsUnchanged']
        report(output,data)
    print(output)


def main():
    global SUITE, STATE, FIXTURE, REPOSITORIES, CONDITIONS, ENGINE_CONDITIONS, CACHE_POLICY
    p=argparse.ArgumentParser(description=__doc__)
    action=p.add_mutually_exclusive_group()
    action.add_argument('--prepare',action='store_true',help='Freeze clones/binary/models and verify edit graders; no paid calls')
    action.add_argument('--execute',action='store_true',help='Start paid model/Jev sessions')
    action.add_argument('--check',action='store_true',help='Verify all frozen artifacts without model calls')
    p.add_argument('--repositories',type=Path,default=Path.home()/'dev')
    p.add_argument('--clients',help='Comma-separated; default all three, or claude alone for the smoke suite')
    p.add_argument('--resume',type=Path)
    p.add_argument('--oko',type=Path,default=PROJECT/'target/release/oko')
    p.add_argument('--codex-model',default='gpt-5.6-sol')
    p.add_argument('--opencode-model',default='openai/gpt-5.6-sol')
    p.add_argument('--claude-model',default='claude-sonnet-5')
    p.add_argument('--effort',choices=('low','medium','high'),default='low')
    p.add_argument('--timeout',type=int,default=180)
    p.add_argument('--suite', choices=('legacy','branch','smoke'), default='legacy',
                   help='smoke: previous vs current build on a few branch tasks, minutes not hours; direction only')
    p.add_argument('--tasks',help='Smoke suite only: comma-separated branch task ids (default: '+','.join(SMOKE_TASKS)+')')
    p.add_argument('--repeats',type=int)
    p.add_argument('--cache-policy',choices=('cold','warm'),default='warm')
    p.add_argument('--baseline-ref',default='main')
    p.add_argument('--current-ref',default='HEAD')
    args=p.parse_args()
    SUITE=args.suite
    args.clients=(args.clients or ('claude' if SUITE=='smoke' else ','.join(CLIENTS))).split(',')
    if args.tasks and SUITE!='smoke':p.error('--tasks requires --suite smoke')
    args.repeats=args.repeats if args.repeats is not None else (3 if compares_builds() else 1)
    if args.repeats<1:p.error('Repeats must be positive')
    if SUITE=='branch':
        STATE=PROJECT/'benchmarks/results/public-branch'
        FIXTURE=ROOT/'tasks-branch.json'
        REPOSITORIES=json.loads(FIXTURE.read_text())['repositories']
        CONDITIONS=('native','previous','current')
        ENGINE_CONDITIONS={'native':'native','previous':'oko-'+args.cache_policy,'current':'oko-'+args.cache_policy}
        CACHE_POLICY=args.cache_policy
    if SUITE=='smoke':
        # Native search does not change between Oko builds; reuse a full run for it.
        STATE=PROJECT/'benchmarks/results/public-smoke'
        FIXTURE=ROOT/'tasks-branch.json'
        wanted=args.tasks.split(',') if args.tasks else list(SMOKE_TASKS)
        available={t['id'] for item in json.loads(FIXTURE.read_text())['repositories'] for t in item['tasks']}
        if len(set(wanted))!=len(wanted) or not set(wanted)<=available:p.error('Unknown or repeated smoke task; choose from: '+', '.join(sorted(available)))
        REPOSITORIES=[dict(item,tasks=[t for t in item['tasks'] if t['id'] in wanted])
                      for item in json.loads(FIXTURE.read_text())['repositories']]
        REPOSITORIES=[item for item in REPOSITORIES if item['tasks']]
        CONDITIONS=('previous','current')
        ENGINE_CONDITIONS={'previous':'oko-'+args.cache_policy,'current':'oko-'+args.cache_policy}
        CACHE_POLICY=args.cache_policy
    if not args.clients or len(set(args.clients))!=len(args.clients) or any(c not in CLIENTS for c in args.clients):p.error('Invalid clients')
    if args.timeout<1:p.error('Timeout must be positive')
    if args.resume and not args.execute:p.error('--resume requires --execute')
    if args.prepare:
        with suite_lock():prepare(args)
        return
    schedule=plan(REPOSITORIES,args.clients,args.repeats)
    if args.check:
        with suite_lock():
            verify_settings(args,schedule)
            print(f'Ready: {len(schedule)} timed sessions; '+(f'{2*len(args.clients)} memory-canary sessions; ' if SUITE=='branch' else '')+'no model calls made.')
        return
    if args.execute:
        with suite_lock():execute(args,schedule)
        return
    print(f'{len(schedule)} sessions; suite={SUITE}; repeats={args.repeats}; conditions={CONDITIONS}. No model calls.')
    for i,(name,t,c,condition) in enumerate(schedule,1):print(f'{i:03} repeat={t["repetition"]} {name} {t["kind"]} {t["id"]} {c} {condition}')


@contextlib.contextmanager
def suite_lock():
    STATE.mkdir(parents=True,exist_ok=True)
    with (STATE/'.lock').open('w') as lock:
        try:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError('This suite is already preparing or running')
        yield


if __name__=='__main__':
    main()
