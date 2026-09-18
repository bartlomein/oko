import test from 'node:test';
import assert from 'node:assert/strict';
import { grade, validateResults, summarize, codexOutput, execute } from './compare-codex.mjs';
const result = { path: 'src/auth.rs', startLine: 15, endLine: 18 };
const expected = [{ path: 'src/auth.rs', startLine: 17, endLine: 20, function: { startLine: 10, endLine: 30 } }];
test('grades exact implementation and function separately, including wrong files', () => {
  assert.equal(grade([result], expected).top1, true);
  assert.deepEqual(grade([{ ...result, startLine: 11, endLine: 13 }], expected),
    { top1: false, top5: false, functionTop1: true, functionTop5: true });
  assert.equal(grade([{ ...result, path: 'wrong.rs' }], expected).top5, false);
  assert.equal(grade([], expected).top1, false);
});
test('rejects invalid, broad, or outside-repository ranges', () => {
  for (const change of [{ path: '../secret' }, { path: '/tmp/file' }, { startLine: 0 }, { endLine: 200 }, { endLine: 2 }, { startLine: 1.5 }]) {
    assert.throws(() => validateResults({ results: [{ ...result, ...change }] }));
  }
  assert.throws(() => validateResults({ results: Array(6).fill(result) }));
  assert.deepEqual(validateResults({ results: [{ ...result, text: 'discard source' }] }), [result]);
});
test('failed runs count as misses and never improve successful median', () => {
  const rows = [{ mode: 'oko', seconds: 2, top1: true }, { mode: 'oko', seconds: 100, error: 'timeout', top1: true }];
  const summary = summarize(rows)[0];
  assert.equal(summary.runs, 2); assert.equal(summary.errors, 1);
  assert.equal(summary.top1, 1); assert.equal(summary.medianSeconds, 2);
  assert.equal(summarize([])[0].medianSeconds, null);
});
test('extracts schema response and usage without saving raw tool output', () => {
  const events = [
    { type: 'item.completed', item: { type: 'command_execution', command: 'rg token', aggregated_output: 'private source text' } },
    { type: 'item.completed', item: { type: 'agent_message', text: JSON.stringify({ results: [result] }) } },
    { type: 'turn.completed', usage: { input_tokens: 42, output_tokens: 12 } },
  ];
  const output = codexOutput(events.map(e => JSON.stringify(e)).join('\n'));
  assert.deepEqual(output.results, [result]); assert.equal(output.usage.input_tokens, 42);
  assert.equal(output.commandCount, 1); assert.ok(!JSON.stringify(output).includes('private source'));
  assert.throws(() => codexOutput(JSON.stringify({ type: 'turn.failed' })));
  assert.throws(() => codexOutput(''));
});
test('process failure and timeout are bounded outcomes with elapsed time', async () => {
  const options = { cwd: process.cwd(), env: process.env, timeout: 1000 };
  const failed = await execute(process.execPath, ['-e', 'process.exit(3)'], options);
  assert.equal(failed.code, 3); assert.ok(failed.seconds > 0);
  const timed = await execute(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { ...options, timeout: 80 });
  assert.equal(timed.error, 'Timed out'); assert.ok(timed.seconds < 2);
});

import { openCodeOutput, openCodeConfig, rotateModes, validateFixture } from './compare-codex.mjs';
import { readFile } from 'node:fs/promises';
test('OpenCode extracts only the completed final message and rejects incomplete or failed output', () => {
  const events = [
    { type: 'text', part: { messageID: 'old', text: 'Searching...' } },
    { type: 'tool_use', part: { tool: 'grep', state: { status: 'completed', output: 'private source' } } },
    { type: 'step_finish', part: { messageID: 'old', reason: 'tool-calls', tokens: { input: 10 } } },
    { type: 'text', part: { messageID: 'final', text: JSON.stringify({ results: [result] }) } },
    { type: 'step_finish', sessionID: 'session', part: { messageID: 'final', reason: 'stop', tokens: { input: 20 } } },
  ];
  const encode = rows => rows.map(e => JSON.stringify(e)).join('\n');
  const parsed = openCodeOutput(encode(events));
  assert.deepEqual(parsed.results, [result]);
  assert.deepEqual(parsed.toolTypes, ['grep']);
  assert.equal(parsed.usage.length, 2);
  assert.ok(!JSON.stringify(parsed).includes('private source'));
  assert.throws(() => openCodeOutput(encode(events.slice(0, 3))));
  assert.throws(() => openCodeOutput(encode([...events, { type: 'error' }])));
  assert.throws(() => openCodeOutput(''));
});
test('three tools rotate through every position, and errors remain in denominators', () => {
  const modes = ['oko', 'codex', 'opencode'];
  assert.deepEqual(rotateModes(modes, 0), modes);
  assert.deepEqual(rotateModes(modes, 1), ['codex', 'opencode', 'oko']);
  assert.deepEqual(rotateModes(modes, 2), ['opencode', 'oko', 'codex']);
  assert.deepEqual(rotateModes(modes, 3), modes);
  const rows = summarize([{ mode: 'opencode', seconds: 1, error: 'failed', top1: true }], modes);
  assert.equal(rows[2].runs, 1); assert.equal(rows[2].top1, 0);
  assert.equal(rows[2].medianSeconds, null);
});
test('expanded fixture has 5 development and 10 new cases with validated ranges', async () => {
  const fixture = JSON.parse(await readFile(new URL('../benchmarks/telemetry-studio-expanded.json', import.meta.url)));
  validateFixture(fixture);
  assert.equal(fixture.cases.filter(c => c.group === 'development').length, 5);
  assert.equal(fixture.cases.filter(c => c.group === 'new').length, 10);
  assert.throws(() => validateFixture({ cases: [fixture.cases[0], fixture.cases[0]] }));
  const bad = structuredClone(fixture); bad.cases[0].expected[0].path = '../outside';
  assert.throws(() => validateFixture(bad));
  const permission = openCodeConfig().agent.oko_benchmark.permission;
  assert.equal(permission['*'], 'deny'); assert.equal(permission.read['*.env'], 'deny');
});

test('both agent adapters run offline with one shared model and portable fixture', async () => {
  const { mkdtemp, writeFile, chmod, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join, resolve } = await import('node:path');
  const { hash, runProcess } = await import('./runtime-utils.mjs');
  const directory = await mkdtemp(join(tmpdir(), 'oko-agent-test-'));
  let reportDirectory;
  try {
    await writeFile(join(directory, 'answer.rs'), 'fn answer() {}\n');
    await runProcess('git', ['init', '-q', directory]);
    const commit = await runProcess('git', ['-C', directory, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'fixture']);
    assert.equal(commit.code, 0);
    const fixture = { description: 'Synthetic adapter check', cases: [{ id: 'answer', question: 'Where is the answer?', group: 'new', expected: [{ path: 'answer.rs', startLine: 1, endLine: 1, sha256: hash('fn answer() {}\n'), function: { startLine: 1, endLine: 1 } }] }] };
    const fixturePath = join(directory, 'fixture.json');
    await writeFile(fixturePath, JSON.stringify(fixture));
    const executable = join(directory, 'fake-cli.mjs');
    await writeFile(executable, `#!${process.execPath}\n` + `
      import assert from 'node:assert/strict';
      const args = process.argv.slice(2);
      if (args[0] === '--version') { console.log('fake-cli 1'); process.exit(0); }
      assert.equal(process.env.TYPESAFE_API_KEY, undefined);
      const openCode = args[0] === 'run';
      assert.equal(args[args.indexOf('--model') + 1], openCode ? 'openai/gpt-5.5' : 'gpt-5.5');
      if (openCode) {
        assert.equal(args[args.indexOf('--variant') + 1], 'medium');
        assert.equal(JSON.parse(process.env.OPENCODE_CONFIG_CONTENT).agent.oko_benchmark.permission['*'], 'deny');
      } else assert.ok(args.includes('model_reasoning_effort="medium"'));
      const text = JSON.stringify({results:[{path:'answer.rs',startLine:1,endLine:1}]});
      const events = openCode ? [
        {type:'text',part:{messageID:'final',text}},
        {type:'step_finish',part:{messageID:'final',reason:'stop'}}
      ] : [
        {type:'item.completed',item:{type:'agent_message',text}},
        {type:'turn.completed'}
      ];
      for (const event of events) console.log(JSON.stringify(event));
    `);
    await chmod(executable, 0o755);
    const run = await execute(process.execPath, ['scripts/compare-agents.mjs', directory, '--tools', 'codex,opencode', '--fixture', fixturePath, '--repeats', '1', '--codex-bin', executable, '--opencode-bin', executable],
      { cwd: resolve('.'), env: { ...process.env, TYPESAFE_API_KEY: 'fake-key-not-forwarded' }, timeout: 10000 });
    assert.equal(run.code, 0, run.stdout);
    const reportPath = run.stdout.match(/Report: (.+\/report.md)/)?.[1].replace(/\.md$/, '.json');
    assert.ok(reportPath);
    reportDirectory = resolve(reportPath, '..');
    const report = JSON.parse(await readFile(reportPath));
    assert.equal(report.completed, true);
    assert.equal(report.repositoryUnchanged, true);
    assert.equal(report.artifactsUnchanged, true);
    assert.deepEqual(report.summary.map(s => s.top1), [1, 1]);
    assert.equal(report.groups.new.length, 2);
    assert.ok(!JSON.stringify(report).includes('fake-key-not-forwarded'));
  } finally {
    await rm(directory, { recursive: true, force: true });
    if (reportDirectory) await rm(reportDirectory, { recursive: true, force: true });
  }
});
