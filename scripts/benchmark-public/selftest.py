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
        with patch.object(r,'original_args',return_value=(['client'],{})):
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
