"""Pi 1.0 CLI adapter: isolated sessions and authoritative JSON event accounting."""
import json
import os
from pathlib import Path
import sys


def args_for(task, condition, work, trial, settings, prompt, root):
    enabled = condition != 'native'
    # Start with process essentials and provider credentials, never inherited Pi
    # controls. The isolated agent directory links login state only.
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('PI_', 'TYPESAFE_', 'OKO_', 'CODEX_', 'OPENCODE_', 'CLAUDE_CODE_'))}
    agent = trial / 'harness-config' / 'pi'
    agent.mkdir(parents=True, exist_ok=True, mode=0o700)
    original = Path(os.environ.get('PI_CODING_AGENT_DIR', str(Path.home() / '.pi/agent')))
    auth = original / 'auth.json'
    if auth.is_file():
        (agent / 'auth.json').symlink_to(auth.resolve())
    # All other state is fresh; disable provider retries for auditable attempts.
    (agent / 'settings.json').write_text(json.dumps({'retry': {'enabled': False}, 'compaction': {'enabled': False}}))
    env.update(PI_CODING_AGENT_DIR=str(agent), PI_OFFLINE='1', PI_TELEMETRY='0', PWD=str(work))
    tools = ['read', 'grep', 'find', 'ls']
    if task['kind'] == 'edit':
        tools += ['edit', 'write']
    if enabled:
        tools.append('mcp__oko__search')
    args = [settings['clients']['pi'], '--mode', 'json', '--no-session',
            '--no-context-files', '--no-skills', '--no-prompt-templates', '--no-themes',
            '--no-extensions', '--model', settings['models']['pi'],
            '--thinking', settings.get('effort', 'low'), '--tools', ','.join(tools)]
    # The same guard applies to both sides, including canaries. No shell tool is
    # exposed; native grep/find/ls/read match the other clients' retrieval tools.
    guard = agent / 'scope.js'
    guard.write_text('''import { resolve, relative, isAbsolute, basename } from "node:path";
import { realpathSync } from "node:fs";
export default function(pi) {
  pi.on("tool_call", (event, ctx) => {
    if (event.toolName === "mcp__oko__search") return;
    const path = resolve(ctx.cwd, event.input.path || ".");
    let actual;
    try { actual = realpathSync(path); }
    catch { try { actual = resolve(realpathSync(resolve(path, "..")), basename(path)); } catch { actual = path; } }
    const rel = relative(realpathSync(ctx.cwd), actual);
    if (rel === ".." || rel.startsWith("../") || isAbsolute(rel) || basename(path).startsWith(".env"))
      return { block: true, reason: "Benchmark tools are scoped to this checkout." };
  });
}
''')
    args += ['-e', str(guard)]
    if enabled:
        command = [sys.executable, str(root / 'oko-server.py'), str(work), str(trial / 'cache')]
        (agent / 'mcp.json').write_text(json.dumps({'mcpServers': {'oko': {
            'command': command[0], 'args': command[1:], 'exposure': 'direct',
            'description': 'Find relevant code by intent and return bounded source excerpts.'}}}))
        args += ['-e', 'builtin:mcp']
        if task.get('cacheCondition') == 'prefetch':
            plugin = (root.parents[1] / 'src/pi_prefetch.js').read_text()
            plugin = plugin.replace('__OKO_COMMAND__', json.dumps(command)).replace('__OKO_ENV__', '{}')
            path = agent / 'oko-prefetch.js'
            path.write_text(plugin)
            args += ['-e', str(path)]
    if task.get('cacheCondition') in ('guided', 'prefetch'):
        args += ['--append-system-prompt', (root.parents[1] / 'src/guidance.md').read_text()]
    return args + [prompt], env


def parse_events(events):
    tools, messages, errors = [], [], []
    starts = {}
    settled = False
    for event in events:
        kind = event.get('type')
        if kind == 'agent_start':
            settled = False
        elif kind == 'agent_settled':
            settled = True
        elif kind == 'tool_execution_start':
            starts[event['toolCallId']] = event
        elif kind == 'tool_execution_end':
            start = starts.get(event.get('toolCallId'), {})
            tools.append({'id': event.get('toolCallId'), 'name': event.get('toolName'),
                          'input': start.get('args'), 'result': event.get('result'),
                          'isError': event.get('isError', False)})
        elif kind == 'message_end' and event.get('message', {}).get('role') == 'assistant':
            message = event['message']
            messages.append(message)
            if message.get('stopReason') in ('error', 'aborted'):
                errors.append({'stopReason': message['stopReason'], 'error': message.get('errorMessage')})
        elif kind == 'error':
            errors.append(event)
    final_message = messages[-1] if messages else {}
    final = ''.join(part.get('text', '') for part in final_message.get('content', []) if part.get('type') == 'text')
    usage = [m.get('usage') for m in messages]
    def summed(key):
        values = [u.get(key) if isinstance(u, dict) else None for u in usage]
        return sum(values) if values and all(isinstance(v, (int, float)) and not isinstance(v, bool) and v >= 0 for v in values) else None
    tokens = {key: summed(key) for key in ('input', 'output', 'cacheRead', 'cacheWrite')}
    tokens['total'] = summed('totalTokens')
    return dict(final=final, complete=settled and final_message.get('stopReason') == 'stop',
                usage=usage, tokens=tokens, agentUsage=None, agentUsageSteps=None,
                tools=tools, okoCalls=sum(t['name'] == 'mcp__oko__search' for t in tools),
                toolCalls=len(tools), providerErrors=errors)


def check_isolation(executable, root, state):
    """Capture a real Pi request at a localhost fake provider; no paid calls."""
    import http.server
    import subprocess
    import tempfile
    import threading
    if not executable:
        raise RuntimeError('Install Pi 1.0+ before preparing')
    requests = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            chunks = [
                {'id': 'probe', 'object': 'chat.completion.chunk', 'model': 'probe', 'choices': [{'index': 0, 'delta': {'role': 'assistant', 'content': 'READY'}, 'finish_reason': None}]},
                {'id': 'probe', 'object': 'chat.completion.chunk', 'model': 'probe', 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}], 'usage': {'prompt_tokens': 10, 'completion_tokens': 1, 'total_tokens': 11}},
            ]
            for chunk in chunks:
                self.wfile.write(('data: ' + json.dumps(chunk) + '\n\n').encode())
            self.wfile.write(b'data: [DONE]\n\n')
        def log_message(self, *args):
            pass
    with tempfile.TemporaryDirectory(prefix='oko-pi-isolation-') as temp:
        trial = Path(temp)
        work = trial / 'workspace'
        work.mkdir()
        sentinel = 'PI_BENCH_PERSONAL_INSTRUCTIONS_MUST_NOT_LOAD'
        (trial / 'AGENTS.md').write_text(sentinel)
        (work / '.pi/extensions').mkdir(parents=True)
        (work / '.pi/extensions/unwanted.js').write_text('throw new Error("' + sentinel + '");')
        (work / 'AGENTS.override.md').write_text(sentinel)
        settings = {'clients': {'pi': executable}, 'models': {'pi': 'bench/probe'}, 'effort': 'low'}
        args, env = args_for({'kind': 'search'}, 'native', work, trial, settings, 'Reply READY.', root)
        agent = Path(env['PI_CODING_AGENT_DIR'])
        # The probe has no reason to access real credentials, including symlinked auth.
        auth = agent / 'auth.json'
        if auth.is_symlink():
            auth.unlink()
        args[-1:-1] = ['--api-key', 'offline-probe']
        with http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            (agent / 'models.json').write_text(json.dumps({'providers': {'bench': {
                'baseUrl': f'http://127.0.0.1:{server.server_port}/v1', 'api': 'openai-completions',
                'models': [{'id': 'probe', 'name': 'probe', 'reasoning': False, 'input': ['text'],
                            'contextWindow': 8192, 'maxTokens': 32, 'cost': {'input': 0, 'output': 0, 'cacheRead': 0, 'cacheWrite': 0}}]}}}))
            try:
                result = subprocess.run(args, cwd=work, env=env, capture_output=True, text=True, timeout=30)
            finally:
                server.shutdown()
                thread.join()
        events = [json.loads(line) for line in result.stdout.split('\n') if line.startswith('{')]
        parsed = parse_events(events)
        raw = json.dumps(requests)
        if result.returncode or not parsed['complete'] or parsed['final'] != 'READY' or not requests or sentinel in raw or 'mcp__oko' in raw:
            raise RuntimeError('Pi isolation probe failed: ' + result.stderr[-1500:] + '\n' + result.stdout[-1500:])
        evidence = {'passed': True, 'method': 'pi-1.0-localhost-request-capture',
                    'requests': len(requests), 'personalContextAbsent': True, 'projectExtensionsAbsent': True,
                    'nativeOkoAbsent': True, 'limits': 'Configuration isolation, not an OS sandbox; paid seed/probe canary follows.'}
        (state / 'isolation-preflight.json').write_text(json.dumps(evidence, indent=2) + '\n')
        return evidence
