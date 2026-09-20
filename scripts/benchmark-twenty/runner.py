#!/usr/bin/env python3
"""Twenty benchmark (10 read-only, 5 edits). Defaults to plan-only; --execute is required for model calls."""
import argparse, hashlib, json, os, re, shutil, signal, statistics, subprocess, sys, tarfile, tempfile
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import benchmark_observability as observability

ROOT = Path(__file__).resolve().parent
PROJECT_NAME = 'Twenty'
STATE = ROOT.parents[1] / 'benchmarks/results/twenty'
SETTINGS = {}
TASKS = json.loads((ROOT / 'tasks.json').read_text())['tasks']
MODES = [(client, oko) for client in ('codex', 'opencode', 'claude') for oko in (False, True)]
FAST_TASK_IDS = ('rich-text-preview', 'metadata-pagination', 'locale-direction', 'preview-debounce', 'email-retry-delay')
PILOT_TASK_IDS = ('search-normalization', 'email-retry-delay')

ISOLATION_VERSION = 'blank-slate-v1'
CUSTOM_DIRS = {'.agents', '.codex', '.claude', '.opencode'}
CUSTOM_FILES = {'AGENTS.md', 'AGENTS.override.md', 'CLAUDE.md', 'CLAUDE.local.md',
                'SKILL.md', 'opencode.json', 'opencode.jsonc', '.mcp.json'}

def codex_skill_overrides(work, isolated):
    """Disable discovered skill paths as well as discovery (CLI versions differ)."""
    user_root = Path(os.environ.get('CODEX_HOME', str(Path.home() / '.codex')))
    roots = {user_root / 'skills', Path.home() / '.agents/skills', Path('/etc/codex/skills')}
    for parent in (work, *work.parents):
        roots.update((parent / '.agents/skills', parent / '.codex/skills'))
    paths, visited = set(), set()
    for root in roots:
        for base, dirs, files in os.walk(root, followlinks=True):
            real = Path(base).resolve()
            if real in visited:
                dirs[:] = []
                continue
            visited.add(real)
            if 'SKILL.md' in files:
                skill = Path(base) / 'SKILL.md'
                paths.update((skill, skill.parent, skill.resolve(), skill.resolve().parent))
    # Codex installs bundled skills into each fresh CLI home at startup.
    for source in (user_root / 'skills/.system').glob('*/SKILL.md'):
        target = isolated / 'skills/.system' / source.parent.name / 'SKILL.md'
        paths.update((target, target.parent))
    return '[' + ','.join('{path=' + json.dumps(str(path)) + ',enabled=false}'
                          for path in sorted(paths)) + ']'

def strip_customizations(work):
    """Only called on a new disposable checkout, before its baseline commit."""
    removed = []
    for base, dirs, files in os.walk(work, followlinks=False):
        for name in list(dirs):
            if name in CUSTOM_DIRS:
                path = Path(base) / name
                removed.append(str(path.relative_to(work)))
                if path.is_symlink():
                    path.unlink()
                else:
                    shutil.rmtree(path)
                dirs.remove(name)
        for name in files:
            if name in CUSTOM_FILES or name in CUSTOM_DIRS:
                path = Path(base) / name
                removed.append(str(path.relative_to(work)))
                path.unlink()
    return sorted(removed)

def select_tasks(*, fast=False, pilot=False):
    if fast:
        by_id = {task['id']: task for task in TASKS}
        return [by_id[task_id] for task_id in FAST_TASK_IDS]
    if pilot:
        return [task for task in TASKS if task['id'] in PILOT_TASK_IDS]
    return TASKS

def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def git(work, *args):
    return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'gc.auto=0', '-c', 'maintenance.auto=false', *args], cwd=work, env={**os.environ, 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_SYSTEM': '/dev/null'}, stderr=subprocess.DEVNULL)

def checkout(work):
    work.mkdir()
    with tarfile.open(STATE / 'baseline.tar') as tar:
        tar.extractall(work, filter='data')
    removed = strip_customizations(work)
    save(work.parent / 'isolation.json', {'version': ISOLATION_VERSION, 'removedPaths': removed})
    git(work, 'init', '-q')
    git(work, 'add', '-f', '.')
    git(work, '-c', 'user.name=Benchmark', '-c', 'user.email=benchmark@localhost', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'Frozen benchmark baseline')

def prompt(task, enabled):
    mode = 'Use Oko MCP search first to locate the relevant code, phrasing the search from the request. Use its evidence when sufficient; native follow-up reads/searches are allowed.' if enabled else 'Use native local search/read tools. Do not invoke Oko through MCP or the shell; it is disabled for this comparison. This benchmark condition overrides project guidance to use Oko.'
    scope = 'Do not edit any files. Return only JSON: {"results":[{"path":"relative/file","startLine":1,"endLine":2}]}, at most five results, inclusive 1-based ranges at most 120 lines.' if task['kind'] == 'search' else 'Implement the requested small edit in this disposable checkout. Preserve unrelated behavior. Do not run builds/tests or install dependencies; this fixture evaluates retrieval and a bounded patch without application dependencies. Do not commit. Finish with a concise summary of changes and checks actually performed. This is an experimental patch, not a production deployment.'
    return task['question'] + '\n\n' + scope + '\n' + mode + '\nWork only inside the current checkout. Do not inspect other repositories, benchmark fixtures or answers, saved sessions or memories. Do not run builds/tests, use the web, delegate, deploy, install dependencies, contact live product services, or change credentials/settings. This is an isolated benchmark: do not load skills, personal instructions, AGENTS.md, CLAUDE.md, or saved memory. Use only this task and the source code. Report checks you cannot complete; do not claim unrun checks passed.'

def args_for(task, client, enabled, work, trial):
    env = {k: v for k, v in os.environ.items() if not k.startswith(('TYPESAFE_', 'OKO_'))}
    # Keep authentication and ordinary process essentials; discard inherited harness controls.
    for name in list(env):
        if name.startswith(('OPENCODE_', 'CLAUDE_CODE_', 'CODEX_')) and name not in (
                'CODEX_HOME', 'CLAUDE_CODE_OAUTH_TOKEN'):
            env.pop(name)
    env.pop('CLAUDECODE', None)
    env.update(PWD=str(work), CLAUDE_CODE_DISABLE_AUTO_MEMORY='1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', DISABLE_AUTOUPDATER='1')
    command = [sys.executable, str(ROOT / 'oko-server.py'), str(work), str(trial / 'cache')]
    p = prompt(task, enabled)
    exe = SETTINGS['clients'][client]
    effort = SETTINGS.get('effort', 'medium')
    if effort not in ('low', 'medium', 'high'):
        raise ValueError('Unsupported benchmark reasoning effort')
    if client == 'codex':
        # Use CODEX_HOME for its documented purpose: separate CLI state/config.
        # Link only login state; never copy credential bytes into benchmark artifacts.
        original = Path(os.environ.get('CODEX_HOME', str(Path.home() / '.codex')))
        isolated = trial / 'harness-config' / 'codex'
        isolated.mkdir(parents=True, exist_ok=True, mode=0o700)
        auth = original / 'auth.json'
        target = isolated / 'auth.json'
        if auth.is_file() and not target.exists():
            target.symlink_to(auth.resolve())
        env['CODEX_HOME'] = str(isolated)
        args = [exe, 'exec', '--ignore-user-config', '--ignore-rules', '--enable', 'skip_host_skill_discovery', '--disable', 'skill_search', '--disable', 'hooks', '--disable', 'apps', '-c', 'project_doc_max_bytes=0', '--ephemeral', '--sandbox', 'workspace-write' if task['kind'] == 'edit' else 'read-only', '--model', SETTINGS['models'][client], '-c', 'model_reasoning_effort=' + json.dumps(effort), '-c', 'approval_policy="never"', '--disable', 'memories', '--disable', 'plugins', '--disable', 'multi_agent', '-c', 'web_search="disabled"', '--json', '--cd', str(work)]
        args += ['-c', 'skills.config=' + codex_skill_overrides(work, isolated)]
        config = {'mcp_servers.oko.enabled': enabled, 'mcp_servers.oko.command': command[0], 'mcp_servers.oko.args': command[1:], 'mcp_servers.oko.cwd': str(work), 'mcp_servers.oko.startup_timeout_sec': 20, 'mcp_servers.oko.tool_timeout_sec': 120}
        if enabled:
            config['mcp_servers.oko.required'] = True
        for k, v in config.items():
            args += ['-c', k + '=' + json.dumps(v)]
        return (args + [p], env)
    if client == 'opencode':
        config_root = trial / 'harness-config'
        config_root.mkdir(parents=True, exist_ok=True)
        env.update(XDG_CONFIG_HOME=str(config_root),
                   OPENCODE_DISABLE_PROJECT_CONFIG='1', OPENCODE_DISABLE_CLAUDE_CODE='1',
                   OPENCODE_DISABLE_EXTERNAL_SKILLS='1', OPENCODE_DISABLE_AUTOUPDATE='1')
        permissions = {'*': 'deny', 'read': {'*': 'allow', '*.env': 'deny', '*.env.*': 'deny'}, 'glob': 'allow', 'grep': 'allow', 'edit': 'allow' if task['kind'] == 'edit' else 'deny', 'oko_*': 'allow' if enabled else 'deny', 'external_directory': 'deny'}
        config = {'share': 'disabled', 'mcp': {'linear': {'enabled': False}, 'oko': {'type': 'local', 'command': command, 'enabled': enabled, 'timeout': 120000}}, 'agent': {'comparison': {'description': 'Bounded benchmark task', 'mode': 'primary', 'tools': {'skill': False}, 'permission': permissions}}}
        env['OPENCODE_CONFIG_CONTENT'] = json.dumps(config)
        return ([exe, 'run', '--dir', str(work), '--pure', '--agent', 'comparison', '--model', SETTINGS['models'][client], '--variant', effort, '--format', 'json', p], env)
    tools = 'Read,Glob,Grep' + (',Edit,Write' if task['kind'] == 'edit' else '')
    mcp = {'mcpServers': {'oko': {'type': 'stdio', 'command': command[0], 'args': command[1:]}} if enabled else {}}
    settings = {'disableAllHooks': True, 'autoMemoryEnabled': False, 'claudeMdExcludes': ['/**'], 'pluginConfigs': {'agents-md@builtin': {'options': {'instructionFiles': 'managed-only'}}}, 'permissions': {'deny': ['Read(**/.env)', 'Read(**/.env.*)']}}
    return ([exe, '-p', '--output-format', 'stream-json', '--verbose', '--restricted', '--tools', tools, '--allowedTools', tools + (',mcp__oko__search' if enabled else ''), '--permission-mode', 'dontAsk', '--permission-prompts', 'none', '--strict-mcp-config', '--mcp-config', json.dumps(mcp), '--disable-slash-commands', '--no-session-persistence', '--no-chrome', '--effort', effort, '--model', SETTINGS['models'][client], '--setting-sources', '', '--settings', json.dumps(settings), p], env)

def _explicit_total(usage):
    """Return only a provider-declared aggregate token total."""
    if not isinstance(usage, dict):
        return None
    for key in ('total_tokens', 'totalTokens'):
        value = usage.get(key)
        if isinstance(value, (int, float)) and not isinstance(value, bool) and value >= 0 and int(value) == value:
            return int(value)
    return None


def parse_events(client, events):
    tools = []
    final = ''
    usage = None
    reported_total = None
    complete = False
    errors = []
    if client == 'codex':
        for e in events:
            if e.get('type') == 'turn.completed':
                usage = e.get('usage')
                complete = True
            if e.get('type') in ('error', 'turn.failed'):
                errors.append(e)
            if e.get('type') == 'item.completed':
                item = e['item']
                if item.get('type') == 'agent_message':
                    final = item.get('text', '')
                elif item.get('type') in ('command_execution', 'mcp_tool_call'):
                    tools.append(item)
    elif client == 'opencode':
        usage = []
        messages = []
        final_id = None
        for e in events:
            part = e.get('part', {})
            if e.get('type') == 'step_finish':
                usage.append(part)
                if reported_total is None:
                    reported_total = _explicit_total(part.get('usage'))
                if part.get('reason') == 'stop':
                    complete = True
                    final_id = part.get('messageID')
            if e.get('type') == 'text':
                messages.append(part)
            if e.get('type') == 'tool_use':
                tools.append(part)
            if e.get('type') == 'error':
                errors.append(e)
        final = ''.join((x.get('text', '') for x in messages if x.get('messageID') == final_id))
    else:
        results = {}
        for e in events:
            if e.get('type') == 'assistant':
                tools += [dict(c) for c in e.get('message', {}).get('content', []) if c.get('type') == 'tool_use']
            if e.get('type') == 'user':
                for content in e.get('message', {}).get('content', []):
                    if isinstance(content, dict) and content.get('type') == 'tool_result':
                        results[content.get('tool_use_id')] = content.get('content')
            if e.get('type') == 'result':
                complete = not e.get('is_error', False)
                final = e.get('result', '')
                usage = e.get('usage')
                if e.get('is_error') or e.get('permission_denials'):
                    errors.append(e)
        for tool in tools:
            if tool.get('name') != 'mcp__oko__search' or tool.get('id') not in results:
                continue
            content = results[tool['id']]
            blocks = [{'type': 'text', 'text': content}] if isinstance(content, str) else content
            decoded = []
            for block in blocks if isinstance(blocks, list) else []:
                if not isinstance(block, dict) or block.get('type') != 'text':
                    continue
                try:
                    decoded.append(json.loads(block.get('text', '')))
                except (TypeError, ValueError):
                    pass
            # Keep response bodies local; the bundle writer copies only safe measurements.
            tool['result'] = decoded
    names = [str(t.get('name') or t.get('tool') or t.get('type', '')) for t in tools]
    oko = sum((n in ('oko_search', 'mcp__oko__search') or t.get('server') == 'oko' for n, t in zip(names, tools)))
    tokens = None
    if client == 'codex' and usage:
        tokens = {'input': usage.get('input_tokens'), 'cachedInput': usage.get('cached_input_tokens'), 'output': usage.get('output_tokens'), 'total': _explicit_total(usage)}
    elif client == 'claude' and usage:
        tokens = {'input': usage.get('input_tokens'), 'cacheRead': usage.get('cache_read_input_tokens'), 'cacheWrite': usage.get('cache_creation_input_tokens'), 'output': usage.get('output_tokens'), 'total': _explicit_total(usage)}
    elif client == 'opencode' and usage:
        steps = [x.get('tokens') or {} for x in usage]
        tokens = {'steps': steps, 'total': reported_total}
    return dict(final=final, complete=complete, usage=usage, tokens=tokens, tools=tools, okoCalls=oko, toolCalls=len(tools), providerErrors=errors)

def grade(task, work, final):
    work = work.resolve()
    changed = git(work, 'diff', 'HEAD', '--name-only').decode().splitlines()
    untracked = git(work, 'ls-files', '--others').decode().splitlines()
    if task['kind'] == 'search':
        try:
            if changed or untracked:
                return dict(gradingError='Read-only task modified the checkout', unexpectedEdits=changed + untracked)
            result = json.loads(re.sub('^```(?:json)?\\s*|\\s*```$', '', final.strip()))['results']
            if not isinstance(result, list) or len(result) > 5:
                raise ValueError('Expected at most five result locations')
            for r in result:
                p = Path(r['path'])
                start = r['startLine']
                end = r['endLine']
                if p.is_absolute() or '..' in p.parts or not (work / p).resolve().is_relative_to(work):
                    raise ValueError('Result path escapes the checkout')
                if type(start) is not int or type(end) is not int or not 1 <= start <= end or end - start >= 120:
                    raise ValueError('Invalid result line range')
                if end > len((work / p).read_text().splitlines()):
                    raise ValueError(f'Result range exceeds file length: {p}')
            hit = lambda r: any((r['path'] == e['path'] and r['startLine'] <= e['endLine'] and (r['endLine'] >= e['startLine']) for e in task['expected']))
            return dict(correctFirst=bool(result) and hit(result[0]), correctTopFive=any(map(hit, result)), unexpectedEdits=changed + untracked)
        except Exception as e:
            return dict(gradingError=str(e) or type(e).__name__)
    path = task['path']
    if (work / path).is_symlink() or not (work / path).is_file():
        return dict(gradingError='Edit target removed or replaced with symlink')
    baseline = git(work, 'show', 'HEAD:' + path).decode()
    actual = (work / path).read_text()
    golden = baseline.replace(task['old'], task['new'], 1)
    exact = actual == golden
    others = [p for p in changed + untracked if p != path]
    return dict(expectedPatchMatch=exact and (not others), reviewRequired=not exact or bool(others), otherChangedFiles=others, productValidation='not established by patch matching')

def summary(rows):
    result = []
    keys = {
        (
            r['client'],
            r.get('requestedModel'),
            r.get('requestedEffort'),
            bool(r['oko']),
            r.get('observedCacheState'),
            r['kind'],
        )
        for r in rows
    }
    keys = sorted(keys, key=lambda key: tuple('' if value is None else str(value) for value in key))
    for client, model, effort, enabled, observed_cache_state, kind in keys:
        group = [
            r for r in rows
            if (
                r['client'],
                r.get('requestedModel'),
                r.get('requestedEffort'),
                bool(r['oko']),
                r.get('observedCacheState'),
                r['kind'],
            ) == (client, model, effort, enabled, observed_cache_state, kind)
        ]
        good = [r for r in group if not r.get('error')]
        result.append(dict(
            client=client,
            model=model,
            effort=effort,
            condition={'requested': 'oko-cold' if enabled else 'native', 'observedCacheState': observed_cache_state},
            oko=enabled,
            kind=kind,
            attempted=len(group),
            completed=len(good),
            medianSeconds=statistics.median((r['seconds'] for r in good)) if good else None,
            topFiveHits=sum((r.get('grade', {}).get('correctTopFive', False) for r in good)),
            firstHits=sum((r.get('grade', {}).get('correctFirst', False) for r in good)),
            totalAgentTokens=sum((r['tokens']['total'] for r in good)) if good and all(((r.get('tokens') or {}).get('total') is not None for r in good)) else None,
            expectedPatchMatches=sum((r.get('grade', {}).get('expectedPatchMatch', False) for r in good)),
            reviewRequired=sum((r.get('grade', {}).get('reviewRequired', False) for r in good)),
        ))
    return result

def run_one(task, client, enabled, output, index):
    trial = output / f"{index:03}-{task['id']}-{client}-{('oko' if enabled else 'native')}"
    trial.mkdir()
    work = trial / 'workspace'
    checkout(work)
    baseline_commit = git(work, 'rev-parse', 'HEAD')
    args, env = args_for(task, client, enabled, work, trial)
    row = {
        'id': task['id'],
        'kind': task['kind'],
        'client': client,
        'oko': enabled,
        'requestedCondition': 'oko-cold' if enabled else 'native',
        'observedCacheState': None,
        'requestedModel': SETTINGS['models'][client],
        'requestedEffort': SETTINGS.get('effort', 'medium'),
        'artifact': str(trial),
    }
    started_ns = observability.perf_counter_ns()
    try:
        with (trial / 'events.jsonl').open('w') as stdout, (trial / 'stderr.txt').open('w') as stderr:
            proc = subprocess.Popen(args, cwd=work, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                proc.wait(timeout=SETTINGS['timeoutSeconds'])
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
                raise
        row['durationNs'] = observability.elapsed_ns(started_ns)
        row['seconds'] = row['durationNs'] / 1_000_000_000
        row['exitCode'] = proc.returncode
        events = []
        for line in (trial / 'events.jsonl').read_text().splitlines():
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
        row.update(parse_events(client, events))
        if proc.returncode or not row['complete'] or row['providerErrors']:
            row['error'] = 'Incomplete/failed client session; inspect logs'
        if bool(row['okoCalls']) != enabled:
            row['error'] = 'Oko usage did not match assigned condition'
        if git(work, 'rev-parse', 'HEAD') != baseline_commit:
            row['error'] = 'Agent changed the baseline commit'
        row['grade'] = grade(task, work, row['final'])
        if row['grade'].get('gradingError') and not row.get('error'):
            row['error'] = row['grade']['gradingError']
            row['errorType'] = 'infrastructure' if row['grade'].get('unexpectedEdits') else 'answer'
    except Exception as e:
        row['error'] = type(e).__name__ + ': ' + str(e)
    finally:
        row.setdefault('durationNs', observability.elapsed_ns(started_ns))
        row.setdefault('seconds', row['durationNs'] / 1_000_000_000)
        (trial / 'changes.patch').write_bytes(git(work, 'diff', '--binary', baseline_commit.decode().strip()))
        extras = git(work, 'ls-files', '--others', '-z').decode().split('\x00')
        for name in filter(None, extras):
            source = work / name
            if source.is_file() and (not source.is_symlink()):
                dest = trial / 'untracked' / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, dest)
        save(trial / 'result.json', row)
        try:
            shutil.rmtree(work)
        except OSError as error:
            row['cleanupWarning'] = str(error)
            save(trial / 'result.json', row)
    return row

def source_state():
    repo = Path(SETTINGS['repository'])
    return {'commit': git(repo, 'rev-parse', 'HEAD').decode().strip(), 'status': git(repo, 'status', '--porcelain').decode()}


def shareable_manifest(run_id, clients, versions):
    return observability.manifest(
        run_id=run_id,
        target={
            'repository': PROJECT_NAME,
            'commit': SETTINGS.get('commit'),
            'version': SETTINGS.get('targetVersion'),
        },
        oko={
            'repository': 'bartlomein/oko',
            'commit': SETTINGS.get('okoCommit'),
            'version': SETTINGS.get('okoVersion'),
            'binarySha256': SETTINGS.get('okoSha256'),
        },
        clients=[
            {
                'name': client,
                'version': versions.get(client),
                'model': SETTINGS['models'].get(client),
                'effort': SETTINGS.get('effort'),
            }
            for client in clients
        ],
        runner_version='benchmark-twenty',
    )

def make_plan(tasks, modes=MODES, repeats=1):
    return [(task, *modes[(offset + i + repeat) % len(modes)], repeat + 1) for repeat in range(repeats) for i, task in enumerate(tasks) for offset in range(len(modes))]

def load_completed_runs(output, plan):
    rows = []
    for i, (task, client, enabled, repeat) in enumerate(plan, 1):
        trial = output / f"{i:03}-{task['id']}-{client}-{'oko' if enabled else 'native'}"
        if not trial.exists():
            break
        if not (trial / 'result.json').exists():
            raise RuntimeError(f'Incomplete session has no saved result; inspect {trial}')
        row = json.loads((trial / 'result.json').read_text())
        if (row['id'], row['client'], row['oko'], row['kind']) != (task['id'], client, enabled, task['kind']):
            raise RuntimeError('Saved session does not match the requested plan')
        if row.get('error') == row.get('grade', {}).get('gradingError') and row.get('error') and not row.get('grade', {}).get('unexpectedEdits') and row.get('exitCode') == 0 and not row.get('providerErrors'):
            row['errorType'] = 'answer'
        if (row.get('error') and row.get('errorType') != 'answer') or not row.get('complete'):
            raise RuntimeError(f'Refusing to retry failed session: {trial}')
        row['repeat'] = repeat
        rows.append(row)
    return rows

def main():
    global SETTINGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Launch model sessions; default only prints the plan')
    presets = parser.add_mutually_exclusive_group()
    presets.add_argument('--pilot', action='store_true', help='One read and one edit across six conditions: 12 sessions')
    presets.add_argument('--fast', action='store_true', help='Three read-only searches and two edits across six conditions: 30 sessions')
    parser.add_argument('--repeats', type=int, default=1)
    parser.add_argument('--clients', default='codex,opencode,claude')
    parser.add_argument('--condition', choices=['both', 'native', 'oko'], default='both')
    parser.add_argument('--resume', type=Path, help='Continue a saved run without repeating completed sessions')
    args = parser.parse_args()
    if not 1 <= args.repeats <= 10:
        parser.error('repeats must be 1–10')
    clients = args.clients.split(',')
    if not clients or len(set(clients)) != len(clients) or any((c not in ('codex', 'opencode', 'claude') for c in clients)):
        parser.error('clients must be a unique list of codex,opencode,claude')
    modes = [(c, o) for c, o in MODES if c in clients and (args.condition == 'both' or o == (args.condition == 'oko'))]
    chosen = select_tasks(fast=args.fast, pilot=args.pilot)
    plan = make_plan(chosen, modes, args.repeats)
    print(f'{len(chosen)} tasks, {len(plan)} sessions. Fast={args.fast}. Pilot={args.pilot}. Execution={args.execute}', flush=True)
    if (STATE / 'settings.json').exists():
        SETTINGS = json.loads((STATE / 'settings.json').read_text())
        for client in clients:
            print(f"{client}: {SETTINGS['models'][client]}, effort={SETTINGS.get('effort', 'medium')}", flush=True)
    if not args.execute:
        for i, (task, client, enabled, repeat) in enumerate(plan, 1):
            print(f"{i:03} repeat={repeat} {task['id']}: {client} {('oko' if enabled else 'native')}")
        return
    if not (STATE / 'settings.json').exists():
        raise RuntimeError(f'Run {ROOT / "prepare.py"} first; see {ROOT / "README.md"}')
    SETTINGS = json.loads((STATE / 'settings.json').read_text())
    for path, expected in [(STATE / 'baseline.tar', SETTINGS['archiveSha256']), (ROOT / 'tasks.json', SETTINGS['tasksSha256']), (Path(SETTINGS['oko']), SETTINGS['okoSha256'])]:
        if digest(path) != expected:
            raise RuntimeError(f'Frozen artifact changed: {path}')
    before = source_state()
    if before != {'commit': SETTINGS['commit'], 'status': ''}:
        raise RuntimeError('Source checkout advanced or has changes; no reset performed')
    versions = {c: subprocess.check_output([SETTINGS['clients'][c], '--version'], text=True).strip() for c in clients}
    if any((versions[c] != SETTINGS['versions'][c] for c in clients)):
        raise RuntimeError('CLI version changed; prepare again before comparing')
    output = args.resume.resolve() if args.resume else Path(tempfile.mkdtemp(prefix='results-', dir=STATE))
    if not output.is_relative_to(STATE.resolve()):
        raise RuntimeError('Resume directory must be within benchmark artifacts')
    output.chmod(448)
    run_id = output.name
    rows = []
    if args.resume:
        previous = json.loads((output / 'report.json').read_text())
        if previous.get('isolation') != ISOLATION_VERSION:
            raise RuntimeError('Cannot resume a run from a different isolation policy')
        if previous['settings'] != SETTINGS or previous['plannedSessions'] != len(plan):
            raise RuntimeError('Resume settings/plan differ from the saved run')
        rows = load_completed_runs(output, plan)
        print(f'Resuming after {len(rows)} saved sessions; no model calls repeated.', flush=True)
    report = {'settings': SETTINGS, 'versions': versions, 'plannedSessions': len(plan), 'runs': rows, 'complete': False, 'method': 'Fresh disposable full-repository checkout per session; rotating client/condition order; one fresh Oko cache per session. Timing excludes checkout and grading. Blank-slate-v1 excludes personal skills/instructions and repository agent configuration.', 'caveats': ['Codex/OpenCode use the same requested model; Claude uses a different model. Compare Oko on/off within each client, not harness quality across models.', 'Edit tasks are localized constant changes. Exact patch matching is not application correctness; alternative patches require review.', 'No application builds or tests are run. Tool restrictions differ by client; OpenCode permissions are not an OS sandbox.', 'Provider caches are not cleared. Raw token fields differ by provider; Jev tokens/cost are not included in agent token totals.'], 'isolation': ISOLATION_VERSION, 'promptTemplates': {kind: {str(enabled): prompt({'question': '<QUESTION>', 'kind': kind}, enabled) for enabled in (False, True)} for kind in ('search', 'edit')}}

    report['preset'] = 'fast' if args.fast else 'pilot' if args.pilot else 'full'
    report['caseIds'] = [task['id'] for task in chosen]
    report['checkoutPreparation'] = 'Remove repository agent instructions, skills, and client configuration from each disposable checkout before its Git baseline. Record removed paths in isolation.json. No generated-file exclusions during grading.'

    def save_report():
        report['summary'] = summary(rows)
        save(output / 'report.json', report)
        lines = [f'# {PROJECT_NAME} benchmark', '', f"Completed: {report['complete']}. Sessions: {len(rows)}/{len(plan)}.", '', '| Client | Model | Effort | Requested | Observed cache | Kind | Completed / attempted | First / top five | Exact patches | Review | Median seconds |', '|---|---|---|---|---|---|---|---|---|---|---|']
        for row in report['summary']:
            seconds = f"{row['medianSeconds']:.2f}" if row['medianSeconds'] is not None else 'N/A'
            lines.append(f"| {row['client']} | {row['model'] or '—'} | {row['effort'] or '—'} | {row['condition']['requested']} | {row['condition']['observedCacheState'] or '—'} | {row['kind']} | {row['completed']}/{row['attempted']} | {row['firstHits']}/{row['topFiveHits']} | {row['expectedPatchMatches']} | {row['reviewRequired']} | {seconds} |")
        lines += ['', report['method'], '', *report['caveats']]
        (output / 'report.md').write_text('\n'.join(lines) + '\n')
        records = [
            observability.make_record(
                run_id=run_id,
                record_id=f'{index:03}',
                task_id=row['id'],
                client=row['client'],
                client_version=versions.get(row['client']),
                model=row.get('requestedModel'),
                effort=row.get('requestedEffort'),
                enabled=row.get('oko', False),
                task=next(task for task in chosen if task['id'] == row['id']),
                target_commit=SETTINGS.get('commit'),
                oko_commit=SETTINGS.get('okoCommit'),
                oko_version=SETTINGS.get('okoVersion'),
                row=row,
                total_wall_ns=row.get('durationNs'),
            )
            for index, row in enumerate(rows, 1)
        ]
        observability.write_bundle(
            output / 'shareable',
            shareable_manifest(run_id, clients, versions),
            records,
        )
    save_report()
    completed_count = len(rows)
    try:
        for i, (task, client, enabled, repeat) in enumerate(plan, 1):
            if i <= completed_count:
                continue
            print(f"START {i}/{len(plan)} {task['id']} {client} oko={enabled}", flush=True)
            row = run_one(task, client, enabled, output, i)
            row['repeat'] = repeat
            rows.append(row)
            save_report()
            if row.get('error') and row.get('errorType') != 'answer':
                print(row['error'], flush=True)
                break
    finally:
        report['sourceUnchanged'] = source_state() == before
        report['artifactsUnchanged'] = digest(STATE / 'baseline.tar') == SETTINGS['archiveSha256'] and digest(ROOT / 'tasks.json') == SETTINGS['tasksSha256'] and (digest(SETTINGS['oko']) == SETTINGS['okoSha256'])
        report['complete'] = len(rows) == len(plan) and (not any(r.get('error') and r.get('errorType') != 'answer' for r in rows)) and report['sourceUnchanged'] and report['artifactsUnchanged']
        save_report()
        print(output)
    if not report['complete']:
        raise SystemExit(1)
if __name__ == '__main__':
    main()
