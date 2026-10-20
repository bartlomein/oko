#!/usr/bin/env python3
"""Build Oko's MCP Bundle (.mcpb) for macOS from the two macOS release archives.

An MCP Bundle is a zip with a manifest.json that apps such as Claude Desktop
install with a settings form; the official MCP Registry lists Oko through it.
The bundle holds one universal binary (Apple silicon and Intel) for Oko and for
its ripgrep, made with `lipo`, so it needs macOS to build.

  python3 scripts/mcpb.py --arm oko-vX-aarch64-apple-darwin.tar.gz \\
      --intel oko-vX-x86_64-apple-darwin.tar.gz --output target/release-bundles

Writes oko-vX-macos.mcpb and its .sha256 sidecar.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

DESCRIPTION = 'Code search for coding agents: ranked, exact excerpts with line numbers.'
LONG_DESCRIPTION = (
    'Oko answers questions about your code with the relevant excerpts, current file text with '
    'exact line numbers, ranked by a relevance model (Jev, from TypeSafe AI; a key is needed). '
    'This bundle adds Oko\'s search tool for one project folder. In Claude Code, Codex, Cursor, '
    'OpenCode and Pi, `brew install bartlomein/tap/oko` and `oko setup` in a project also add '
    'Oko\'s answer to each prompt before the agent starts, where most of its savings come from.')
# Reproducible archives: one timestamp for every entry.
ZIP_TIME = (2020, 1, 1, 0, 0, 0)


def extract(archive, into):
    with tarfile.open(archive) as tar:
        tar.extractall(into, filter='data')
    (folder,) = [path for path in Path(into).iterdir() if path.is_dir()]
    return folder


def manifest(version):
    return {
        'manifest_version': '0.3',
        'name': 'oko',
        'display_name': 'Oko',
        'version': version,
        'description': DESCRIPTION,
        'long_description': LONG_DESCRIPTION,
        'author': {'name': 'Bartlomiej Tyrpien', 'url': 'https://github.com/bartlomein'},
        'homepage': 'https://github.com/bartlomein/oko',
        'documentation': 'https://github.com/bartlomein/oko#readme',
        'repository': {'type': 'git', 'url': 'https://github.com/bartlomein/oko'},
        'license': 'MIT',
        'keywords': ['code search', 'coding agents', 'retrieval', 'mcp'],
        'server': {
            'type': 'binary',
            'entry_point': 'server/oko',
            'mcp_config': {
                'command': '${__dirname}/server/oko',
                'args': ['mcp', '--root', '${user_config.project}'],
                'env': {
                    'TYPESAFE_API_KEY': '${user_config.typesafe_api_key}',
                    'OKO_RIPGREP': '${__dirname}/server/rg',
                },
            },
        },
        'tools': [{'name': 'search', 'description': 'Find code by describing its behavior or naming a '
                   'function, class or method; returns ranked excerpts with exact line numbers.'}],
        'user_config': {
            'project': {
                'type': 'directory',
                'title': 'Project folder',
                'description': 'Click Browse and pick the project Oko searches.',
                'required': True,
            },
            # Required: an empty override would disable a key saved with
            # `oko auth login`, so an optional field could not fall back to it.
            'typesafe_api_key': {
                'type': 'string',
                'title': 'TypeSafe API key',
                'description': 'For ranking with Jev; from your TypeSafe AI account.',
                'sensitive': True,
                'required': True,
            },
        },
        'compatibility': {'platforms': ['darwin']},
    }


def add(bundle, path, name):
    info = zipfile.ZipInfo(name, date_time=ZIP_TIME)
    info.compress_type = zipfile.ZIP_DEFLATED
    executable = path.stat().st_mode & 0o111
    info.external_attr = (0o100755 if executable else 0o100644) << 16
    bundle.writestr(info, path.read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--arm', type=Path, required=True)
    parser.add_argument('--intel', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as scratch:
        scratch = Path(scratch)
        arm = extract(args.arm, scratch / 'arm')
        intel = extract(args.intel, scratch / 'intel')
        builds = [json.loads((folder / 'BUILD.json').read_text()) for folder in (arm, intel)]
        if builds[0]['version'] != builds[1]['version'] or builds[0]['sourceCommit'] != builds[1]['sourceCommit']:
            raise SystemExit('The two archives are from different releases.')
        version = builds[0]['version']
        stage = scratch / 'bundle'
        (stage / 'server').mkdir(parents=True)
        for name in ('oko', 'rg'):
            subprocess.run(['lipo', '-create', arm / name, intel / name, '-output', stage / 'server' / name],
                           check=True)
            (stage / 'server' / name).chmod(0o755)
        (stage / 'manifest.json').write_text(json.dumps(manifest(version), indent=2) + '\n')
        for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'README.md'):
            shutil.copy(arm / name, stage / name)
        shutil.copytree(arm / 'licenses', stage / 'licenses')
        args.output.mkdir(parents=True, exist_ok=True)
        target = args.output / f'oko-v{version}-macos.mcpb'
        with zipfile.ZipFile(target, 'w') as bundle:
            for path in sorted(p for p in stage.rglob('*') if p.is_file()):
                add(bundle, path, path.relative_to(stage).as_posix())
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    (target.parent / (target.name + '.sha256')).write_text(f'{digest}  {target.name}\n')
    print(f'{target} {digest}')


if __name__ == '__main__':
    main()
