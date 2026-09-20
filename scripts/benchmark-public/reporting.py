"""Reports preserve failed attempts and unknown measurements."""
import statistics


def render(data, clients, conditions):
    runs=data['runs']
    def median(rows,key):
        values=[r.get('measurements',{}).get(key) for r in rows]
        return round(statistics.median(values),3) if values and all(v is not None for v in values) else 'unavailable'
    def summed(rows,section,key):
        values=[(r.get('measurements',{}).get(section) or {}).get(key) for r in rows]
        return sum(values) if values and all(v is not None for v in values) else 'unavailable'
    def passed(rows):return sum(bool(r.get('grade',{}).get('passed')) and not r.get('error') for r in rows)
    lines=['# Public repository benchmark','',f"Sessions: {len(runs)}/{len(data['plan'])}. Complete: {data['complete']}.",
           f"Suite: {data.get('suite','legacy')}; repeats: {data.get('repeats',1)}; cache policy: {data.get('cachePolicy','legacy')}.",'',
           '| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |',
           '|---|---|---:|---:|---:|']
    for c in clients:
        for condition in conditions:
            rows=[r for r in runs if r['client']==c and r['condition']==condition]
            if rows:lines.append(f"| {c} | {condition} | {passed(rows)}/{len(rows)} | {statistics.median(r['seconds'] for r in rows):.2f} | {summed(rows,'agentTokens','total')} |")
    lines+=['','All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.',
            'Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.',
            'Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.',
            'Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.','',
            '## Components','',
            '| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |',
            '|---|---|---:|---:|---:|---:|---|---:|---:|---:|']
    for c in clients:
        for condition in conditions:
            rows=[r for r in runs if r['client']==c and r['condition']==condition]
            if not rows:continue
            tokens=[summed(rows,'agentTokens',k) for k in ('uncachedInput','cacheRead','cacheWrite','output')]
            jev=[summed(rows,'jev',k) for k in ('inputTokens','outputTokens')]
            lines.append(f"| {c} | {condition} | {' | '.join(map(str,tokens))} | {jev[0]} / {jev[1]} | {median(rows,'toolCalls')} | {median(rows,'modelRounds')} | {median(rows,'okoSearchSeconds')} |")
    lines+=['','Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.','',
            '## Per-task medians','',f"| Repository | Task | Client | {' | '.join(conditions)} | Passed/attempted |",'|---|---|---|'+'---:|'*len(conditions)+'---:|']
    for name,task,client in dict.fromkeys((r['repository'],r['id'],r['client']) for r in runs):
        rows=[r for r in runs if (r['repository'],r['id'],r['client'])==(name,task,client)]
        groups=[[r['seconds'] for r in rows if r['condition']==c] for c in conditions]
        times=[f'{statistics.median(g):.2f}s' if g else 'pending' for g in groups]
        lines.append(f"| {name} | {task} | {client} | {' | '.join(times)} | {passed(rows)}/{len(rows)} |")
    if data.get('suite')=='smoke':
        lines+=['','Smoke suite: a few tasks chosen because they separated builds before. It shows direction and catches regressions; it supports no speed or quality claim. Native search is not rerun; take it from a branch-suite run.']
    if data.get('suite') in ('branch','smoke'):
        lines+=['','## Paired current-build changes','',
                'Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.','',
                '| Client | Baseline | Pairs | Median seconds change | Median percent change |',
                '|---|---|---:|---:|---:|']
        for c in clients:
            current={(r['repository'],r['id'],r.get('repetition',1)):r for r in runs if r['client']==c and r['condition']=='current'}
            for baseline in ('native','previous'):
                pairs=[(r,current[(r['repository'],r['id'],r.get('repetition',1))]) for r in runs
                       if r['client']==c and r['condition']==baseline and (r['repository'],r['id'],r.get('repetition',1)) in current and r['seconds']>0]
                if not pairs:continue
                seconds=statistics.median(b['seconds']-a['seconds'] for a,b in pairs)
                percent=statistics.median(100*(b['seconds']/a['seconds']-1) for a,b in pairs)
                lines.append(f'| {c} | {baseline} | {len(pairs)} | {seconds:+.2f} | {percent:+.1f}% |')
    return '\n'.join(lines)+'\n'
