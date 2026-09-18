import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { config } from 'dotenv';

const exec = promisify(execFile);
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function scoreResults(results, expected) {
  const hits = (functionLocation) => results.slice(0, 5).map(result => expected.some(target => {
    const range = functionLocation ? target.function : target;
    return range && result.path === target.path &&
      result.startLine <= range.endLine && result.endLine >= range.startLine;
  }));
  const exact = hits(false);
  const functions = hits(true);
  return { top1: exact[0] === true, top5: exact.includes(true),
    functionTop1: functions[0] === true, functionTop5: functions.includes(true) };
}

export function summarize(runs) {
  return ['lexical', 'jev'].map(mode => {
    const selected = runs.filter(run => run.mode === mode);
    const successful = selected.filter(run => !run.error);
    return { mode, runs: selected.length, errors: selected.length - successful.length,
      top1: selected.filter(run => run.top1).length, top5: selected.filter(run => run.top5).length,
      functionTop1: selected.filter(run => run.functionTop1).length,
      functionTop5: selected.filter(run => run.functionTop5).length,
      medianSeconds: successful.length ? median(successful.map(run => run.seconds)) : null };
  });
}

export function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

async function snapshot(repo, cases) {
  const git = async (...args) => (await exec('git', ['-C', repo, ...args])).stdout.trim();
  const hashes = {};
  for (const item of cases) {
    for (const target of item.expected) {
      const bytes = await readFile(resolve(repo, target.path));
      const hash = createHash('sha256').update(bytes).digest('hex');
      if (hash !== target.sha256) {
        throw new Error(`Fixture is stale: ${target.path}. Review its expected lines before updating its hash.`);
      }
      hashes[target.path] = hash;
    }
  }
  return { commit: await git('rev-parse', 'HEAD'), status: await git('status', '--porcelain'), hashes };
}

async function main() {
  const [repoArg = '../telemetry-studio', repeatsArg = '1', baselineArg, ...extra] = process.argv.slice(2);
  const repeats = Number(repeatsArg);
  if (extra.length || !Number.isInteger(repeats) || repeats < 1 || repeats > 10) {
    throw new Error('Usage: npm run benchmark -- /path/to/telemetry-studio [repeats: 1-10] [baseline-report.json]');
  }
  const repo = resolve(repoArg);
  const fixture = JSON.parse(await readFile(resolve(root, 'benchmarks/telemetry-studio.json'), 'utf8'));
  const before = await snapshot(repo, fixture.cases);
  let baseline;
  if (baselineArg) {
    const saved = JSON.parse(await readFile(resolve(baselineArg), 'utf8'));
    if (!saved.repositoryUnchanged || saved.repository.commit !== before.commit ||
        saved.repository.status !== before.status ||
        JSON.stringify(saved.repository.hashes) !== JSON.stringify(before.hashes)) {
      throw new Error('Baseline repository snapshot differs; use a comparable baseline.');
    }
    const rescored = saved.runs.map(run => {
      const item = fixture.cases.find(item => item.id === run.id && item.question === run.question);
      if (!item) throw new Error(`Baseline question differs: ${run.id}`);
      return { ...run, ...scoreResults(run.error ? [] : run.results, item.expected) };
    });
    baseline = { path: resolve(baselineArg), startedAt: saved.startedAt, summary: summarize(rescored) };
  }
  const env = { ...process.env };
  const loaded = config({ path: resolve(root, '.env'), quiet: true, processEnv: env });
  if (loaded.error && loaded.error.code !== 'ENOENT') throw loaded.error;
  if (!env.TYPESAFE_API_KEY?.trim()) throw new Error('Add TYPESAFE_API_KEY to Oko/.env before benchmarking.');
  await exec('rg', ['--version'], { env });
  const startedAt = new Date().toISOString();
  const runs = [];
  console.log(`Testing ${fixture.cases.length} questions, ${repeats} repeat(s); at most ${fixture.cases.length * repeats} Jev requests.`);
  for (let repeat = 0; repeat < repeats; repeat++) {
    for (const [index, item] of fixture.cases.entries()) {
      // Alternate order to reduce systematic filesystem-cache advantages.
      const modes = (index + repeat) % 2 ? ['jev', 'lexical'] : ['lexical', 'jev'];
      for (const mode of modes) {
        const args = [resolve(root, 'dist/cli.js'), 'ask', item.question, '--json'];
        if (mode === 'lexical') args.push('--no-jev');
        const start = performance.now();
        let result;
        try {
          const { stdout } = await exec(process.execPath, args, {
            cwd: repo, env, timeout: 60_000, maxBuffer: 4 * 1024 * 1024,
          });
          const output = JSON.parse(stdout);
          if (output.ranking !== mode || !Array.isArray(output.results) ||
              output.results.some(r => typeof r.path !== 'string' ||
                !Number.isInteger(r.startLine) || !Number.isInteger(r.endLine) ||
                r.startLine < 1 || r.endLine < r.startLine)) {
            throw new Error('Invalid CLI output');
          }
          result = {
            ...scoreResults(output.results, item.expected),
            // Keep locations only: no code snippets, credentials or API response bodies.
            results: output.results.map(({ path, startLine, endLine }) => ({ path, startLine, endLine })),
          };
        } catch (error) {
          result = { ...scoreResults([], item.expected), results: [],
            error: error.killed ? 'Timed out after 60 seconds' : 'CLI failed or returned invalid output' };
        }
        const run = { id: item.id, question: item.question, repeat: repeat + 1, mode,
          seconds: (performance.now() - start) / 1000, ...result };
        runs.push(run);
        console.log(`${mode.padEnd(7)} ${item.id.padEnd(20)} function@1=${run.functionTop1} exact@1=${run.top1} exact@5=${run.top5} ${run.seconds.toFixed(2)}s${run.error ? ` ERROR: ${run.error}` : ''}`);
      }
    }
  }
  let repositoryUnchanged = false;
  try { repositoryUnchanged = JSON.stringify(before) === JSON.stringify(await snapshot(repo, fixture.cases)); }
  catch { /* Preserve the report but mark it invalid if fixtures changed during execution. */ }
  const summary = summarize(runs);
  const report = { metricVersion: 2, startedAt, repo, repository: before, repositoryUnchanged, baseline,
    okoCommit: (await exec('git', ['-C', root, 'rev-parse', 'HEAD'])).stdout.trim(),
    okoStatus: (await exec('git', ['-C', root, 'status', '--porcelain'])).stdout.trim(),
    node: process.version, repeats, fixture, summary, runs };
  const outputDir = resolve(root, 'benchmarks/results', startedAt.replaceAll(':', '-'));
  await mkdir(outputDir, { recursive: true });
  await writeFile(resolve(outputDir, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  const lines = [
    '# Oko benchmark', '', `Repository: ${repo}`, `Commit: ${before.commit}`, '',
    'Five fixed source-verified questions. Function hits overlap the verified function range; exact hits overlap the original implementation range. Same-file matches outside the function do not count.',
    'Errors count as misses. Timing is end-to-end CLI time, including startup, file search and the Jev request; build time is excluded.',
    'Median timing excludes errors. No warm-up; modes alternate order. These are smoke tests, not a general accuracy estimate.', '',
    `Repository snapshot unchanged: ${repositoryUnchanged}.`, '',
    '| Run | Mode | Function first | Function top five | Exact first | Exact top five | Median seconds | Errors |',
    '|---|---|---:|---:|---:|---:|---:|---:|',
    ...(baseline ? baseline.summary.map(s => summaryRow('Baseline (rescored)', s)) : []),
    ...summary.map(s => summaryRow('Current', s)), '',
    ...(baseline ? [`Baseline: ${baseline.path}. Its stored results were rescored with the same two metrics; historical timings were retained.`, ''] : []),
    '| Question | Mode | Function first | Function top five | Exact first | Exact top five | Seconds | Error |',
    '|---|---|---|---|---|---|---:|---|',
    ...runs.map(r => `| ${r.question} | ${r.mode} | ${r.functionTop1} | ${r.functionTop5} | ${r.top1} | ${r.top5} | ${r.seconds.toFixed(2)} | ${r.error ?? ''} |`), '',
  ];
  await writeFile(resolve(outputDir, 'report.md'), lines.join('\n'));
  console.table(summary);
  console.log(`Report: ${resolve(outputDir, 'report.md')}`);
  if (!repositoryUnchanged || runs.some(run => run.error)) process.exitCode = 1;
}

function summaryRow(label, s) {
  return `| ${label} | ${s.mode} | ${s.functionTop1}/${s.runs} | ${s.functionTop5}/${s.runs} | ${s.top1}/${s.runs} | ${s.top5}/${s.runs} | ${s.medianSeconds?.toFixed(2) ?? 'N/A'} | ${s.errors} |`;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
