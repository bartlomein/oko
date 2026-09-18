import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { config } from 'dotenv';
import { rankItems } from '../dist/index.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const env = { ...process.env };
const loaded = config({ path: resolve(root, '.env'), processEnv: env, quiet: true });
if (loaded.error && loaded.error.code !== 'ENOENT') throw loaded.error;
if (!env.TYPESAFE_API_KEY?.trim()) throw new Error('Add TYPESAFE_API_KEY to Oko/.env.');
const fixture = JSON.parse(await readFile(resolve(root, 'benchmarks/support-tickets.json'), 'utf8'));
const runs = [];
for (const [index, item] of fixture.cases.entries()) {
  for (const mode of index % 2 ? ['jev', 'input'] : ['input', 'jev']) {
    const start = performance.now();
    try {
      const ranked = await rankItems(item.question, fixture.items, { apiKey: env.TYPESAFE_API_KEY, noJev: mode === 'input' });
      const ids = ranked.results.map(result => result.id);
      const top1 = item.expectedIds.length ? item.expectedIds.includes(ids[0]) : ids.length === 0;
      const top5 = item.expectedIds.length ? ids.some(id => item.expectedIds.includes(id)) : ids.length === 0;
      runs.push({ id: item.id, mode, top1, top5, seconds: (performance.now() - start) / 1000, ids, omittedCount: ranked.omittedCount });
    } catch {
      runs.push({ id: item.id, mode, top1: false, top5: false, seconds: (performance.now() - start) / 1000, error: 'Ranking request failed' });
    }
    console.log(runs.at(-1));
  }
}
const summary = ['input', 'jev'].map(mode => {
  const selected = runs.filter(run => run.mode === mode);
  const seconds = selected.filter(run => !run.error).map(run => run.seconds).sort((a, b) => a - b);
  const middle = Math.floor(seconds.length / 2);
  const medianSeconds = seconds.length ? (seconds.length % 2 ? seconds[middle] : (seconds[middle - 1] + seconds[middle]) / 2) : null;
  return { mode, top1: selected.filter(r => r.top1).length, top5: selected.filter(r => r.top5).length,
    count: selected.length, errors: selected.filter(r => r.error).length, medianSeconds };
});
const startedAt = new Date().toISOString();
const directory = resolve(root, 'benchmarks/results', `items-${startedAt.replaceAll(':', '-')}`);
await mkdir(directory, { recursive: true });
await writeFile(resolve(directory, 'report.json'), JSON.stringify({ startedAt, fixture, summary, runs }, null, 2) + '\n');
await writeFile(resolve(directory, 'report.md'), [
  '# Synthetic support-ticket benchmark', '',
  'Five questions, one run each. Input baseline keeps the supplied order; it does not perform search. No-match questions count as correct only when no items are returned.',
  'Times measure the ranking API only, excluding file loading and CLI startup. This is a synthetic smoke test, not evidence of general search accuracy.', '',
  '| Mode | Correct first | Correct top five | Errors | Median seconds |', '|---|---:|---:|---:|---:|',
  ...summary.map(s => `| ${s.mode} | ${s.top1}/${s.count} | ${s.top5}/${s.count} | ${s.errors} | ${s.medianSeconds?.toFixed(3) ?? 'N/A'} |`), '',
].join('\n'));
console.table(summary);
console.log(`Report: ${resolve(directory, 'report.md')}`);
if (runs.some(run => run.error)) process.exitCode = 1;
