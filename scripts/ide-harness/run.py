#!/usr/bin/env python3
"""Check that Oko's prompt hook reaches the model inside the VS Code extensions.

Builds two small projects under ~/.cache/oko-ide-harness, sets Oko up in both
with the given build (one with the prompt hook, one with --no-prefetch as the
control), then asks the same question through the program behind each
extension with read and search tools out of reach:

  - Claude Code: the extension's bundled binary through the Agent SDK, with the
    extension's options (claude_vscode.mjs).
  - Codex: the extension's bundled `codex app-server`, spoken to as the
    extension does (codex_vscode.py).

A pass means the hooked project's answer names the right file and the
control's says NO CONTEXT, so the check can tell the two apart.

  run.py [--oko target/release/oko] [--clients claude,codex] [--out results.json]

Setup copies Oko into a harness-owned install directory, so the installed Oko
and its projects.json are untouched. Two things do land in your own config:
`claude mcp add-json --scope local` entries for the two harness projects in
~/.claude.json, and Codex trust for them in ~/.codex/config.toml (what the
extension's "Trust all and continue" writes). Model calls use your Claude and
ChatGPT logins and Jev calls your TypeSafe key: a run costs a few cents.
"""
import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROJECT = HERE.parent.parent
HOME = Path.home() / '.cache' / 'oko-ide-harness'
QUESTION = ('Answer without using any tools: which file and line number define the function that '
            'dispatches parcels to a depot? If nothing in your context tells you, reply exactly NO CONTEXT.')
EXPECTED = 'shipping.rs'
SOURCES = {
    'src/shipping.rs': '''pub struct Parcel {
    pub grams: u32,
    pub postcode: String,
}

/// Picks the depot for a parcel and hands it to the courier.
pub fn dispatch_parcel_to_depot(parcel: &Parcel) -> &'static str {
    if parcel.postcode.starts_with("ZX") {
        "north-depot"
    } else {
        "south-depot"
    }
}
''',
    'src/billing.rs': '''/// Charges a customer for a finished order.
pub fn charge_customer(cents: u64) -> u64 {
    cents + cents / 10
}
''',
}


def fixture(name, oko, clients, prefetch):
    """A fresh copy of the sample project, set up with this build of Oko."""
    root = HOME / name
    if root.exists():
        shutil.rmtree(root)
    for relative, text in SOURCES.items():
        (root / relative).parent.mkdir(parents=True, exist_ok=True)
        (root / relative).write_text(text)
    subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
    subprocess.run(['git', 'add', '-A'], cwd=root, check=True)
    subprocess.run(['git', '-c', 'user.name=harness', '-c', 'user.email=harness@localhost', 'commit', '-qm', 'sample'], cwd=root, check=True)
    command = [str(oko), 'setup', '--client', ','.join(clients), '--install-dir', str(HOME / 'install'), '--quiet']
    if not prefetch:
        command.append('--no-prefetch')
    subprocess.run(command, cwd=root, check=True)
    return root


def probe(client, root):
    if client == 'claude':
        command = ['node', str(HERE / 'claude_vscode.mjs'), '--cwd', str(root), '--prompt', QUESTION]
    else:
        command = [sys.executable, '-I', str(HERE / 'codex_vscode.py'), '--trust', '--cwd', str(root), '--prompt', QUESTION]
    done = subprocess.run(command, capture_output=True, text=True, timeout=600)
    if done.returncode != 0:
        return {'error': (done.stderr or done.stdout).strip()[-2000:]}
    return json.loads(done.stdout.strip().splitlines()[-1])


def verdict(hooked, control):
    """Whether the hook's context reached the model, judged against the control."""
    answered = EXPECTED in hooked.get('answer', '') and 'NO CONTEXT' not in hooked.get('answer', '')
    control_blind = 'NO CONTEXT' in control.get('answer', '')
    if hooked.get('error') or control.get('error'):
        return 'ERROR'
    if answered and control_blind:
        return 'PASS'
    if not control_blind:
        return 'INCONCLUSIVE'  # the model found the answer without the hook, so the test cannot tell
    return 'FAIL'


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--oko', default=str(PROJECT / 'target' / 'release' / 'oko'))
    parser.add_argument('--clients', default='claude,codex')
    parser.add_argument('--out', help='write the full reports here as JSON')
    args = parser.parse_args()
    clients = args.clients.split(',')
    if 'claude' in clients and not (HERE / 'node_modules').exists():
        subprocess.run(['npm', 'ci', '--silent'], cwd=HERE, check=True)
    oko = Path(args.oko).resolve()
    version = subprocess.run([str(oko), '--version'], capture_output=True, text=True, check=True).stdout.strip()
    hooked = fixture('project', oko, clients, prefetch=True)
    control = fixture('control', oko, clients, prefetch=False)
    results = {'oko': version, 'clients': {}}
    for client in clients:
        reports = {'hooked': probe(client, hooked), 'control': probe(client, control)}
        reports['verdict'] = verdict(reports['hooked'], reports['control'])
        results['clients'][client] = reports
    if args.out:
        Path(args.out).write_text(json.dumps(results, indent=2) + '\n')
    print(f'{version}')
    for client, reports in results['clients'].items():
        hooked_report = reports['hooked']
        where = f"extension {hooked_report.get('extensionVersion')}"
        # Claude reports a hook's start and its response; Codex reports completions only.
        hooks = [h for h in hooked_report.get('hooks', []) if h.get('event') in ('UserPromptSubmit', 'userPromptSubmit') and h.get('subtype') != 'hook_started']
        print(f"  {client:7} {reports['verdict']:12} {where}; prompt hook runs: {len(hooks)}; "
              f"answer: {hooked_report.get('answer', '')[:70]!r}; control: {reports['control'].get('answer', '')[:20]!r}"
              + (f"; error: {hooked_report.get('error') or reports['control'].get('error')}" if reports['verdict'] == 'ERROR' else ''))
    sys.exit(0 if all(r['verdict'] == 'PASS' for r in results['clients'].values()) else 1)


if __name__ == '__main__':
    main()
