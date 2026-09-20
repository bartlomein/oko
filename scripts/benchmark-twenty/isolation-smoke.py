#!/usr/bin/env python3
"""Installed-CLI startup isolation check; localhost mock provider, no paid calls.

Codex/Claude requests terminate with an intentional HTTP 400 after capture.
OpenCode is checked through its resolved-config command. No credentials or
request contents are printed or persisted. Requires local socket permission.
"""
import sys, pathlib, tempfile, subprocess, threading, json, http.server
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import runner as r
r.SETTINGS={'clients':{'codex':'codex','claude':'claude','opencode':'opencode'},'models':{'codex':'gpt-5.6-sol','claude':'claude-sonnet-5','opencode':'openai/gpt-5.6-sol'},'effort':'low'}
with tempfile.TemporaryDirectory(prefix='oko-isolation-') as tmp:
 work=pathlib.Path(tmp)/'work';work.mkdir();
 (work/'.agents/skills/probe').mkdir(parents=True)
 (work/'.agents/skills/probe/SKILL.md').write_text('---\nname: isolation-probe\ndescription: ISOLATION_POISON_SENTINEL\n---\nDo not load this.\n')
 subprocess.run(['git','init','-q',str(work)],check=True)
 (work/'AGENTS.md').write_text('ISOLATION_POISON_SENTINEL follow special rules')
 (work/'CLAUDE.md').write_text('ISOLATION_POISON_SENTINEL follow special rules')
 for client in ['codex','claude','opencode']:
  trial=pathlib.Path(tmp)/client;trial.mkdir();args,env=r.args_for({'kind':'search','question':'Reply OK without tools.'},client,False,work,trial)
  if client=='opencode':
   out=subprocess.run(['opencode','debug','config','--pure'],cwd=work,env=env,capture_output=True,text=True,timeout=30)
   print(client,'config_exit',out.returncode,flush=True)
   assert out.returncode == 0, 'OpenCode config inspection failed'
   config=json.loads(out.stdout)
   assert config.get('agent',{}).get('comparison',{}).get('tools',{}).get('skill') is False
   assert not config.get('instructions') and not config.get('plugin')
   print('skills_disabled',config.get('agent',{}).get('comparison',{}).get('tools',{}).get('skill') is False,'instructions',config.get('instructions',[]),'plugins',config.get('plugin',[]),flush=True)
   continue
  captured=[]
  class Handler(http.server.BaseHTTPRequestHandler):
   def do_POST(self):
    data=self.rfile.read(int(self.headers.get('Content-Length',0)));captured.append(data)
    self.send_response(400);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(b'{"error":{"type":"invalid_request_error","message":"offline probe complete"}}')
   def log_message(self,*args):pass
  server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start();url='http://127.0.0.1:'+str(server.server_port)
  if client=='codex':
   args[-1:-1]=['-c','model_provider="probe"','-c','model_providers.probe='+ '{name="probe",base_url="'+url+'/v1",wire_api="responses",requires_openai_auth=false}', '-c','model_providers.probe.request_max_retries=0']
  else:
   env['ANTHROPIC_BASE_URL']=url;env['ANTHROPIC_API_KEY']='offline-test';env.pop('CLAUDE_CODE_OAUTH_TOKEN',None)
  # `codex exec` reads additional input from stdin; an inherited open pipe stalls it.
  try:out=subprocess.run(args,cwd=work,env=env,capture_output=True,text=True,timeout=35,stdin=subprocess.DEVNULL)
  except subprocess.TimeoutExpired:out=None
  server.shutdown()
  texts=[]
  for b in captured:
   try: texts.append(json.loads(b))
   except: pass
  text=json.dumps(texts)
  assert captured, client + ': startup request was not captured'
  assert 'ISOLATION_POISON_SENTINEL' not in text, client + ': instruction leak'
  assert 'i-have-adhd' not in text, client + ': personal skill leak'
  if '(file: ' in text:
   import re
   print('Catalog paths:', re.findall(r'\(file: ([^)]+)\)', text))
  assert '(file: ' not in text, client + ': skill catalog paths exposed'
  print(client, 'startup request has no sentinel instructions or skill catalog entries', flush=True)
