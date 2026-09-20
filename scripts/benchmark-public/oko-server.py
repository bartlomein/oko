#!/usr/bin/env python3
"""Launch the binary frozen for this exact public benchmark session."""
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent

def main():
    results = (ROOT.parents[1]/'benchmarks/results').resolve()
    workspace = Path(sys.argv[1]).resolve()
    parts = workspace.relative_to(results).parts
    if len(parts)!=6 or parts[0] not in ('public','public-branch','public-smoke') or parts[1] not in ('astro','httpx','ripgrep') or parts[-1]!='workspace':
        raise ValueError('Unknown benchmark workspace')
    trial=workspace.parent
    if Path(sys.argv[2]).resolve()!=trial/'cache':
        raise ValueError('Cache must belong to this session')
    spec=importlib.util.spec_from_file_location('shared_server',ROOT.parent/'benchmark-twenty/oko-server.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    module.main(root=ROOT,state_name=str(trial.relative_to(results)))

if __name__=='__main__':main()
