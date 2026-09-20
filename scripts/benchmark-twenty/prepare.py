#!/usr/bin/env python3
"""Freeze the clean Twenty checkout and local executables; no model calls."""
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PROJECT = ROOT.parents[1]
STATE = PROJECT / 'benchmarks/results/twenty'


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def oko_metadata(binary):
    version = subprocess.check_output([str(binary), '--version'], text=True).strip()
    try:
        repository = binary.parent.parent.parent
        commit = subprocess.check_output(
            ['git', '-c', 'core.hooksPath=/dev/null', 'rev-parse', 'HEAD'],
            cwd=repository,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        commit = None
    return commit, version


def main(project_name='Twenty', repository_name='twenty'):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repository', nargs='?', default=str(Path.home() / 'dev' / repository_name))
    parser.add_argument('--oko', default=str(PROJECT / 'target/release/oko'))
    parser.add_argument('--codex-model', default='gpt-6-astra')
    parser.add_argument('--opencode-model', default='openai/gpt-6-astra')
    parser.add_argument('--claude-model', default='claude-opus-5[1m]')
    parser.add_argument('--effort', choices=('low', 'medium', 'high'), default='medium',
                        help='Reasoning effort requested from all three clients')
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    repo = Path(args.repository).expanduser().resolve()
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=repo, text=True).strip()
    fixture = json.loads((ROOT / 'tasks.json').read_text())
    if git('rev-parse', 'HEAD') != fixture['commit']:
        raise RuntimeError('Checkout must match the commit in tasks.json; review and refreeze questions before using another commit')
    if git('status', '--porcelain'):
        raise RuntimeError(f'{project_name} checkout must be clean; no reset or modifications were made')
    if args.timeout < 1:
        raise ValueError('Timeout must be positive')
    for task in fixture['tasks']:
        targets = task['expected'] if task['kind'] == 'search' else [task]
        for target in targets:
            if digest(repo / target['path']) != target['sha256']:
                raise RuntimeError('Stale fixture: ' + target['path'])
        if task['kind'] == 'edit' and (repo / task['path']).read_text().count(task['old']) != 1:
            raise RuntimeError('Edit must have exactly one source match: ' + task['id'])
    clients = {name: shutil.which(name) for name in ('codex', 'opencode', 'claude')}
    if not all(clients.values()):
        raise RuntimeError('Install codex, opencode and claude on PATH before preparing')
    rg = shutil.which('rg')
    if not rg:
        raise RuntimeError('ripgrep must be on PATH')
    oko = Path(args.oko).expanduser().resolve()
    oko_commit, oko_version = oko_metadata(oko)
    versions = {name: subprocess.check_output([exe, '--version'], text=True).strip() for name, exe in clients.items()}
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    archive = STATE / 'baseline.tar'
    subprocess.run(['git', 'archive', '--format=tar', '--output', str(archive), fixture['commit']], cwd=repo, check=True)
    settings = dict(project=project_name, repository=str(repo), commit=fixture['commit'], targetVersion=None,
                    timeoutSeconds=args.timeout,
                    models={name: getattr(args, name + '_model') for name in clients}, effort=args.effort,
                    clients=clients, versions=versions, oko=str(oko), rg=rg, jevModel='jev-1.13.0',
                    okoCommit=oko_commit, okoVersion=oko_version,
                    archiveSha256=digest(archive), okoSha256=digest(oko), tasksSha256=digest(ROOT / 'tasks.json'))
    (STATE / 'settings.json').write_text(json.dumps(settings, indent=2) + '\n')
    print(f'Prepared {len(fixture["tasks"])} tasks at {fixture["commit"]}; snapshot {archive.stat().st_size // 1024 // 1024} MiB')
    print(f'Settings: {STATE / "settings.json"}')


if __name__ == '__main__':
    main()
