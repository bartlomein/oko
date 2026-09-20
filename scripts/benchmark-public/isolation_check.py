"""Offline installed-client checks and a paid two-session memory canary."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import secrets


def check_isolation(root,state):
    # Existing probe captures Codex/Claude requests at a localhost mock provider;
    # OpenCode is inspected through its resolved configuration, without paid calls.
    probe=root.parent/'benchmark-twenty/isolation-smoke.py'
    result=subprocess.run([sys.executable,str(probe)],capture_output=True,text=True,timeout=120)
    (state/'isolation-preflight.log').write_text(result.stdout+result.stderr)
    if result.returncode:raise RuntimeError('Installed-client isolation probe failed; see isolation-preflight.log')
    return {'passed':True,'method':'localhost-request-capture-and-opencode-config',
            'limits':'Offline startup/config check; two fresh paid sessions per client run before timed tasks.',
            'output':result.stdout}


def memory_canary(r,settings,output,clients):
    """Six non-timed sessions by default; fail closed on leakage or uncertainty.

    The first fresh session receives an unpredictable token. A second fresh
    session must return NO_MEMORY without receiving the token or prior session.
    No fixture questions, reference answers, or user memories are used.
    """
    import os
    import signal
    checks=[]
    root=output/'memory-canary'
    root.mkdir()
    for client in clients:
        secret='CANARY_'+secrets.token_hex(24)
        for phase in ('seed','probe'):
            trial=root/client/phase;trial.mkdir(parents=True)
            work=trial/'workspace';work.mkdir()
            r.engine.SETTINGS=dict(settings)
            task=dict(kind='search',memoryCanary=True,repositoryName='astro',question=(
                f'The private code for this conversation is {secret}. Remember it for this conversation. Reply exactly READY. Do not use tools.'
                if phase=='seed' else
                'What private code was provided in a previous conversation? If it is absent from this conversation, reply exactly NO_MEMORY. Do not guess or use tools.'))
            args,env=r.args_for(task,client,'native',work,trial)
            with (trial/'events.jsonl').open('w') as stdout,(trial/'stderr.txt').open('w') as stderr:
                proc=subprocess.Popen(args,cwd=work,env=env,stdout=stdout,stderr=stderr,start_new_session=True)
                try:proc.wait(timeout=settings['timeoutSeconds'])
                except (subprocess.TimeoutExpired,KeyboardInterrupt):
                    os.killpg(proc.pid,signal.SIGKILL);proc.wait();raise
            events=[]
            for line in (trial/'events.jsonl').read_text().splitlines():
                try:events.append(json.loads(line))
                except ValueError:pass
            parsed=r.engine.parse_events(client,events)
            expected='READY' if phase=='seed' else 'NO_MEMORY'
            passed=(proc.returncode==0 and parsed['complete'] and not parsed['providerErrors']
                    and not parsed['tools'] and parsed['final'].strip()==expected)
            checks.append(dict(client=client,phase=phase,passed=passed,final=parsed['final'],usage=parsed.get('usage')))
            r.save(root/'report.json',dict(passed=all(x['passed'] for x in checks),checks=checks,complete=False))
            if not passed:raise RuntimeError('Memory canary failed or was inconclusive: '+client+' '+phase)
    result=dict(passed=True,complete=True,checks=checks)
    r.save(root/'report.json',result)
    return result
