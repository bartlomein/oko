#!/usr/bin/env python3
"""Launch only Oko with its credential. Never include secrets in configs or logs."""
import json, os, re, sys
from pathlib import Path

def main(root=None, state_name="twenty"):
    root = Path(root or Path(__file__).resolve().parent)
    state = root.parents[1] / 'benchmarks/results' / state_name
    settings = json.loads((state / 'settings.json').read_text())
    workspace = Path(sys.argv[1]).resolve()
    cache = Path(sys.argv[2]).resolve()
    if not (workspace.is_relative_to(state.resolve()) and cache.is_relative_to(state.resolve())):
        raise RuntimeError('Oko paths must remain within benchmark artifacts')
    env = dict(os.environ)
    if not env.get('TYPESAFE_API_KEY'):
        path = root.parents[1] / '.env'
        if path.exists():
            for line in path.read_text().splitlines():
                match = re.match('^\\s*(?:export\\s+)?TYPESAFE_API_KEY\\s*=\\s*(.*)$', line)
                if not match:
                    continue
                value = match.group(1).strip()
                if value[:1] in ('"', "'", '`'):
                    end = value.find(value[0], 1)
                    if end < 1:
                        raise RuntimeError('Unsupported credential format')
                    value = value[1:end]
                else:
                    value = value.split('#', 1)[0].strip()
                env['TYPESAFE_API_KEY'] = value
    # Oko keeps serving metadata out of the agent-visible tool result; the runner
    # reads it from this per-trial file beside the cache directory.
    env.update(OKO_RIPGREP=settings['rg'], OKO_CACHE_DIR=str(cache), OKO_NO_CACHE='0', TYPESAFE_DEFAULT_MODEL=settings['jevModel'],
               OKO_METRICS_FILE=str(cache.parent / 'oko-metrics.jsonl'))
    os.execve(settings['oko'], [settings['oko'], 'mcp', '--root', str(workspace)], env)


if __name__ == "__main__":
    main()
