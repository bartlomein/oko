#!/usr/bin/env python3
"""Turn a Claude Code session log into a ContextBench trajectory.

ContextBench (github.com/EuniAI/ContextBench) scores what an agent looked at
while working on an issue: every file and line range it viewed, step by step,
plus the context it declares essential before it finishes. This reads the
`stream-json` events Claude Code writes (the runner saves them as
`events.jsonl`) and produces the benchmark's unified format:

    {"instance_id": ..., "traj_data": {"pred_steps": [...], "pred_files": [...],
     "pred_spans": {...}}, "model_patch": ...}

What counts as a view, one step per tool call in order:

- `Read`: the file, with `offset`/`limit` when given, else the line numbers
  in the returned text.
- `Grep`: matched files; with content output, each matched line.
- `Bash`: `cat FILE`, `sed -n 'A,Bp' FILE`, `head -n N FILE`, `grep -n`
  output, as the benchmark's own extractors read them.
- Oko `search`: the excerpts the packet shows, with their line ranges. The
  paths merely listed as other candidates are not views; the agent saw the
  names, not the code.

The declared context is a `<PATCH_CONTEXT>` block in the agent's final text:

    <PATCH_CONTEXT>
    File: src/foo.py
    Lines: 10-42, 80-95
    File: src/bar.py
    Lines: 1-20
    </PATCH_CONTEXT>

Without one, the union of everything viewed stands in, which the benchmark
also allows (the leaderboard's agents mostly declare).

    extract.py events.jsonl --workspace DIR --instance ID [--patch diff] > traj.json
    extract.py --selftest
"""
import argparse
import json
import re
import sys
from pathlib import Path

VIEW_TOOLS = ('Read', 'Grep', 'Bash', 'mcp__oko__search')


def relative(path, workspace):
    """A path inside the workspace, relative to it; None for anything else."""
    if not path:
        return None
    p = Path(path)
    if p.is_absolute():
        try:
            return p.resolve().relative_to(Path(workspace).resolve()).as_posix()
        except ValueError:
            return None
    text = p.as_posix()
    return None if text.startswith('..') else text.lstrip('./') or None


def numbered_range(text):
    """First and last line number of `N\\tcode` output such as Read's."""
    numbers = [int(m.group(1)) for m in re.finditer(r'(?m)^\s*(\d+)\t', text or '')]
    return (min(numbers), max(numbers)) if numbers else None


def result_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return '\n'.join(part.get('text', '') for part in content if isinstance(part, dict))
    return ''


def read_step(inp, result, workspace):
    path = relative(inp.get('file_path'), workspace)
    if not path:
        return {}
    if inp.get('offset') or inp.get('limit'):
        start = int(inp.get('offset') or 1)
        limit = int(inp.get('limit') or 2000)
        return {path: [(start, start + limit - 1)]}
    span = numbered_range(result_text(result))
    return {path: [span]} if span else {path: []}


def grep_step(inp, result, workspace):
    text = result_text(result)
    views = {}
    base = inp.get('path')

    def resolve(candidate):
        # Results are relative to the searched directory, unless they already
        # name the searched path (a file) or are absolute.
        if Path(candidate).is_absolute() or not base or candidate.startswith(base.rstrip('/')):
            return relative(candidate, workspace)
        return relative(str(Path(base) / candidate), workspace)

    for line in text.splitlines():
        m = re.match(r'^([^\s:][^:]*?):(\d+)[:-]', line)
        if m:
            path = resolve(m.group(1))
            if path:
                n = int(m.group(2))
                views.setdefault(path, []).append((n, n))
            continue
        candidate = line.strip()
        if candidate and not candidate.startswith('Found ') and ('/' in candidate or '.' in candidate) and ' ' not in candidate:
            path = resolve(candidate)
            if path:
                views.setdefault(path, [])
    return views


def bash_step(inp, result, workspace):
    command = inp.get('command') or ''
    text = result_text(result)
    views = {}
    for m in re.finditer(r"sed\s+-n\s+'?(\d+),(\d+)p'?\s+(\S+)", command):
        path = relative(m.group(3), workspace)
        if path:
            views.setdefault(path, []).append((int(m.group(1)), int(m.group(2))))
    for m in re.finditer(r'\bcat\s+(?:-n\s+)?(\S+)', command):
        path = relative(m.group(1), workspace)
        if path and path not in views:
            lines = text.count('\n') + 1 if text else 0
            views[path] = [(1, lines)] if lines else []
    for m in re.finditer(r'\bhead\s+-n?\s*(\d+)\s+(\S+)', command):
        path = relative(m.group(2), workspace)
        if path:
            views.setdefault(path, []).append((1, int(m.group(1))))
    if re.search(r'\bgrep\b.*\s-\w*n', command) or 'rg ' in command:
        for line in text.splitlines():
            m = re.match(r'^([^\s:]+):(\d+)[:-]', line)
            if m:
                path = relative(m.group(1), workspace)
                if path:
                    n = int(m.group(2))
                    views.setdefault(path, []).append((n, n))
    return views


def oko_step(inp, result, workspace):
    text = result_text(result)
    try:
        packet = json.loads(text)
    except ValueError:
        # The MCP text form: `path:start-end (…)` headings above each excerpt.
        views = {}
        for m in re.finditer(r'(?m)^([^\s:]+):(\d+)-(\d+) \(', text):
            path = relative(m.group(1), workspace)
            if path:
                views.setdefault(path, []).append((int(m.group(2)), int(m.group(3))))
        return views
    views = {}
    for kind in ('results', 'related'):
        for entry in packet.get(kind) or []:
            path = relative(entry.get('path'), workspace)
            if path and entry.get('startLine'):
                views.setdefault(path, []).append((int(entry['startLine']), int(entry['endLine'])))
    return views


def declared_context(text):
    m = re.search(r'<PATCH_CONTEXT>(.*?)</PATCH_CONTEXT>', text or '', re.S)
    if not m:
        return None
    spans, current = {}, None
    for line in m.group(1).splitlines():
        line = line.strip()
        if line.lower().startswith('file:'):
            current = line[5:].strip().strip('`')
            spans.setdefault(current, [])
        elif line.lower().startswith('lines:') and current:
            for a, b in re.findall(r'(\d+)\s*-\s*(\d+)', line):
                spans[current].append((int(a), int(b)))
            for single in re.findall(r'(?<![\d-])(\d+)(?![\d-])', line):
                if not any(int(single) == a or int(single) == b for a, b in spans[current]):
                    spans[current].append((int(single), int(single)))
    return spans


def extract(events_path, workspace, instance_id, patch=''):
    tool_uses, results, final_text = [], {}, ''
    for line in Path(events_path).read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        message = event.get('message') or {}
        content = message.get('content') if isinstance(message.get('content'), list) else []
        for part in content:
            if not isinstance(part, dict):
                continue
            if part.get('type') == 'tool_use':
                tool_uses.append(part)
            elif part.get('type') == 'tool_result':
                results[part.get('tool_use_id')] = part.get('content')
            elif part.get('type') == 'text' and event.get('type') == 'assistant':
                final_text = part.get('text') or final_text
        if event.get('type') == 'result' and isinstance(event.get('result'), str):
            final_text = event['result']
    steps = []
    for use in tool_uses:
        name, inp = use.get('name'), use.get('input') or {}
        result = results.get(use.get('id'))
        views = {}
        if name == 'Read':
            views = read_step(inp, result, workspace)
        elif name == 'Grep':
            views = grep_step(inp, result, workspace)
        elif name == 'Bash':
            views = bash_step(inp, result, workspace)
        elif name == 'mcp__oko__search':
            views = oko_step(inp, result, workspace)
        if views:
            steps.append({
                'files': sorted(views),
                'spans': {f: [{'start': a, 'end': b} for a, b in s] for f, s in views.items() if s},
                'symbols': {},
                'tool': name,
            })
    declared = declared_context(final_text)
    if declared is None:
        union = {}
        for step in steps:
            for f in step['files']:
                union.setdefault(f, [])
            for f, spans in step['spans'].items():
                union[f].extend((s['start'], s['end']) for s in spans)
        declared, declared_source = union, 'union-of-views'
    else:
        declared = {relative(f, workspace) or f: s for f, s in declared.items()}
        declared_source = 'declared'
    return {
        'instance_id': instance_id,
        'traj_data': {
            'pred_steps': steps,
            'pred_files': sorted(declared),
            'pred_spans': {f: [{'start': a, 'end': b} for a, b in s] for f, s in declared.items() if s},
            'declared_source': declared_source,
        },
        'model_patch': patch,
    }


def selftest():
    root = Path(__file__).resolve().parents[3] / 'benchmarks/results/documenso'
    cases = [
        ('results-ayaocol3/011-document-search-delay-claude-oko', 'mcp__oko__search'),
        ('results-ayaocol3/005-pdf-page-count-claude-native', 'Read'),
    ]
    ok = True
    for trial, expected_tool in cases:
        events = root / trial / 'events.jsonl'
        if not events.is_file():
            print(f'skip {trial}: no events.jsonl')
            continue
        out = extract(events, root / trial / 'workspace', trial)
        tools = [s['tool'] for s in out['traj_data']['pred_steps']]
        spans = out['traj_data']['pred_spans']
        good = expected_tool in tools and spans and all(s['start'] >= 1 and s['end'] >= s['start'] for v in spans.values() for s in v)
        ok &= bool(good)
        print(f'{"ok " if good else "BAD"} {trial}: steps {tools}, files {out["traj_data"]["pred_files"][:4]}, '
              f'spans {sum(len(v) for v in spans.values())}, {out["traj_data"]["declared_source"]}')
    declared = declared_context('done\n<PATCH_CONTEXT>\nFile: a/b.py\nLines: 3-9, 20\nFile: c.py\nLines: 1-2\n</PATCH_CONTEXT>')
    good = declared == {'a/b.py': [(3, 9), (20, 20)], 'c.py': [(1, 2)]}
    ok &= good
    print(f'{"ok " if good else "BAD"} declared block: {declared}')
    return ok


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('events', nargs='?')
    parser.add_argument('--workspace')
    parser.add_argument('--instance')
    parser.add_argument('--patch', help='File holding the final git diff')
    parser.add_argument('--selftest', action='store_true')
    args = parser.parse_args()
    if args.selftest:
        sys.exit(0 if selftest() else 1)
    if not (args.events and args.workspace and args.instance):
        parser.error('events, --workspace and --instance are required')
    patch = Path(args.patch).read_text() if args.patch else ''
    print(json.dumps(extract(args.events, args.workspace, args.instance, patch)))


if __name__ == '__main__':
    main()
