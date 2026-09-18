// Live Jev-only comparison. The supplied repository's snippets are sent to Jev.
import { readFile, writeFile, mkdir, realpath } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { parseEnv } from 'node:util';
import { root, rustBinary, hash, workspaceSnapshot, median } from './runtime-utils.mjs';
import { execute, grade, validateFixture, validateResults, summarize, rotateModes } from './compare-codex.mjs';

const [repoArg, fixtureArg] = process.argv.slice(2);
if (!repoArg || !fixtureArg) throw new Error('Usage: node scripts/compare-investigation.mjs REPO FIXTURE');
const repo = await realpath(repoArg);
const fixturePath = resolve(fixtureArg);
const fixtureBytes = await readFile(fixturePath);
const fixture = JSON.parse(fixtureBytes);
validateFixture(fixture);
for (const item of fixture.cases) for (const target of item.expected) {
  if (hash(await readFile(join(repo, target.path))) !== target.sha256) throw new Error(`Fixture source changed: ${target.path}`);
}
let dotenv = {};
try { dotenv = parseEnv(await readFile(join(root, '.env'), 'utf8')); } catch (e) { if (e.code !== 'ENOENT') throw e; }
const key = process.env.TYPESAFE_API_KEY ?? dotenv.TYPESAFE_API_KEY;
if (!key?.trim()) throw new Error('TYPESAFE_API_KEY required');
const env = { ...process.env, TYPESAFE_API_KEY: key };
const modes = ['fast', 'deep'];
const repeats = 3;
const maxSteps = 5;
const directory = join(root, 'benchmarks/results', `deep-comparison-${new Date().toISOString().replaceAll(':', '-')}`);
await mkdir(directory, { recursive: true });
const before = await workspaceSnapshot(repo);
const report = { repo, fixtureHash: hash(fixtureBytes), artifactHash: hash(await readFile(rustBinary)), repository: before,
  repeats, maxSteps, description: fixture.description, timing: 'Fresh process to exit; rotating mode order; no warmup or retries; network included.', runs: [] };
async function save() {
  report.summary = summarize(report.runs, modes).map(row => ({ ...row,
    medianJevCalls: median(report.runs.filter(r => r.mode === row.mode && !r.error).map(r => r.jevCalls)),
    stopReasons: report.runs.filter(r => r.mode === row.mode).reduce((a, r) => { const k = r.investigation?.stopReason ?? 'single_pass'; a[k] = (a[k] ?? 0) + 1; return a; }, {}) }));
  await writeFile(join(directory, 'report.json'), JSON.stringify(report, null, 2) + '\n');
}
console.log(`Report: ${directory}/report.json`);
await save();
trials: for (let repeat = 1; repeat <= repeats; repeat++) for (const [index, item] of fixture.cases.entries()) {
  for (const mode of rotateModes(modes, index + repeat - 1)) {
    const out = await execute(rustBinary, ['ask', item.question, '--json', ...(mode === 'deep' ? ['--deep', '--max-steps', String(maxSteps)] : [])], { cwd: repo, env });
    const run = { mode, repeat, id: item.id, seconds: out.seconds, results: [], ...grade([], item.expected) };
    try {
      if (out.code !== 0 || out.error) throw new Error(out.error ?? `CLI exited ${out.code}`);
      const value = JSON.parse(out.stdout);
      if (value.ranking !== 'jev') throw new Error('Expected Jev ranking');
      run.results = validateResults(value);
      for (const result of run.results) {
        const file = await realpath(join(repo, result.path));
        if (!file.startsWith(repo + '/')) throw new Error('Result escapes repository');
        if (result.endLine > (await readFile(file, 'utf8')).split(/\r\n|\n|\r/).length) throw new Error('Invalid result line range');
      }
      Object.assign(run, grade(run.results, item.expected), { investigation: value.investigation,
        jevCalls: value.investigation?.jevCalls ?? 1 });
    } catch (e) { run.error = e.message; }
    report.runs.push(run);
    await save();
    console.log(`${mode} ${item.id} ${repeat}/${repeats} first=${run.top1} top5=${run.top5} ${run.seconds.toFixed(2)}s calls=${run.jevCalls} stop=${run.investigation?.stopReason ?? 'single_pass'}${run.error ? ' ERROR: ' + run.error : ''}`);
    if (run.error) break trials;
  }
}
report.completed = report.runs.length === fixture.cases.length * repeats * modes.length && report.runs.every(r => !r.error);
report.repositoryUnchanged = JSON.stringify(before) === JSON.stringify(await workspaceSnapshot(repo));
report.artifactsUnchanged = report.artifactHash === hash(await readFile(rustBinary)) && report.fixtureHash === hash(await readFile(fixturePath));
await save();
console.log(JSON.stringify(report.summary));
if (!report.completed || !report.repositoryUnchanged || !report.artifactsUnchanged) process.exitCode = 1;
