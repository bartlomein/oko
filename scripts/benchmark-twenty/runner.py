#!/usr/bin/env python3
"""Twenty benchmark (10 read-only, 5 edits). Defaults to plan-only; --execute is required for model calls."""
import argparse, hashlib, json, os, re, shutil, signal, statistics, subprocess, sys, tarfile, tempfile
from collections.abc import Mapping
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import benchmark_observability as observability

ROOT = Path(__file__).resolve().parent
PROJECT_NAME = 'Twenty'
STATE = ROOT.parents[1] / 'benchmarks/results/twenty'
SETTINGS = {}
TASKS = json.loads((ROOT / 'tasks.json').read_text())['tasks']
CONDITIONS = ('native', 'oko-cold', 'oko-warm')
MODES = [(client, condition) for client in ('codex', 'opencode', 'claude') for condition in CONDITIONS]
FAST_TASK_IDS = ('rich-text-preview', 'metadata-pagination', 'locale-direction', 'preview-debounce', 'email-retry-delay')
PILOT_TASK_IDS = ('search-normalization', 'email-retry-delay')
CANARY_REPEATS = 1
CACHE_POLICY = 'cold-vs-prebuilt-disk-v1'

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

def condition_name(condition):
    if condition is True:
        return 'oko-cold'
    if condition is False:
        return 'native'
    if condition not in CONDITIONS:
        raise ValueError(f'Unsupported benchmark condition: {condition}')
    return condition


def condition_enabled(condition):
    return condition_name(condition) != 'native'


def args_for(task, client, condition, work, trial):
    condition = condition_name(condition)
    enabled = condition_enabled(condition)
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


def _decode_payload(value):
    if isinstance(value, (dict, list)):
        return value
    if isinstance(value, str):
        try:
            return json.loads(value)
        except (TypeError, ValueError):
            return None
    return None


def _safe_usage(value):
    if not isinstance(value, dict):
        return None
    return observability.normalize_usage(value)


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
                tool = dict(part)
                state = tool.get('state') if isinstance(tool.get('state'), dict) else {}
                output = state.get('output')
                if output is not None:
                    decoded = _decode_payload(output)
                    tool['result'] = [] if decoded is None else [decoded]
                tools.append(tool)
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
            if isinstance(content, Mapping) and content.get('is_error'):
                tool['result'] = []
                continue
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
    agent_usage_steps = []
    for step in usage if isinstance(usage, list) else []:
        token_usage = _safe_usage(step.get('tokens')) if isinstance(step, dict) else None
        declared_usage = _safe_usage(step.get('usage')) if isinstance(step, dict) else None
        if token_usage or declared_usage:
            merged = dict(token_usage or {})
            for key, value in (declared_usage or {}).items():
                if value is not None:
                    merged[key] = value
            agent_usage_steps.append(merged)
    agent_usage = _safe_usage(usage)
    if agent_usage is None and len(agent_usage_steps) == 1:
        agent_usage = agent_usage_steps[0]
    return dict(final=final, complete=complete, usage=usage, tokens=tokens,
                agentUsage=agent_usage, agentUsageSteps=agent_usage_steps or None,
                tools=tools, okoCalls=oko, toolCalls=len(tools), providerErrors=errors)


def cache_observations(row):
    def packets(value):
        if isinstance(value, str):
            decoded = _decode_payload(value)
            return packets(decoded) if decoded is not None else []
        if isinstance(value, Mapping):
            timings = value.get('timings')
            if isinstance(timings, Mapping) and isinstance(timings.get('cache'), Mapping):
                result = [dict(timings['cache'])]
            elif value.get('event') == 'prewarm' and isinstance(value.get('cache'), Mapping):
                result = [dict(value['cache'])]
            else:
                result = []
            for child in value.values():
                result.extend(packets(child))
            return result
        if isinstance(value, list):
            result = []
            for child in value:
                result.extend(packets(child))
            return result
        return []

    observations = []
    for tool in row.get('tools', []):
        if observability.is_oko_tool(tool):
            # Startup preparation decides the session's cache state; the search
            # that follows it is a memory hit. Older results embedded metadata.
            observations.extend(packets(tool.get('okoPrewarm')))
            observations.extend(packets({k: v for k, v in tool.items() if k != 'okoPrewarm'}))
    unique = []
    seen = set()
    for observation in observations:
        marker = json.dumps(observation, sort_keys=True, separators=(',', ':'))
        if marker not in seen:
            seen.add(marker)
            unique.append(observation)
    return unique


def check_condition(condition, observations, oko_calls=0):
    condition = condition_name(condition)
    if condition == 'native':
        return not observations and oko_calls == 0
    if not observations:
        return False
    first = observations[0]
    if condition == 'oko-cold':
        return first.get('status') == 'cold'
    return (first.get('status') == 'disk'
            and first.get('rebuiltFiles') == 0
            and first.get('reusedFiles', 0) > 0)


def prewarm(work, trial):
    """Build the exact disposable workspace cache outside the timed session."""
    cache = trial / 'cache'
    if cache.exists():
        raise RuntimeError('Warm condition requires a fresh cache directory')
    env = {k: v for k, v in os.environ.items() if not k.startswith(('TYPESAFE_', 'OKO_'))}
    env.update(OKO_CACHE_DIR=str(cache), OKO_NO_CACHE='0', OKO_RIPGREP=SETTINGS.get('rg', ''))
    started = observability.perf_counter_ns()
    proc = subprocess.run(
        [SETTINGS['oko'], 'ask', '--no-jev', '--json', 'project source overview'],
        cwd=work,
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if proc.returncode:
        raise RuntimeError('Warm cache preparation failed')
    try:
        payload = json.loads(proc.stdout)
        cache_metadata = payload['cache']
    except (ValueError, KeyError, TypeError) as error:
        raise RuntimeError('Warm cache preparation returned invalid Oko metadata') from error
    if cache_metadata.get('status') != 'cold':
        raise RuntimeError('Warm cache preparation did not observe cold')
    warmup = {
        'durationNs': observability.elapsed_ns(started),
        'cache': cache_metadata,
        'providerCalls': 0,
        'method': 'Task-independent lexical query; exact disposable workspace prebuilt to disk',
    }
    save(trial / 'warmup.json', warmup)
    return warmup

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
            condition_name(r.get('condition', 'oko-cold' if r.get('oko') else 'native')),
            r.get('observedCacheState'),
            r.get('taskKind', r['kind']),
        )
        for r in rows
    }
    keys = sorted(keys, key=lambda key: tuple('' if value is None else str(value) for value in key))
    for client, model, effort, condition, observed_cache_state, task_kind in keys:
        group = [
            r for r in rows
            if (
                r['client'],
                r.get('requestedModel'),
                r.get('requestedEffort'),
                condition_name(r.get('condition', 'oko-cold' if r.get('oko') else 'native')),
                r.get('observedCacheState'),
                r.get('taskKind', r['kind']),
            ) == (client, model, effort, condition, observed_cache_state, task_kind)
        ]
        good = [r for r in group if not r.get('error')]
        result.append(dict(
            client=client,
            model=model,
            effort=effort,
            condition={'requested': condition, 'observedCacheState': observed_cache_state},
            oko=condition != 'native',
            kind=task_kind,
            taskKind=task_kind,
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

def run_one(task, client, condition, output, index):
    condition = condition_name(condition)
    enabled = condition_enabled(condition)
    trial = output / f"{index:03}-{task['id']}-{client}-{condition}"
    trial.mkdir()
    work = trial / 'workspace'
    checkout(work)
    baseline_commit = git(work, 'rev-parse', 'HEAD')
    row = {
        'id': task['id'],
        'kind': task['kind'],
        'taskKind': task['kind'],
        'client': client,
        'oko': enabled,
        'condition': condition,
        'requestedCondition': condition,
        'observedCacheState': None,
        'requestedModel': SETTINGS['models'][client],
        'requestedEffort': SETTINGS.get('effort', 'medium'),
        'artifact': str(trial),
    }
    started_ns = None
    try:
        if condition == 'oko-warm':
            row['warmup'] = prewarm(work, trial)
        args, env = args_for(task, client, condition, work, trial)
        started_ns = observability.perf_counter_ns()
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
        observability.attach_oko_metrics(row['tools'], trial / 'oko-metrics.jsonl')
        row['cacheObservations'] = cache_observations(row)
        row['observedCacheState'] = (
            'native' if condition == 'native'
            else row['cacheObservations'][0].get('status') if row['cacheObservations'] else None
        )
        if proc.returncode or not row['complete'] or row['providerErrors']:
            row['error'] = 'Incomplete/failed client session; inspect logs'
        if not check_condition(condition, row['cacheObservations'], row.get('okoCalls', 0)):
            row['error'] = f'Observed Oko cache state did not match {condition}'
            row['errorType'] = 'infrastructure'
        if git(work, 'rev-parse', 'HEAD') != baseline_commit:
            row['error'] = 'Agent changed the baseline commit'
        row['grade'] = grade(task, work, row['final'])
        if row['grade'].get('gradingError') and not row.get('error'):
            row['error'] = row['grade']['gradingError']
            row['errorType'] = 'infrastructure' if row['grade'].get('unexpectedEdits') else 'answer'
    except Exception as e:
        row['error'] = type(e).__name__ + ': ' + str(e)
    finally:
        row.setdefault('durationNs', observability.elapsed_ns(started_ns) if started_ns is not None else 0)
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
    for i, (task, client, condition, repeat) in enumerate(plan, 1):
        condition = condition_name(condition)
        trial = output / f"{i:03}-{task['id']}-{client}-{condition}"
        if not trial.exists():
            break
        if not (trial / 'result.json').exists():
            raise RuntimeError(f'Incomplete session has no saved result; inspect {trial}')
        row = json.loads((trial / 'result.json').read_text())
        if (row['id'], row['client'], row.get('condition'), row['kind']) != (task['id'], client, condition, task['kind']):
            raise RuntimeError('Saved session does not match the requested plan')
        if row.get('error') == row.get('grade', {}).get('gradingError') and row.get('error') and not row.get('grade', {}).get('unexpectedEdits') and row.get('exitCode') == 0 and not row.get('providerErrors'):
            row['errorType'] = 'answer'
        if (row.get('error') and row.get('errorType') != 'answer') or not row.get('complete'):
            raise RuntimeError(f'Refusing to retry failed session: {trial}')
        row['repeat'] = repeat
        rows.append(row)
    return rows


def shareable_settings(preset, clients, conditions, repeats, task_ids):
    return {
        'preset': preset,
        'clients': list(clients),
        'conditions': list(conditions),
        'repeats': repeats,
        'taskIds': list(task_ids),
        'models': {client: SETTINGS['models'].get(client) for client in clients},
        'effort': SETTINGS.get('effort', 'medium'),
        'timeoutSeconds': SETTINGS.get('timeoutSeconds'),
        'isolation': ISOLATION_VERSION,
        'cachePolicy': CACHE_POLICY,
    }


def records_for_rows(rows, tasks, versions, run_id):
    by_id = {task['id']: task for task in tasks}
    return [
        observability.make_record(
            run_id=run_id,
            record_id=f'{index:03}',
            task_id=row['id'],
            client=row['client'],
            client_version=versions.get(row['client']),
            model=row.get('requestedModel'),
            effort=row.get('requestedEffort'),
            enabled=row.get('oko', False),
            task=by_id[row['id']],
            target_commit=SETTINGS.get('commit'),
            oko_commit=SETTINGS.get('okoCommit'),
            oko_version=SETTINGS.get('okoVersion'),
            row=row,
            total_wall_ns=row.get('durationNs'),
        )
        for index, row in enumerate(rows, 1)
    ]


def canary_result(rows, tasks, clients, versions, run_id):
    client_runs = all(row.get('complete') and not row.get('providerErrors') for row in rows)
    condition_match = all(not str(row.get('error') or '').startswith('Observed Oko cache state') for row in rows)
    records = []
    metrics_present = True
    try:
        records = records_for_rows(rows, tasks, versions, run_id)
        metrics_present = all(
            bool(record['okoUsage']['phaseMetrics'])
            and bool(row.get('cacheObservations'))
            and _canary_oko_instrumentation_present(row, record)
            for row, record in zip(rows, records)
            if row.get('condition') != 'native'
        )
        envelope = observability.build_envelope(
            shareable_manifest(run_id, clients, versions),
            records,
            settings=shareable_settings('canary', clients, sorted({row['condition'] for row in rows}), CANARY_REPEATS, [task['id'] for task in tasks]),
            canary={
                'status': 'pilot-run',
                'plannedSessions': len(rows),
                'completedSessions': sum(bool(row.get('complete')) for row in rows),
                'checks': {
                    'completedClientRuns': client_runs,
                    'conditionMatch': condition_match,
                    'okoMetrics': metrics_present,
                    'artifactSchema': False,
                    'credentialsAndPaths': False,
                },
                'taskIds': [task['id'] for task in tasks],
                'conditions': sorted({row['condition'] for row in rows}),
            },
        )
        observability.validate_benchmark(envelope)
        schema_valid = True
        privacy_valid = True
    except (ValueError, observability.PrivacyError, KeyError, TypeError):
        schema_valid = False
        privacy_valid = False
    passed = client_runs and condition_match and metrics_present and schema_valid and privacy_valid
    return {
        'status': 'passed' if passed else 'failed',
        'plannedSessions': len(rows),
        'completedSessions': sum(bool(row.get('complete')) for row in rows),
        'checks': {
            'completedClientRuns': client_runs,
            'conditionMatch': condition_match,
            'okoMetrics': metrics_present,
            'artifactSchema': schema_valid,
            'credentialsAndPaths': privacy_valid,
        },
        'taskIds': [task['id'] for task in tasks],
        'conditions': sorted({row['condition'] for row in rows}),
    }


def _canary_oko_instrumentation_present(row, record):
    """Require call metadata and at least one safe provider observation.

    The invariant is intentionally not provider-call count == Oko MCP-call
    count: one Oko call may perform recovery/deep provider calls. Instead,
    the recorded Oko call count must be positive and fit within the agent's
    tool-call count, while the safe provider-call list must be non-empty and
    agree with the record's Jev count. Failed provider calls remain evidence.
    """
    oko_calls = row.get('okoCalls')
    tool_calls = row.get('toolCalls')
    recorded_calls = record.get('calls', {})
    provider_calls = record.get('okoUsage', {}).get('providerCalls', [])
    return (
        isinstance(oko_calls, int) and not isinstance(oko_calls, bool) and oko_calls > 0
        and isinstance(tool_calls, int) and not isinstance(tool_calls, bool)
        and tool_calls >= oko_calls
        and recorded_calls.get('okoCalls') == oko_calls
        and recorded_calls.get('agentToolCalls') == tool_calls
        and recorded_calls.get('jevCalls') == len(provider_calls) > 0
        and bool(provider_calls)
    )

def main():
    global SETTINGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Launch model sessions; default only prints the plan')
    presets = parser.add_mutually_exclusive_group()
    presets.add_argument('--pilot', action='store_true', help='One read and one edit across native, cold, and warm Oko: 18 sessions')
    presets.add_argument('--fast', action='store_true', help='Three read-only searches and two edits across native, cold, and warm Oko: 45 sessions')
    parser.add_argument('--canary-only', action='store_true', help='Run only the cost-conscious pilot canary and write benchmark.json')
    parser.add_argument('--repeats', type=int, default=1)
    parser.add_argument('--clients', default='codex,opencode,claude')
    parser.add_argument('--condition', choices=['both', *CONDITIONS], default='both')
    parser.add_argument('--resume', type=Path, help='Continue a saved run without repeating completed sessions')
    args = parser.parse_args()
    if not 1 <= args.repeats <= 10:
        parser.error('repeats must be 1–10')
    clients = args.clients.split(',')
    if not clients or len(set(clients)) != len(clients) or any((c not in ('codex', 'opencode', 'claude') for c in clients)):
        parser.error('clients must be a unique list of codex,opencode,claude')
    if args.canary_only and (args.fast or args.pilot):
        parser.error('--canary-only cannot be combined with --pilot or --fast')
    selected_conditions = CONDITIONS if args.condition == 'both' else (args.condition,)
    modes = [(c, condition) for c in clients for condition in selected_conditions]
    chosen = select_tasks(fast=args.fast, pilot=args.pilot or args.canary_only)
    plan = make_plan(chosen, modes, args.repeats)
    print(f'{len(chosen)} tasks, {len(plan)} sessions. Fast={args.fast}. Pilot={args.pilot}. CanaryOnly={args.canary_only}. Execution={args.execute}', flush=True)
    if (STATE / 'settings.json').exists():
        SETTINGS = json.loads((STATE / 'settings.json').read_text())
        for client in clients:
            print(f"{client}: {SETTINGS['models'][client]}, effort={SETTINGS.get('effort', 'medium')}", flush=True)
    if not args.execute:
        for i, (task, client, condition, repeat) in enumerate(plan, 1):
            print(f"{i:03} repeat={repeat} {task['id']}: {client} {condition}")
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
    canary = None
    if args.resume:
        previous = json.loads((output / 'report.json').read_text())
        if previous.get('isolation') != ISOLATION_VERSION:
            raise RuntimeError('Cannot resume a run from a different isolation policy')
        if previous['settings'] != SETTINGS or previous['plannedSessions'] != len(plan):
            raise RuntimeError('Resume settings/plan differ from the saved run')
        canary = previous.get('canary')
        if not args.pilot and (not canary or canary.get('status') != 'passed'):
            raise RuntimeError('Cannot resume a paid run without a passed canary')
        rows = load_completed_runs(output, plan)
        print(f'Resuming after {len(rows)} saved sessions; no model calls repeated.', flush=True)
    elif args.execute and (not args.pilot or args.canary_only):
        canary_tasks = select_tasks(pilot=True)
        canary_plan = make_plan(canary_tasks, modes, CANARY_REPEATS)
        canary_output = output / 'canary-runs'
        canary_output.mkdir(parents=True, exist_ok=True)
        canary_rows = []
        for index, (task, client, condition, repeat) in enumerate(canary_plan, 1):
            print(f"CANARY {index}/{len(canary_plan)} {task['id']} {client} {condition}", flush=True)
            row = run_one(task, client, condition, canary_output, index)
            row['repeat'] = repeat
            canary_rows.append(row)
        canary = canary_result(canary_rows, canary_tasks, clients, versions, run_id + '-canary')
        save(output / 'canary.json', canary)
        if canary['status'] != 'passed':
            observability.write_bundle(
                output / 'shareable',
                shareable_manifest(run_id + '-canary', clients, versions),
                records_for_rows(canary_rows, canary_tasks, versions, run_id + '-canary'),
                settings=shareable_settings('canary', clients, sorted({condition_name(condition) for _, condition in modes}), CANARY_REPEATS, [task['id'] for task in canary_tasks]),
                canary=canary,
                report='# Oko benchmark canary\n\nCanary failed; the main benchmark was not started.\n',
            )
            raise RuntimeError('Canary failed; main benchmark was not started')
        if args.canary_only:
            chosen = canary_tasks
            plan = canary_plan
            rows = canary_rows
    report = {'settings': SETTINGS, 'versions': versions, 'plannedSessions': len(plan), 'runs': rows, 'complete': False, 'canary': canary, 'method': 'Fresh disposable full-repository checkout per session; rotating client/condition order; native, cold Oko, and warm Oko conditions. Warm indexes are prebuilt in the exact disposable workspace and validated as disk-backed before timed client execution. Timing excludes checkout, warm preparation, and grading. Blank-slate-v1 excludes personal skills/instructions and repository agent configuration.', 'caveats': ['Codex/OpenCode use the same requested model; Claude uses a different model. Compare conditions within each client, not harness quality across models.', 'Edit tasks are localized constant changes. Exact patch matching is not application correctness; alternative patches require review.', 'No application builds or tests are run. Tool restrictions differ by client; OpenCode permissions are not an OS sandbox.', 'Provider caches are not cleared. Raw token fields differ by provider; Jev tokens/cost are not included in agent token totals.'], 'isolation': ISOLATION_VERSION, 'cachePolicy': CACHE_POLICY, 'promptTemplates': {kind: {str(condition): prompt({'question': '<QUESTION>', 'kind': kind}, condition != 'native') for condition in CONDITIONS} for kind in ('search', 'edit')}}

    report['preset'] = 'canary' if args.canary_only else 'fast' if args.fast else 'pilot' if args.pilot else 'full'
    report['caseIds'] = [task['id'] for task in chosen]
    report['checkoutPreparation'] = 'Remove repository agent instructions, skills, and client configuration from each disposable checkout before its Git baseline. Record removed paths in isolation.json. No generated-file exclusions during grading.'

    def save_report():
        nonlocal canary
        report['summary'] = summary(rows)
        if args.pilot:
            # Recompute from the full pilot, including when resuming older reports.
            canary = (canary_result(rows, chosen, clients, versions, run_id + '-canary')
                      if len(rows) == len(plan) else None)
            report['canary'] = canary
        save(output / 'report.json', report)
        lines = [f'# {PROJECT_NAME} benchmark', '', f"Completed: {report['complete']}. Sessions: {len(rows)}/{len(plan)}.", '', f"Canary: {canary['status'] if canary else 'not-run (pilot itself is the canary)'}.", '', '| Client | Model | Effort | Task kind | Requested | Observed cache | Completed / attempted | First / top five | Exact patches | Review | Median seconds |', '|---|---|---|---|---|---|---|---|---|---|---|']
        for row in report['summary']:
            seconds = f"{row['medianSeconds']:.2f}" if row['medianSeconds'] is not None else 'N/A'
            lines.append(f"| {row['client']} | {row['model'] or '—'} | {row['effort'] or '—'} | {row['taskKind']} | {row['condition']['requested']} | {row['condition']['observedCacheState'] or '—'} | {row['completed']}/{row['attempted']} | {row['firstHits']}/{row['topFiveHits']} | {row['expectedPatchMatches']} | {row['reviewRequired']} | {seconds} |")
        lines += ['', report['method'], '', *report['caveats']]
        (output / 'report.md').write_text('\n'.join(lines) + '\n')
        report_text = '\n'.join(lines) + '\n'
        records = records_for_rows(rows, chosen, versions, run_id)
        observability.write_bundle(
            output / 'shareable',
            shareable_manifest(run_id, clients, versions),
            records,
            settings=shareable_settings(report['preset'], clients, sorted({condition_name(condition) for _, condition in modes}), args.repeats, [task['id'] for task in chosen]),
            canary=canary,
            report=report_text,
        )
    save_report()
    completed_count = len(rows)
    try:
        for i, (task, client, condition, repeat) in enumerate(plan, 1):
            if i <= completed_count:
                continue
            print(f"START {i}/{len(plan)} {task['id']} {client} condition={condition}", flush=True)
            row = run_one(task, client, condition, output, i)
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
