"""Load the existing benchmark engine without depending on the caller's cwd."""
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def load_engine(name):
    path = ROOT.parent / 'benchmark-twenty' / f'{name}.py'
    spec = importlib.util.spec_from_file_location(f'documenso_{name}', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
