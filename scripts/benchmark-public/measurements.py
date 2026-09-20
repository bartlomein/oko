"""Derived benchmark measurements; raw provider usage remains untouched."""
import json
from pathlib import Path


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and value >= 0


def token_breakdown(row):
    t = row.get('tokens') or {}
    client = row.get('client')
    try:
        if client == 'codex':
            values = [t['input'], t['cachedInput'], t['output']]
            if not all(number(v) for v in values) or values[1] > values[0]:
                return None
            uncached, read, write, output = values[0] - values[1], values[1], 0, values[2]
        elif client == 'claude':
            uncached, read, write, output = (t[k] for k in ('input', 'cacheRead', 'cacheWrite', 'output'))
        elif client == 'opencode':
            steps = t['steps']
            if not steps:
                return None
            values = [[s['input'], s['cache']['read'], s['cache']['write'], s['output'], s.get('reasoning', 0)] for s in steps]
            if not all(number(v) for step in values for v in step):
                return None
            uncached, read, write, output = (sum(s[i] for s in values) for i in range(4))
            output += sum(s[4] for s in values)
        else:
            return None
        if not all(number(v) for v in (uncached, read, write, output)):
            return None
        total = uncached + read + write + output
        return dict(total=total, totalSource='derived-from-components', providerTotal=t.get('total'),
                    uncachedInput=uncached, cacheRead=read, cacheWrite=write, output=output)
    except (KeyError, TypeError):
        return None


def measurements(row):
    def embedded(value):
        if isinstance(value,str):
            try:return embedded(json.loads(value))
            except (ValueError,RecursionError):return None
        if isinstance(value,dict):
            if isinstance(value.get('timings'),dict) and 'retrieval' in value:
                return value
            children=value.values()
        elif isinstance(value,list):children=value
        else:return None
        for child in children:
            found=embedded(child)
            if found is not None:return found
        return None
    metrics=[]
    for tool in row.get('tools',[]):
        if tool.get('okoMetrics'):
            metrics.extend(tool['okoMetrics'])
        else:
            # Previous builds expose the same metadata in structured MCP output.
            # Take one copy per tool, avoiding duplicated JSON/text representations.
            found=embedded(tool)
            if found is not None:metrics.append(found)
    calls = [call for m in metrics for call in (m.get('retrieval') or {}).get('jevCalls', [])]
    def summed(values):
        return sum(values) if all(number(v) for v in values) else None
    jev = {key: summed([(c.get('usage') or {}).get(key) for c in calls])
           for key in ('inputTokens', 'outputTokens', 'cacheReadTokens', 'cacheWriteTokens')}
    jev['calls'] = len(calls)
    jev['seconds'] = summed([c.get('durationNs') for c in calls])
    if jev['seconds'] is not None:
        jev['seconds'] /= 1e9
    if row.get('oko') and not metrics:
        jev = {key: None for key in jev}
    events = []
    path = Path(row.get('artifact', '')) / 'events.jsonl'
    if path.is_file():
        for line in path.read_text().splitlines():
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
    client = row.get('client')
    if client == 'opencode':
        rounds = len([e for e in events if e.get('type') == 'step_finish']) or None
    elif client == 'claude':
        ids = {e.get('message', {}).get('id') for e in events if e.get('type') == 'assistant'} - {None}
        rounds = len(ids) or None
    else:
        # Codex turn.completed is an entire agent turn, not a model request.
        rounds = None
    prep = [e['cache'].get('totalMs') for t in row.get('tools', []) for e in t.get('okoPrewarm', [])]
    search_ms = summed([m.get('timings', {}).get('totalMs') for m in metrics]) if metrics else None
    return dict(agentTokens=token_breakdown(row), jev=jev,
                toolCalls=row.get('toolCalls'), modelRounds=rounds,
                modelRoundsSource='unavailable' if rounds is None else 'client-events',
                agentSeconds=row.get('seconds'),
                okoSearchSeconds=search_ms / 1000 if search_ms is not None else (None if row.get('oko') else 0),
                startupPreparationSeconds=sum(prep) / 1000 if prep and all(number(v) for v in prep) else None,
                offlineWarmupSeconds=(row.get('warmup') or {}).get('seconds'))
