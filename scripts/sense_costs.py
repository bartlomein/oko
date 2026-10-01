#!/usr/bin/env python3
"""Token and cost breakdown of a Sense-harness run, per column.

  sense_costs.py LABEL=RESULTS_DIR [LABEL=RESULTS_DIR ...] [--jev-usd-per-million 0.042] [--json OUT]

For each session (RESULTS_DIR/<tool>/<repo>/run-N/) it reads:
- transcript.json (Claude Code stream-json): the model's tokens from the final
  `result` event, or summed from the assistant messages when the session was
  killed before writing one (marked estimated), and the model cost Claude Code
  reports;
- oko-metrics.jsonl (Oko's OKO_METRICS_FILE, when the image sets it): every Jev
  request with its input and output tokens; the prompt hook's (prefetch) apart.

The harness's own report gives only the model's dollar cost; Jev is not in it.
Jev cost uses the rate given (default: TypeSafe's list price as of 2026-09,
about $42 per billion tokens). Sense makes no paid calls of its own.
"""
import argparse
import glob
import json
import os
import statistics
import sys


def model_usage(transcript):
    """(usage dict, cost or None, estimated) for one session."""
    lines = []
    for line in open(transcript):
        try:
            lines.append(json.loads(line))
        except ValueError:
            pass
    result = next((e for e in lines if e.get('type') == 'result'), None)
    keys = ('input_tokens', 'cache_creation_input_tokens', 'cache_read_input_tokens', 'output_tokens')
    if result and isinstance(result.get('usage'), dict):
        usage = {k: result['usage'].get(k) or 0 for k in keys}
        return usage, result.get('total_cost_usd'), False
    # Killed before the result event: each assistant message carries its usage;
    # stream-json repeats a message once per content block, so keep the last.
    by_id = {}
    for e in lines:
        message = e.get('message') or {}
        if e.get('type') == 'assistant' and message.get('id') and isinstance(message.get('usage'), dict):
            by_id[message['id']] = message['usage']
    usage = {k: sum(u.get(k) or 0 for u in by_id.values()) for k in keys}
    return usage, None, True


def jev_usage(metrics):
    out = {'agentRequests': 0, 'agentInput': 0, 'agentOutput': 0,
           'hookRequests': 0, 'hookInput': 0, 'hookOutput': 0, 'recorded': os.path.exists(metrics)}
    if not out['recorded']:
        return out
    for line in open(metrics):
        try:
            value = json.loads(line)
        except ValueError:
            continue
        calls = (value.get('retrieval') or {}).get('jevCalls') or []
        side = 'hook' if 'prefetch' in value else 'agent'
        out[side + 'Requests'] += len(calls)
        out[side + 'Input'] += sum((c.get('usage') or {}).get('inputTokens') or 0 for c in calls)
        out[side + 'Output'] += sum((c.get('usage') or {}).get('outputTokens') or 0 for c in calls)
    return out


def harness_cost(run):
    """The harness's cost for a session (scored.json), which prices a killed
    session from its tokens; None before scoring."""
    path = os.path.join(run, 'scored.json')
    if not os.path.exists(path):
        return None
    def find(value):
        if isinstance(value, dict):
            if isinstance(value.get('cost_usd'), (int, float)):
                return value['cost_usd']
            for child in value.values():
                found = find(child)
                if found is not None:
                    return found
        return None
    return find(json.load(open(path)))


def column(results_dir):
    sessions = []
    for transcript in sorted(glob.glob(os.path.join(results_dir, '*', '*', 'run-*', 'transcript.json'))):
        run = os.path.dirname(transcript)
        usage, cost, estimated = model_usage(transcript)
        if cost is None:
            cost = harness_cost(run)
        sessions.append({'run': os.path.relpath(run, results_dir), 'usage': usage, 'cost': cost,
                         'estimated': estimated, 'jev': jev_usage(os.path.join(run, 'oko-metrics.jsonl'))})
    return sessions


def summarize(sessions, jev_rate):
    def total(key):
        return sum(s['usage'][key] for s in sessions)
    uncached, write, read, output = (total(k) for k in (
        'input_tokens', 'cache_creation_input_tokens', 'cache_read_input_tokens', 'output_tokens'))
    jev = {k: sum(s['jev'][k] for s in sessions) for k in
           ('agentRequests', 'agentInput', 'agentOutput', 'hookRequests', 'hookInput', 'hookOutput')}
    recorded = sum(s['jev']['recorded'] for s in sessions)
    jev_in = jev['agentInput'] + jev['hookInput']
    jev_out = jev['agentOutput'] + jev['hookOutput']
    model_cost = sum(s['cost'] or 0 for s in sessions)
    jev_cost = (jev_in + jev_out) / 1e6 * jev_rate
    return {
        'sessions': len(sessions), 'estimatedSessions': sum(s['estimated'] for s in sessions),
        'model': {'uncachedInput': uncached, 'cacheWrite': write, 'cacheRead': read,
                  'input': uncached + write + read, 'output': output, 'total': uncached + write + read + output,
                  'costUsd': round(model_cost, 2)},
        'jev': {'recordedSessions': recorded, **jev, 'requests': jev['agentRequests'] + jev['hookRequests'],
                'input': jev_in, 'output': jev_out, 'total': jev_in + jev_out,
                'costUsd': round(jev_cost, 2) if recorded else None},
        'totalCostUsd': round(model_cost + (jev_cost if recorded else 0), 2),
    }


def render(columns):
    def n(value):
        return f'{value:,}' if isinstance(value, int) else ('–' if value is None else str(value))
    rows = [
        ('Sessions (estimated: killed before a result)', lambda c: f"{c['sessions']} ({c['estimatedSessions']})"),
        ('Model uncached input', lambda c: n(c['model']['uncachedInput'])),
        ('Model cache writes', lambda c: n(c['model']['cacheWrite'])),
        ('Model cache reads', lambda c: n(c['model']['cacheRead'])),
        ('**Model input**', lambda c: n(c['model']['input'])),
        ('**Model output**', lambda c: n(c['model']['output'])),
        ('**Model total**', lambda c: n(c['model']['total'])),
        ('Model cost', lambda c: f"${c['model']['costUsd']:.2f}"),
        ('Jev recorded in sessions', lambda c: f"{c['jev']['recordedSessions']}/{c['sessions']}"),
        ('Jev requests (agent + prompt hook)', lambda c: f"{c['jev']['requests']} ({c['jev']['agentRequests']} + {c['jev']['hookRequests']})"),
        ('**Jev input**', lambda c: n(c['jev']['input'])),
        ('**Jev output**', lambda c: n(c['jev']['output'])),
        ('**Jev total**', lambda c: n(c['jev']['total'])),
        ('Jev cost', lambda c: '–' if c['jev']['costUsd'] is None else f"${c['jev']['costUsd']:.2f}"),
        ('**Total cost**', lambda c: f"${c['totalCostUsd']:.2f}"),
    ]
    labels = list(columns)
    lines = ['| | ' + ' | '.join(labels) + ' |', '|---|' + '---:|' * len(labels)]
    for name, cell in rows:
        lines.append(f'| {name} | ' + ' | '.join(cell(columns[label]) for label in labels) + ' |')
    return '\n'.join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('columns', nargs='+', metavar='LABEL=RESULTS_DIR')
    parser.add_argument('--jev-usd-per-million', type=float, default=0.042)
    parser.add_argument('--json', help='Also write the per-session and summary numbers here')
    args = parser.parse_args(argv)
    sessions, columns = {}, {}
    for item in args.columns:
        label, _, path = item.partition('=')
        sessions[label] = column(path)
        columns[label] = summarize(sessions[label], args.jev_usd_per_million)
    print(render(columns))
    print(f'\nJev cost at ${args.jev_usd_per_million} per million tokens. Model cost is what Claude Code reports;'
          ' for a session killed before reporting it, the harness\'s estimate from its tokens.')
    if args.json:
        json.dump({'columns': columns, 'sessions': sessions, 'jevUsdPerMillion': args.jev_usd_per_million},
                  open(args.json, 'w'), indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
