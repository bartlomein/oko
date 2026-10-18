#!/usr/bin/env python3
"""Run one prompt through Codex the way its VS Code extension does, and print what happened as JSON.

The extension (openai.chatgpt) starts its bundled `codex` as
`codex -c features.code_mode_host=true app-server --analytics-default-enabled`
with RUST_LOG=warn and CODEX_INTERNAL_ORIGINATOR_OVERRIDE=codex_vscode, then
speaks JSON-RPC, one JSON object per line, starting with `initialize` from a
client named "VS Code". This script does the same, starts a thread and one turn
in the project, and reports the hooks that ran (with the context they added),
the tools the agent used and its answer.

  codex_vscode.py --cwd PROJECT --prompt TEXT [--codex PATH] [--trust]

--codex defaults to the installed extension's binary, else a copy of the latest
extension from the Marketplace kept in ~/.cache/oko-ide-harness/codex-extension.
--trust marks the project and its hooks trusted in ~/.codex/config.toml, as the
extension's "Trust all and continue" does. Without it, untrusted hooks are
reported as such and do not run.
"""
import argparse
import json
import os
import platform
import queue
import subprocess
import sys
import threading
import time
import urllib.request
import zipfile
from pathlib import Path

CACHE = Path.home() / '.cache' / 'oko-ide-harness' / 'codex-extension'
GALLERY = 'https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery'
TURN_SECONDS = 240


def target_platform():
    machine = platform.machine().lower()
    arch = 'arm64' if machine in ('arm64', 'aarch64') else 'x64'
    return {'darwin': f'darwin-{arch}', 'linux': f'linux-{arch}'}[sys.platform]


def bin_dir():
    machine = 'aarch64' if platform.machine().lower() in ('arm64', 'aarch64') else 'x86_64'
    return f"{'macos' if sys.platform == 'darwin' else 'linux'}-{machine}"


def installed_codex():
    root = Path.home() / '.vscode' / 'extensions'
    found = sorted(root.glob('openai.chatgpt-*'), key=lambda p: [int(x) for x in p.name.split('-')[1].split('.') if x.isdigit()])
    for extension in reversed(found):
        binary = extension / 'bin' / bin_dir() / 'codex'
        if binary.exists():
            return binary, json.loads((extension / 'package.json').read_text())['version'], 'installed'
    return None


def downloaded_codex():
    """The latest Marketplace build for this platform, unpacked once per version."""
    body = json.dumps({'filters': [{'criteria': [{'filterType': 7, 'value': 'openai.chatgpt'}]}], 'flags': 914}).encode()
    request = urllib.request.Request(GALLERY, body, {'Content-Type': 'application/json', 'Accept': 'application/json;api-version=7.2-preview.1'})
    with urllib.request.urlopen(request, timeout=60) as response:
        versions = json.load(response)['results'][0]['extensions'][0]['versions']
    wanted = target_platform()
    version = next(v for v in versions if v.get('targetPlatform') == wanted)
    folder = CACHE / f"{version['version']}-{wanted}"
    binary = folder / 'extension' / 'bin' / bin_dir() / 'codex'
    if not binary.exists():
        url = next(f['source'] for f in version['files'] if f['assetType'].endswith('VSIXPackage'))
        folder.mkdir(parents=True, exist_ok=True)
        archive = folder / 'extension.vsix'
        urllib.request.urlretrieve(url, archive)
        with zipfile.ZipFile(archive) as vsix:
            vsix.extractall(folder)
        archive.unlink()
        for path in (folder / 'extension' / 'bin').rglob('*'):
            if path.is_file() and path.parent.name in (bin_dir(), 'codex-path'):
                path.chmod(0o755)
    return binary, version['version'], 'marketplace'


class AppServer:
    def __init__(self, binary):
        env = {**os.environ, 'RUST_LOG': 'warn', 'CODEX_INTERNAL_ORIGINATOR_OVERRIDE': 'codex_vscode'}
        self.proc = subprocess.Popen(
            [str(binary), '-c', 'features.code_mode_host=true', 'app-server', '--analytics-default-enabled'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True, bufsize=1)
        self.messages = queue.Queue()
        self.next_id = 0
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self.proc.stderr.read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            line = line.strip()
            if line:
                try:
                    self.messages.put(json.loads(line))
                except json.JSONDecodeError:
                    pass
        self.messages.put(None)

    def send(self, message):
        self.proc.stdin.write(json.dumps(message) + '\n')
        self.proc.stdin.flush()

    def request(self, method, params, on_message=None, timeout=60):
        self.next_id += 1
        ident = self.next_id
        self.send({'id': ident, 'method': method, 'params': params})
        deadline = time.time() + timeout
        while True:
            message = self.messages.get(timeout=max(0.1, deadline - time.time()))
            if message is None:
                raise RuntimeError(f'app-server exited during {method}')
            if message.get('id') == ident and 'method' not in message:
                if 'error' in message:
                    raise RuntimeError(f"{method}: {message['error'].get('message')}")
                return message['result']
            if on_message:
                on_message(message)

    def close(self):
        self.proc.stdin.close()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def trust(server, cwd, report):
    """What the extension's trust prompts write: the project, then each new or changed hook."""
    write = lambda edits: server.request('config/batchWrite', {'edits': edits, 'filePath': None, 'expectedVersion': None, 'reloadUserConfig': True})
    write([{'keyPath': f'projects.{json.dumps(cwd)}.trust_level', 'value': 'trusted', 'mergeStrategy': 'upsert'}])
    # Project hooks are listed only once the project is trusted.
    edits = [{'keyPath': f"hooks.state.{json.dumps(hook['key'])}.trusted_hash", 'value': hook['currentHash'], 'mergeStrategy': 'upsert'}
             for entry in server.request('hooks/list', {'cwds': [cwd]}).get('data', [])
             for hook in entry.get('hooks', []) if hook.get('trustStatus') in ('untrusted', 'modified')]
    if edits:
        write(edits)
    report['trusted'] = len(edits)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--cwd', required=True)
    parser.add_argument('--prompt', required=True)
    parser.add_argument('--codex', help='codex binary to run as the app-server')
    parser.add_argument('--trust', action='store_true')
    args = parser.parse_args()
    cwd = str(Path(args.cwd).resolve())
    if args.codex:
        binary, version, source = Path(args.codex), None, 'given'
    else:
        binary, version, source = installed_codex() or downloaded_codex()
    report = {'client': 'codex-vscode', 'extensionVersion': version, 'binarySource': source,
              'hooks': [], 'toolItems': [], 'answer': '', 'error': None}
    server = AppServer(binary)
    try:
        init = server.request('initialize', {
            'clientInfo': {'name': 'VS Code', 'title': 'Codex Extension', 'version': version or '0'},
            'capabilities': {'experimentalApi': True}})
        server.send({'method': 'initialized'})
        report['userAgent'] = init.get('userAgent')
        if args.trust:
            trust(server, cwd, report)
        report['hookTrust'] = [{'event': hook.get('eventName'), 'handler': hook.get('handlerType'), 'trust': hook.get('trustStatus')}
                               for entry in server.request('hooks/list', {'cwds': [cwd]}).get('data', []) for hook in entry.get('hooks', [])]
        thread = server.request('thread/start', {'cwd': cwd, 'approvalPolicy': 'never', 'sandbox': 'read-only', 'ephemeral': True})
        thread_id = thread['thread']['id']
        done = threading.Event()

        def observe(message):
            method, params = message.get('method'), message.get('params', {})
            if 'id' in message and method:
                # A server request, such as an approval: refuse it, as nothing should need one.
                server.send({'id': message['id'], 'result': {'decision': 'decline'}})
            elif method == 'hook/completed':
                run = params.get('run', {})
                report['hooks'].append({'event': run.get('eventName'), 'handler': run.get('handlerType'), 'status': run.get('status'),
                                        'contextBytes': sum(len(e.get('text', '')) for e in run.get('entries', []) if e.get('kind') == 'context'),
                                        'entries': [e.get('kind') for e in run.get('entries', [])]})
            elif method == 'item/completed':
                item = params.get('item', {})
                if item.get('type') == 'agentMessage':
                    report['answer'] += item.get('text', '')
                elif item.get('type') in ('commandExecution', 'mcpToolCall', 'fileChange', 'webSearch'):
                    report['toolItems'].append(item.get('type') if item.get('type') != 'mcpToolCall' else f"mcp:{item.get('tool')}")
            elif method == 'turn/completed':
                report['turnStatus'] = params.get('turn', {}).get('status')
                done.set()
            elif method == 'error':
                report['error'] = params.get('error', {}).get('message')

        server.request('turn/start', {'threadId': thread_id, 'input': [{'type': 'text', 'text': args.prompt, 'text_elements': []}]}, observe)
        deadline = time.time() + TURN_SECONDS
        while not done.is_set():
            message = server.messages.get(timeout=max(0.1, deadline - time.time()))
            if message is None:
                raise RuntimeError('app-server exited during the turn')
            observe(message)
    except Exception as error:  # noqa: BLE001  reported, not raised: the caller compares reports
        report['error'] = report['error'] or str(error)
    finally:
        server.close()
    print(json.dumps(report))


if __name__ == '__main__':
    main()
