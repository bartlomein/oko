#!/usr/bin/env python3
"""Documenso: 3 read-only searches + 2 edits, 30 sessions. Plan-only unless --execute."""
import json

from shared import ROOT, load_engine

engine = load_engine('runner')
engine.__doc__ = __doc__
engine.ROOT = ROOT
engine.PROJECT_NAME = 'Documenso'
engine.STATE = ROOT.parents[1] / 'benchmarks/results/documenso'
engine.TASKS = json.loads((ROOT / 'tasks.json').read_text())['tasks']
engine.FAST_TASK_IDS = tuple(task['id'] for task in engine.TASKS)
engine.PILOT_TASK_IDS = ('pdf-page-count', 'document-search-delay')

if __name__ == '__main__':
    engine.main()
