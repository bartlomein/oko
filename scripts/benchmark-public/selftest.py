#!/usr/bin/env python3
"""Offline tests of planning, grading, guards, and real fixture contracts."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import runner as r


class RunnerTests(unittest.TestCase):
    def test_plan_is_complete_and_balanced(self):
        from collections import Counter
        plan = r.plan(r.REPOSITORIES, list(r.CLIENTS))
        self.assertEqual(len(plan), 108)
        self.assertEqual(len({(n,t['id'],c,o) for n,t,c,o in plan}),108)
        for c in r.CLIENTS:
            groups = [plan[i:i+3] for i in range(0,len(plan),3) if plan[i][2]==c]
            self.assertEqual(len(groups),12)
            for pos in range(3):
                self.assertEqual(Counter(g[pos][3] for g in groups),dict.fromkeys(r.CONDITIONS,4))
            for g in groups:
                self.assertEqual(len({(n,t['id'],client) for n,t,client,_ in g}),1)
        self.assertEqual(sum(t['kind']=='edit' for _,t,_,_ in plan),54)

    def test_model_tokens_are_not_confused_with_uncached_tokens(self):
        cases=[
            ({'client':'codex','tokens':{'total':120,'input':100,'cachedInput':70,'output':20}},30,70),
            ({'client':'claude','tokens':{'total':120,'input':10,'cacheRead':70,'cacheWrite':20,'output':20}},10,70),
            ({'client':'opencode','tokens':{'total':120,'steps':[{'input':30,'output':15,'reasoning':5,'cache':{'read':70,'write':0}}]}},30,70)]
        for row,uncached,cached in cases:
            value=r.token_breakdown(row)
            self.assertEqual(value['uncachedInput'],uncached)
            self.assertEqual(value['cacheRead'],cached)
            self.assertEqual(value['output'],20)
        self.assertIsNone(r.token_breakdown({}))

    def test_cross_file_search_requires_both_anchors(self):
        task={'kind':'search','expected':[{'path':'a','startLine':4,'endLine':6},{'path':'b','startLine':8,'endLine':10}]}
        with patch.object(r,'original_grade',return_value={'correctFirst':True}):
            one=json.dumps({'results':[task['expected'][0]]})
            self.assertFalse(r.grade(task,Path('.'),one)['passed'])
            both=json.dumps({'results':task['expected']})
            self.assertTrue(r.grade(task,Path('.'),both)['passed'])
            wrong=json.dumps({'results':[{'path':'a','startLine':1,'endLine':3},task['expected'][1]]})
            self.assertFalse(r.grade(task,Path('.'),wrong)['passed'])

    def test_repo_is_passed_to_the_mcp_launcher(self):
        with patch.object(r,'save'), patch.object(r,'original_args',return_value=(['client'],{})):
            _,env=r.args_for({'repositoryName':'astro'},'codex',True,Path('.'),Path('.'))
            self.assertEqual(env['OKO_PUBLIC_BENCH_REPO'],'astro')

    def test_all_edit_contracts_are_red_then_green(self):
        for repo in r.REPOSITORIES:
            path=Path.home()/'dev'/repo['name']
            for task in repo['tasks']:
                with self.subTest(task=task['id']):
                    result=r.fixture_check(path,task)
                    if task['kind']=='edit':
                        self.assertFalse(result['baseline']['passed'])
                        self.assertTrue(result['reference']['passed'])

    def test_stale_source_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            work=Path(temp);(work/'file').write_text('changed')
            task={'id':'stale','kind':'edit','path':'file','sha256':'invalid'}
            with self.assertRaisesRegex(ValueError,'Stale fixture'):
                r.fixture_check(work,task)

    def test_unrelated_edits_fail_before_running_code(self):
        with tempfile.TemporaryDirectory() as temp:
            work=Path(temp);(work/'target').write_text('source')
            with patch.object(r.engine,'git',side_effect=[b'target\nother\n',b'']),patch.object(r,'validate') as validate:
                result=r.grade({'kind':'edit','path':'target'},work,'')
                self.assertFalse(result['passed']);validate.assert_not_called()

    def test_edit_scope_rejects_other_function_changes(self):
        self.assertTrue(r.within_edit_scope('a\nb\nc\n', 'a\nnew\nc\n', [2,2]))
        self.assertFalse(r.within_edit_scope('a\nb\nc\n', 'changed\nb\nc\n', [2,2]))

    def test_real_session_lifecycle_with_fake_client(self):
        import os
        import sys
        task = next(t for repo in r.REPOSITORIES for t in repo['tasks'] if t['id']=='httpx-empty-unquote')
        task = {**task, 'repositoryName':'httpx'}
        # Exercise archive extraction, baseline, process capture, grading, diffs,
        # and cleanup without launching a coding client or making network calls.
        code = "from pathlib import Path; import json; p=Path('httpx/_utils.py'); "
        change = task['replacements'][0]
        code += 'p.write_text(p.read_text().replace(' + repr(change['old']) + ',' + repr(change['new']) + ')); '
        code += "print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':'done'}})); "
        code += "print(json.dumps({'type':'turn.completed','usage':{'input_tokens':10,'cached_input_tokens':0,'output_tokens':2}}))"
        settings = {'clients':{'codex':'unused'},'models':{'codex':'fake'},'effort':'low','timeoutSeconds':20}
        with tempfile.TemporaryDirectory() as temp, patch.object(r.engine,'STATE',r.STATE/'httpx'), patch.object(r.engine,'SETTINGS',settings), patch.object(r.engine,'args_for',return_value=([sys.executable,'-c',code],dict(os.environ))):
            row = r.engine.run_one(task,'codex',False,Path(temp),1)
            self.assertNotIn('error',row)
            self.assertTrue(row['grade']['passed'])
            trial = Path(row['artifact'])
            self.assertTrue((trial/'validation.json').exists())
            self.assertTrue((trial/'changes.patch').read_text())
            self.assertFalse((trial/'workspace').exists())

    def test_cache_assignment_rejects_mismatched_metadata(self):
        disk = {'status':'disk','rebuiltFiles':0,'reusedFiles':10}
        self.assertTrue(r.check_cache('warm',[disk]))
        self.assertFalse(r.check_cache('cold',[disk]))
        self.assertFalse(r.check_cache('warm',[{**disk,'rebuiltFiles':1}]))
        self.assertFalse(r.check_cache('warm',[]))
        self.assertTrue(r.check_cache('native',[]))

    def test_execute_preserves_conditions_and_existing_session_guard(self):
        from types import SimpleNamespace
        disk = {'status':'disk','rebuiltFiles':0,'reusedFiles':10}
        settings = dict(repository='source', commit='commit', clients={'codex':'codex'},
                        versions={'codex':'v'}, oko='oko', archiveSha256='hash',
                        okoSha256='hash', tasksSha256='hash', implementationSha256='hash',
                        isolation=r.engine.ISOLATION_VERSION)
        for condition in r.CONDITIONS:
            with self.subTest(condition=condition), tempfile.TemporaryDirectory() as temp:
                root=Path(temp)
                r.save(root/'astro/settings.json',settings)
                args=SimpleNamespace(clients=['codex'],resume=None)
                task={'id':'example','kind':'search'}
                expected=r.ENGINE_CONDITIONS[condition]
                observations=[] if condition=='native' else [disk if condition=='warm' else {'status':'cold'}]
                def run_one(task,client,requested,trials,index):
                    self.assertEqual(requested,expected)
                    trial=trials/f'{index:03}-{task["id"]}-{client}-{requested}'
                    trial.mkdir()
                    if condition=='warm':
                        r.save(trial/'warmup.json',{'providerCalls':0,'seconds':0.1})
                    return dict(id=task['id'],client=client,artifact=str(trial),seconds=1,
                                grade={'passed':True},oko=condition!='native')
                with patch.object(r,'STATE',root), patch.object(r.engine,'STATE',root), \
                     patch.object(r.engine,'SETTINGS',settings), patch.object(r,'digest',return_value='hash'), \
                     patch.object(r,'implementation_digest',return_value='hash'), \
                     patch.object(r,'state',return_value={'commit':'commit','status':''}), \
                     patch.object(r.subprocess,'check_output',return_value='v'), \
                     patch.object(r,'cache_observations',return_value=observations), \
                     patch.object(r.engine,'run_one',side_effect=run_one) as run:
                    schedule=[('astro',task,'codex',condition)]
                    r.execute(args,schedule)
                    output=next(root.glob('results-*'))
                    report=json.loads((output/'report.json').read_text())
                    self.assertTrue(report['complete'])
                    # A session without a saved report row must never be paid for twice.
                    report['runs']=[]
                    r.save(output/'report.json',report)
                    args.resume=output
                    with self.assertRaisesRegex(RuntimeError,'Existing unrecorded session'):
                        r.execute(args,schedule)
                    self.assertEqual(run.call_count,1)

    def test_warmup_is_owned_by_shared_session_lifecycle(self):
        with patch.object(r,'save'), patch.object(r,'prewarm',return_value={'providerCalls':0}) as warmup, \
             patch.object(r,'original_args',return_value=(['client'],{})):
            r.engine.prewarm(Path('work'),Path('trial'))
            r.args_for({'repositoryName':'astro','cacheCondition':'warm'},
                       'codex','oko-warm',Path('work'),Path('trial'))
            warmup.assert_called_once_with(Path('work'),Path('trial'),r.engine.SETTINGS)

    def test_cache_metadata_parsing_for_three_clients(self):
        packet = {'timings':{'cache':{'status':'disk','rebuiltFiles':0,'reusedFiles':10}}}
        for tool in [
            {'server':'oko','tool':'search','result':{'structured_content':packet}},
            {'tool':'oko_search','state':{'output':json.dumps(packet)}},
        ]:
            result = r.cache_observations({'client':'codex','tools':[tool]})
            self.assertTrue(r.check_cache('warm',result))
        with tempfile.TemporaryDirectory() as temp:
            event = {'type':'user','message':{'content':[{'type':'tool_result','tool_use_id':'one','content':json.dumps(packet)}]}}
            (Path(temp)/'events.jsonl').write_text(json.dumps(event)+'\n')
            row = {'client':'claude','artifact':temp,'tools':[{'name':'mcp__oko__search','id':'one'}]}
            self.assertTrue(r.check_cache('warm',r.cache_observations(row)))

    def test_real_warm_index_and_edit_freshness_offline(self):
        import importlib.util
        import shutil
        spec = importlib.util.spec_from_file_location('cache_profiler',r.ROOT.parent/'profile-cache.py')
        profiler = importlib.util.module_from_spec(spec);spec.loader.exec_module(profiler)
        with tempfile.TemporaryDirectory() as temp:
            trial=Path(temp);work=trial/'workspace';work.mkdir()
            for i in range(20):
                (work/f'item{i}.rs').write_text(f'pub fn item{i}() -> u32 {{ {i} }}\n')
            target=work/'retry.rs';target.write_text('pub fn retry_delay() -> u32 { 500 }\n')
            settings={'oko':str(r.PROJECT/'target/release/oko'),'rg':shutil.which('rg')}
            warmup=r.prewarm(work,trial,settings)
            self.assertEqual(warmup['providerCalls'],0)
            client=profiler.Client(Path(settings['oko']),work,trial/'cache',30)
            try:
                client.initialize()
                def search():
                    result=client.request('tools/call',{'name':'search','arguments':{'question':'retry_delay','intent':'implementation'}})
                    self.assertFalse(result.get('isError'))
                    return client.last_metrics()
                packet=search()
                self.assertTrue(r.check_cache('warm',[packet['timings']['cache']]))
                target.write_text('pub fn retry_delay() -> u32 { 300 }\n')
                updated=search()
                snippets='\n'.join(x['text'] for x in updated['results'] if x['path']=='retry.rs')
                self.assertIn('300',snippets)
                self.assertNotIn('500',snippets)
            finally:
                client.close()

    def test_branch_plan_pairs_versions_and_balances_each_task(self):
        from collections import Counter
        repos=json.loads((r.ROOT/'tasks-branch.json').read_text())['repositories']
        with patch.object(r,'CONDITIONS',('native','previous','current')):
            plan=r.plan(repos,list(r.CLIENTS),3)
        self.assertEqual(len(plan),243)
        self.assertEqual(len({(n,t['id'],c,k,t['repetition']) for n,t,c,k in plan}),243)
        for repo in repos:
            for task in repo['tasks']:
                for client in r.CLIENTS:
                    groups=[plan[i:i+3] for i in range(0,len(plan),3)
                            if plan[i][1]['id']==task['id'] and plan[i][2]==client]
                    self.assertEqual(len(groups),3)
                    for position in range(3):
                        self.assertEqual(Counter(g[position][3] for g in groups),
                                         {'native':1,'previous':1,'current':1})
        old={t['id'] for repo in r.REPOSITORIES for t in repo['tasks']}
        self.assertFalse(old & {t['id'] for repo in repos for t in repo['tasks']})

    def test_token_components_work_without_provider_total(self):
        for row,expected in [
            ({'client':'codex','tokens':{'input':100,'cachedInput':70,'output':20,'total':None}},120),
            ({'client':'claude','tokens':{'input':10,'cacheRead':70,'cacheWrite':20,'output':20,'total':None}},120),
            ({'client':'opencode','tokens':{'steps':[{'input':30,'output':15,'reasoning':5,'cache':{'read':70,'write':0}}],'total':None}},120),
        ]:
            self.assertEqual(r.token_breakdown(row)['total'],expected)
            self.assertEqual(r.token_breakdown(row)['totalSource'],'derived-from-components')
        self.assertIsNone(r.token_breakdown({'client':'codex','tokens':{'input':100,'cachedInput':None,'output':2}}))
        self.assertIsNone(r.token_breakdown({'client':'codex','tokens':{'input':10,'cachedInput':20,'output':2}}))
        self.assertIsNone(r.measurements({'oko':True})['jev']['inputTokens'])
        self.assertIsNone(r.measurements({'oko':True})['okoSearchSeconds'])

    def test_jev_measurements_support_both_build_formats_without_double_counting(self):
        packet={'timings':{'totalMs':600},'retrieval':{'jevCalls':[{'durationNs':500000000,'usage':{'inputTokens':100,'outputTokens':20}}]}}
        for tool in [
            {'result':{'structuredContent':packet,'content':[{'text':json.dumps(packet)}]}},
            {'okoMetrics':[packet]},
        ]:
            result=r.measurements({'oko':True,'tools':[tool]})
            self.assertEqual(result['jev']['inputTokens'],100)
            self.assertEqual(result['jev']['calls'],1)
            self.assertEqual(result['okoSearchSeconds'],0.6)
            self.assertIsNone(result['jev']['cacheReadTokens'])

    def test_new_fixtures_fail_before_and_pass_after(self):
        repos=json.loads((r.ROOT/'tasks-branch.json').read_text())['repositories']
        for repo in repos:
            for task in repo['tasks']:
                with self.subTest(task=task['id']):
                    result=r.fixture_check(Path.home()/'dev'/repo['name'],task)
                    if task['kind']=='edit':
                        self.assertFalse(result['baseline']['passed'])
                        self.assertTrue(result['reference']['passed'])

    def test_legacy_anchor_accepts_precise_hidden_wiring(self):
        task=next(t for repo in r.REPOSITORIES for t in repo['tasks'] if t['id']=='ripgrep-hidden-walk')
        self.assertEqual(task['expected'][0]['startLine'],906)
        self.assertEqual(task['expected'][0]['endLine'],906)
        with patch.object(r,'original_grade',return_value={'correctFirst':True}):
            answer=json.dumps({'results':[{k:v for k,v in e.items() if k!='sha256'} for e in task['expected']]})
            self.assertTrue(r.grade(task,Path('.'),answer)['passed'])
        self.assertTrue(r.within_edit_scope('a\nb\nc\nd\n','A\nb\nc\nD\n',[[1,1],[4,4]]))
        self.assertFalse(r.within_edit_scope('a\nb\nc\nd\n','A\nB\nc\nD\n',[[1,1],[4,4]]))

    def test_session_settings_and_opencode_state_are_private(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(r,'SUITE','branch'), \
             patch.object(r,'original_args',return_value=(['client'],{})), \
             patch.object(r.engine,'SETTINGS',{'oko':'selected-build'}):
            root=Path(tmp)
            for label in ('seed','probe'):
                trial=root/label;trial.mkdir();work=trial/'workspace';work.mkdir()
                _,env=r.args_for({'repositoryName':'astro'},'opencode','native',work,trial)
                self.assertEqual(json.loads((trial/'settings.json').read_text())['oko'],'selected-build')
                for name in ('DATA','STATE','CACHE'):
                    self.assertTrue(Path(env['XDG_'+name+'_HOME']).is_relative_to(trial))

    def test_memory_canary_fails_closed_on_leakage(self):
        import isolation_check
        from unittest.mock import Mock
        fake=Mock()
        fake.engine.SETTINGS={}
        fake.args_for.return_value=(['fake'],{})
        fake.save=r.save
        def run(secret_recalled):
            fake.engine.parse_events.side_effect=[
                dict(complete=True,providerErrors=[],tools=[],final='READY'),
                dict(complete=True,providerErrors=[],tools=[],final=secret_recalled)]
            with tempfile.TemporaryDirectory() as tmp, patch.object(isolation_check.subprocess,'Popen') as proc:
                proc.return_value.returncode=0
                return isolation_check.memory_canary(fake,{'timeoutSeconds':1},Path(tmp),['codex'])
        self.assertTrue(run('NO_MEMORY')['passed'])
        with self.assertRaisesRegex(RuntimeError,'Memory canary failed'):
            run('CANARY_leaked_from_previous_session')

    def test_launcher_uses_trial_binary_not_repository_settings(self):
        import os
        import subprocess
        import sys
        state=r.PROJECT/'benchmarks/results/public-branch/astro'
        state.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='results-launcher-',dir=state) as tmp:
            for label in ('previous','current'):
                trial=Path(tmp)/label/'trial';work=trial/'workspace';work.mkdir(parents=True)
                binary=trial/'fake-oko'
                binary.write_text('#!'+sys.executable+'\nprint('+repr(label)+')\n')
                binary.chmod(0o700)
                r.save(trial/'settings.json',dict(oko=str(binary),rg='rg',jevModel='offline'))
                result=subprocess.run([sys.executable,str(r.ROOT/'oko-server.py'),str(work),str(trial/'cache')],
                                      env=dict(os.environ,TYPESAFE_API_KEY='offline-test'),capture_output=True,text=True,check=True)
                self.assertEqual(result.stdout.strip(),label)

    def test_both_built_versions_observe_assigned_cache_offline(self):
        import importlib.util
        import shutil
        spec=importlib.util.spec_from_file_location('cache_probe',r.ROOT.parent/'profile-cache.py')
        profiler=importlib.util.module_from_spec(spec);spec.loader.exec_module(profiler)
        builds=list((r.PROJECT/'benchmarks/results/public-branch/builds').glob('*/build.json'))
        self.assertGreaterEqual(len(builds),2,'Prepare the branch builds first')
        for record in builds:
            binary=Path(json.loads(record.read_text())['path'])
            for condition in ('cold','warm'):
                with self.subTest(build=binary.parent.name,condition=condition),tempfile.TemporaryDirectory() as tmp:
                    trial=Path(tmp);work=trial/'workspace';work.mkdir()
                    (work/'sample.rs').write_text('pub fn retry_delay() -> u32 { 123 }\n')
                    if condition=='warm':
                        r.prewarm(work,trial,dict(oko=str(binary),rg=shutil.which('rg')))
                    client=profiler.Client(binary,work,trial/'cache',30,prewarm=True)
                    try:
                        client.initialize()
                        result=client.request('tools/call',{'name':'search','arguments':{'question':'retry_delay','intent':'implementation'}})
                        self.assertFalse(result.get('isError'))
                        observed=client.prewarm()
                        if observed is None:
                            packet=result.get('structuredContent') or client.last_metrics()
                            observed=packet['timings']['cache']
                        self.assertEqual(observed['status'],'cold' if condition=='cold' else 'disk')
                        if condition=='warm':
                            self.assertEqual(observed['rebuiltFiles'],0)
                            self.assertGreater(observed['reusedFiles'],0)
                    finally:client.close()

    def test_branch_execution_selects_each_binary_with_same_cache_policy(self):
        from types import SimpleNamespace
        import isolation_check
        settings=dict(builds={label:{'path':label,'sha256':'hash'} for label in ('previous','current')})
        seen=[]
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            task=dict(id='task',kind='search',repetition=1)
            args=SimpleNamespace(clients=['codex'],resume=None,repeats=1)
            def one(task,client,condition,trials,index):
                seen.append((condition,r.engine.SETTINGS['oko']))
                trial=trials/f'{index:03}-task-{client}-{condition}';trial.mkdir()
                if condition=='oko-warm':r.save(trial/'warmup.json',{'seconds':0.1})
                return dict(id='task',client=client,artifact=str(trial),seconds=1,grade={'passed':True})
            # Exercise the real execution loop. Frozen-state verification is tested separately.
            settings.update(repository='source',commit='commit',archiveSha256='hash',oko='current',okoSha256='hash',tasksSha256='hash')
            with patch.object(r,'SUITE','branch'),patch.object(r,'STATE',root), \
                 patch.object(r.engine,'STATE',root),patch.object(r.engine,'SETTINGS',{}), \
                 patch.object(r,'CONDITIONS',('native','previous','current')), \
                 patch.object(r,'ENGINE_CONDITIONS',{'native':'native','previous':'oko-warm','current':'oko-warm'}), \
                 patch.object(r,'verify_settings',return_value={'astro':settings}), \
                 patch.object(r,'state',return_value={'commit':'commit','status':''}), \
                 patch.object(r,'digest',return_value='hash'), \
                 patch.object(r,'cache_observations',return_value=[]), \
                 patch.object(r,'check_cache',return_value=True), \
                 patch.object(isolation_check,'memory_canary',return_value={'passed':True,'complete':True}), \
                 patch.object(r.engine,'run_one',side_effect=one):
                r.execute(args,[('astro',task,'codex',c) for c in ('native','previous','current')])
            self.assertEqual(seen,[('native','current'),('oko-warm','previous'),('oko-warm','current')])

    def test_frozen_branch_verification_rejects_modified_binary(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp)
            settings=dict(repository='source',commit='commit',oko='current',archiveSha256='hash',
                          okoSha256='hash',tasksSha256='hash',implementationSha256='hash',
                          isolation=r.engine.ISOLATION_VERSION,repeats=3,cachePolicy='warm',
                          builds={'previous':{'path':'previous','sha256':'expected'},'current':{'path':'current','sha256':'hash'}})
            r.save(state/'astro/settings.json',settings)
            with patch.object(r,'STATE',state),patch.object(r,'SUITE','branch'), \
                 patch.object(r,'digest',return_value='hash'),patch.object(r,'implementation_digest',return_value='hash'):
                with self.assertRaisesRegex(RuntimeError,'Frozen build changed'):
                    r.verify_settings(SimpleNamespace(repeats=3,cache_policy='warm'),[('astro',{},'codex','previous')])

    def test_branch_report_uses_all_repetitions_and_paired_differences(self):
        from reporting import render
        rows=[]
        for repeat,old,new in [(1,10,5),(2,20,10),(3,30,15)]:
            for condition,seconds in [('native',old),('previous',old),('current',new)]:
                rows.append(dict(repository='astro',id='task',client='codex',condition=condition,
                                 repetition=repeat,seconds=seconds,grade={'passed':True}))
        text=render(dict(runs=rows,plan=rows,complete=True,suite='branch',repeats=3),['codex'],['native','previous','current'])
        self.assertIn('20.00s | 20.00s | 10.00s',text)
        self.assertIn('| codex | previous | 3 | -10.00 | -50.0% |',text)
        self.assertIn('unavailable',text)

    def test_smoke_suite_is_a_short_build_comparison_without_native_or_memory_canary(self):
        import subprocess, sys
        def run(*extra):
            return subprocess.run([sys.executable,str(r.ROOT/'runner.py'),'--suite','smoke',*extra],capture_output=True,text=True)
        listed=run()
        self.assertEqual(listed.returncode,0,listed.stderr)
        lines=listed.stdout.splitlines()
        self.assertIn("24 sessions; suite=smoke; repeats=3; conditions=('previous', 'current')",lines[0])
        sessions=[line.split() for line in lines[1:]]
        self.assertEqual({s[-2] for s in sessions},{'claude'})
        self.assertEqual({s[-3] for s in sessions},set(r.SMOKE_TASKS))
        # Every task and repetition compares both builds, and neither build always runs first.
        pairs={}
        for s in sessions:pairs.setdefault((s[1],s[-3]),[]).append(s[-1])
        self.assertTrue(all(sorted(v)==['current','previous'] for v in pairs.values()))
        self.assertEqual({v[0] for v in pairs.values()},{'current','previous'})
        self.assertEqual({t for repo in json.loads((r.ROOT/'tasks-branch.json').read_text())['repositories']
                          for t in [x['kind'] for x in repo['tasks'] if x['id'] in r.SMOKE_TASKS]},{'search','edit'})
        chosen=run('--tasks','httpx-decoder-chain','--clients','claude,codex','--repeats','1')
        self.assertIn('4 sessions',chosen.stdout.splitlines()[0])
        self.assertNotEqual(run('--tasks','not-a-task').returncode,0)
        self.assertNotEqual(subprocess.run([sys.executable,str(r.ROOT/'runner.py'),'--tasks','httpx-decoder-chain'],
                                           capture_output=True).returncode,0)
        with patch.object(r,'SUITE','smoke'):
            self.assertTrue(r.compares_builds())
        self.assertFalse(r.compares_builds())

    def test_smoke_report_pairs_builds_and_states_its_limits(self):
        from reporting import render
        rows=[dict(repository='astro',id='task',client='claude',condition=condition,repetition=repeat,
                   seconds=seconds,grade={'passed':condition=='current'})
              for repeat in (1,2,3) for condition,seconds in (('previous',10),('current',8))]
        text=render(dict(runs=rows,plan=rows,complete=True,suite='smoke',repeats=3),['claude'],['previous','current'])
        self.assertIn('| claude | previous | 0/3 | 10.00 |',text)
        self.assertIn('| claude | current | 3/3 | 8.00 |',text)
        self.assertIn('| claude | previous | 3 | -2.00 | -20.0% |',text)
        self.assertNotIn('| claude | native |',text)
        self.assertIn('supports no speed or quality claim',text)

    def test_guided_condition_delivers_setup_guidance_through_each_clients_own_channel(self):
        text=r.GUIDANCE.read_text()
        self.assertIn('Treat each excerpt as a file read you have already done',text)
        with tempfile.TemporaryDirectory() as tmp:
            trial=Path(tmp); home=trial/'codex-home'; home.mkdir()
            args,env=r.guide('codex',['codex','exec','PROMPT'],{'CODEX_HOME':str(home)},trial)
            self.assertEqual((home/'AGENTS.md').read_text(),text)
            self.assertEqual(args,['codex','exec','PROMPT'],'the checkout and the command stay untouched')
            args,env=r.guide('opencode',['opencode','run','PROMPT'],{'OPENCODE_CONFIG_CONTENT':json.dumps({'mcp':{}})},trial)
            config=json.loads(env['OPENCODE_CONFIG_CONTENT'])
            self.assertEqual(Path(config['instructions'][0]).read_text(),text)
            self.assertIn('mcp',config)
            args,env=r.guide('claude',['claude','-p','PROMPT'],{},trial)
            self.assertEqual(args,['claude','-p','--append-system-prompt',text,'PROMPT'])
        task={'kind':'search','question':'Where is it?','cacheCondition':'guided'}
        guided=r.prompt(task,True); plain=r.prompt({**task,'cacheCondition':'current'},True)
        self.assertIn("The project's standing instructions about Oko search apply.",guided)
        self.assertNotIn('AGENTS.md',guided)
        self.assertIn('do not load skills, personal instructions, AGENTS.md, CLAUDE.md, or saved memory.',plain)
        # Identical otherwise: the condition differs only in the guidance.
        self.assertEqual(guided.replace("do not load skills, personal instructions, or saved memory. The project's standing instructions about Oko search apply.",''),
                         plain.replace('do not load skills, personal instructions, AGENTS.md, CLAUDE.md, or saved memory.',''))
        import subprocess, sys
        listed=subprocess.run([sys.executable,str(r.ROOT/'runner.py'),'--suite','smoke','--guided','--repeats','1'],capture_output=True,text=True)
        self.assertIn("conditions=('previous', 'current', 'guided')",listed.stdout)
        self.assertNotEqual(subprocess.run([sys.executable,str(r.ROOT/'runner.py'),'--guided'],capture_output=True).returncode,0)

    def test_report_includes_failed_attempts(self):
        with tempfile.TemporaryDirectory() as temp:
            data={'plan':[{},{}],'complete':True,'runs':[
                {'repository':'astro','id':'task','client':'codex','oko':False,'condition':'native','seconds':100,'error':'timeout'},
                {'repository':'astro','id':'task','client':'codex','oko':True,'condition':'cold','seconds':10,'grade':{'passed':True}}]}
            r.report(Path(temp),data)
            text=(Path(temp)/'report.md').read_text()
            self.assertIn('100.00',text)
            self.assertIn('0/1',text)
            self.assertIn('unavailable',text)


if __name__=='__main__':
    unittest.main()
