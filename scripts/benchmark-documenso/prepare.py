#!/usr/bin/env python3
"""Freeze the clean Documenso checkout and local executables; no model calls."""
from shared import ROOT, load_engine

engine = load_engine('prepare')
engine.__doc__ = __doc__
engine.ROOT = ROOT
engine.STATE = ROOT.parents[1] / 'benchmarks/results/documenso'

if __name__ == '__main__':
    engine.main(project_name='Documenso', repository_name='documenso')
