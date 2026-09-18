// Live smoke benchmark using only synthetic source; no external checkout required.
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseEnv } from 'node:util';
import { root, rustBinary, hash } from './runtime-utils.mjs';
import { execute, summarize } from './compare-codex.mjs';
const repo = await mkdtemp(join(tmpdir(), 'oko-synthetic-deep-'));
try {
  await mkdir(join(repo, 'docs')); await mkdir(join(repo, 'src'));
  const question = 'Where are two optional telemetry values interpolated while preserving available values when the other is missing?';
  for (let i = 0; i < 35; i++) await writeFile(join(repo, 'docs', `${i}.md`), 'UI design notes: two optional telemetry values interpolated while preserving available values when the other is missing. This is a design proposal, not implemented code.\n');
  await writeFile(join(repo, 'src', 'options.rs'), '/// Interpolate between two optional values.\npub fn merge_optional(a: Option<f64>, b: Option<f64>, t: f64) -> Option<f64> {\n    match (a, b) {\n        (Some(x), Some(y)) => Some(x + (y - x) * t),\n        (Some(x), None) => Some(x),\n        (None, Some(y)) => Some(y),\n        (None, None) => None,\n    }\n}\n\nfn unrelated() { }\n');
  let dotenv = {}; try { dotenv = parseEnv(await readFile(join(root, '.env'), 'utf8')); } catch (e) { if (e.code !== 'ENOENT') throw e; }
  const key = process.env.TYPESAFE_API_KEY ?? dotenv.TYPESAFE_API_KEY;
  if (!key?.trim()) throw new Error('TYPESAFE_API_KEY required');
  const env = { ...process.env, TYPESAFE_API_KEY: key };
  const directory = join(root, 'benchmarks/results', `investigation-${new Date().toISOString().replaceAll(':', '-')}`);
  await mkdir(directory, { recursive: true });
  const report = { description: 'Synthetic smoke case only; implementation development case, not held out. Expected implementation lines 2–9 in src/options.rs.', artifactHash: hash(await readFile(rustBinary)), maxSteps: 5, runs: [] };
  console.log(`Report: ${directory}/report.json`);
  for (let repeat = 1; repeat <= 3; repeat++) for (const mode of repeat % 2 ? ['fast', 'deep'] : ['deep', 'fast']) {
    const out = await execute(rustBinary, ['ask', question, '--json', ...(mode === 'deep' ? ['--deep', '--max-steps', '5'] : [])], { cwd: repo, env, timeout: 180000 });
    const run = { mode, repeat, seconds: out.seconds, top1: false, top5: false };
    try {
      if (out.code !== 0 || out.error) throw new Error(out.error ?? `CLI exited ${out.code}`);
      const value = JSON.parse(out.stdout);
      const hit = x => x.path === 'src/options.rs' && x.startLine <= 9 && x.endLine >= 2;
      Object.assign(run, { top1: !!value.results[0] && hit(value.results[0]), top5: value.results.some(hit), investigation: value.investigation,
        locations: value.results.map(({ path, startLine, endLine }) => ({ path, startLine, endLine })) });
    } catch (e) { run.error = e.message; }
    report.runs.push(run); report.summary = summarize(report.runs, ['fast', 'deep']);
    report.completed = report.runs.length === 6 && report.runs.every(r => !r.error);
    report.artifactUnchanged = report.artifactHash === hash(await readFile(rustBinary));
    await writeFile(join(directory, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(run));
    if (run.error) throw new Error('Smoke benchmark stopped on failure');
  }
} finally { await rm(repo, { recursive: true, force: true }); }
