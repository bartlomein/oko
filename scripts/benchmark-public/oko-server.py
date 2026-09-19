#!/usr/bin/env python3
"""Use the existing credential-isolating launcher with a per-repository snapshot."""
import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
state = ROOT.parents[1] / 'benchmarks/results/public'
name = Path(sys.argv[1]).resolve().relative_to(state.resolve()).parts[0]
if name not in ('astro', 'httpx', 'ripgrep'):
    raise ValueError('Unknown benchmark repository')
spec = importlib.util.spec_from_file_location('shared_server', ROOT.parent / 'benchmark-twenty/oko-server.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.main(root=ROOT, state_name='public/' + name)
