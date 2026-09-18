import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { root, rustBinary, runProcess, hash } from './runtime-utils.mjs';

async function main() {
  const bytes = await readFile(resolve(root, 'benchmarks/ranking-intents.json'));
  const fixture = JSON.parse(bytes);
  const startedAt = new Date().toISOString();
  const directory = resolve(root, 'benchmarks/results', `intents-${startedAt.replaceAll(':', '-')}`);
  await mkdir(directory, { recursive: true });
  const runs = [];
  const binaryHash = hash(await readFile(rustBinary));
  for (const item of fixture.cases) {
    const input = resolve(directory, `${item.id}.json`);
    await writeFile(input, JSON.stringify(item.items));
    for (const [intent, expectedId] of Object.entries(item.expected)) {
      const result = await runProcess(rustBinary, ['rank', item.question, '--input', input, '--intent', intent, '--json']);
      const run = { id: item.id, intent, expectedId, seconds: result.seconds, correctFirst: false };
      if (result.code !== 0) run.error = `CLI exited ${result.code}`;
      else {
        const output = JSON.parse(result.stdout);
        if (output.ranking !== 'jev') throw new Error('Expected Jev ranking');
        run.results = output.results.map(({ id, score }) => ({ id, score }));
        run.correctFirst = run.results[0]?.id === expectedId;
      }
      runs.push(run);
      console.log(`${item.id} ${intent}: first=${run.correctFirst} ${run.seconds.toFixed(3)}s`);
    }
  }
  const unchanged = binaryHash === hash(await readFile(rustBinary));
  await writeFile(resolve(directory, 'report.json'), JSON.stringify({ startedAt, fixtureHash: hash(bytes), binaryHash,
    unchanged, runs }, null, 2) + '\n');
  console.log(`Report: ${directory}/report.json`);
  if (!unchanged || runs.some(r => !r.correctFirst || r.error)) process.exitCode = 1;
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
