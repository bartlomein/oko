import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import {
  root, runtimes, requireArtifacts, runJson, assertParity, median, hash,
  workspaceSnapshot, implementationSnapshot,
} from './runtime-utils.mjs';

const WARMUPS = 1;
const REPEATS = 5;

async function main() {
  const [repoArg = '../telemetry-studio', ...extra] = process.argv.slice(2);
  if (extra.length) throw new Error('Usage: node scripts/compare-runtimes.mjs [/path/to/telemetry-studio]');
  const repo = resolve(repoArg);
  await requireArtifacts();
  const codeFixture = JSON.parse(await readFile(resolve(root, 'benchmarks/telemetry-studio.json'), 'utf8'));
  const itemFixture = JSON.parse(await readFile(resolve(root, 'benchmarks/support-tickets.json'), 'utf8'));
  for (const item of codeFixture.cases) {
    for (const expected of item.expected) {
      if (hash(await readFile(resolve(repo, expected.path))) !== expected.sha256) {
        throw new Error(`Fixture is stale: ${expected.path}; review expected source locations before comparing.`);
      }
    }
  }
  const startedAt = new Date().toISOString();
  const before = { repository: await workspaceSnapshot(repo), implementation: await implementationSnapshot() };
  const temporary = await mkdtemp(join(tmpdir(), 'oko-runtime-benchmark-'));
  const runs = [];
  const cases = [];
  try {
    const inputPath = join(temporary, 'items.json');
    await writeFile(inputPath, JSON.stringify(itemFixture.items));
    const workloads = [
      ...codeFixture.cases.map(item => ({ ...item, kind: 'code', cwd: repo,
        args: ['ask', item.question, '--no-jev', '--json'] })),
      ...itemFixture.cases.map(item => ({ ...item, kind: 'items', cwd: temporary,
        args: ['rank', '--input', inputPath, item.question, '--no-jev', '--json'] })),
    ];
    for (const [index, item] of workloads.entries()) {
      let expectedOutput;
      for (let repeat = 0; repeat < WARMUPS + REPEATS; repeat++) {
        const order = (index + repeat) % 2 ? [...runtimes].reverse() : runtimes;
        for (const runtime of order) {
          const result = await runJson(runtime, item.args, item.cwd);
          if (expectedOutput === undefined) expectedOutput = result.value;
          else assertParity(result.value, expectedOutput, `${item.kind}/${item.id}, ${runtime}, repeat ${repeat}`);
          if (repeat >= WARMUPS) runs.push({ kind: item.kind, id: item.id, runtime,
            repeat: repeat - WARMUPS + 1, seconds: result.seconds });
        }
      }
      const timings = Object.fromEntries(runtimes.map(runtime => [runtime,
        median(runs.filter(run => run.kind === item.kind && run.id === item.id && run.runtime === runtime).map(run => run.seconds))]));
      cases.push({ kind: item.kind, id: item.id, question: item.question, parity: true,
        medianSeconds: timings, typescriptOverRust: timings.typescript / timings.rust });
      console.log(`${item.kind.padEnd(5)} ${item.id.padEnd(20)} exact parity; TS ${timings.typescript.toFixed(3)}s; Rust ${timings.rust.toFixed(3)}s`);
    }
  } finally { await rm(temporary, { recursive: true, force: true }); }
  const after = { repository: await workspaceSnapshot(repo), implementation: await implementationSnapshot() };
  assertParity(after, before, 'repository and implementation snapshots before/after the run');
  const summary = ['code', 'items'].map(kind => {
    const selected = runs.filter(run => run.kind === kind);
    const medianSeconds = Object.fromEntries(runtimes.map(runtime => [runtime,
      median(selected.filter(run => run.runtime === runtime).map(run => run.seconds))]));
    return { kind, measuredRunsPerRuntime: selected.length / 2, medianSeconds,
      typescriptOverRust: medianSeconds.typescript / medianSeconds.rust };
  });
  const report = { startedAt, node: process.version, platform: process.platform, arch: process.arch,
    warmupsPerQuestionPerRuntime: WARMUPS, repeatsPerQuestionPerRuntime: REPEATS,
    networkRequests: 0, cachesCleared: false, exactJsonParity: true, snapshotsUnchanged: true,
    timing: 'Fresh process through exit, including CLI startup, file reads, search and output; builds, parsing and snapshots excluded.',
    caveats: ['OS caches are not cleared. Source snapshots pre-read eligible files before warm-ups.',
      'Lexical-only and input-order modes do not measure Jev latency or compare semantic accuracy.',
      'Aggregate median pools all measured runs within each workload; code and items stay separate.'],
    repositoryPath: repo, snapshots: before, summary, cases, runs };
  const directory = resolve(root, 'benchmarks/results', `runtimes-${startedAt.replaceAll(':', '-')}`);
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  await writeFile(join(directory, 'report.md'), [
    '# Rust and TypeScript runtime comparison', '',
    `Repository: ${repo}`, `Repository commit: ${before.repository.commit}`, '',
    'Exact JSON parity passed for every warm-up and measured result. Repository Git state, visible eligible source hashes, implementation source and built artifact hashes were unchanged.',
    'No network or paid API requests. All commands use --no-jev. Code uses lexical search; generic records preserve input order.',
    'One warm-up and five measured runs per question per runtime. Order alternates by question and repeat. Each run starts a fresh process; build time is excluded.',
    'OS caches are not cleared; the snapshot pre-reads eligible files. These results describe warm filesystem behavior, not cold disk performance or Jev speed.', '',
    '| Workload | TypeScript median | Rust median | TS / Rust |', '|---|---:|---:|---:|',
    ...summary.map(row => `| ${row.kind} | ${row.medianSeconds.typescript.toFixed(4)}s | ${row.medianSeconds.rust.toFixed(4)}s | ${row.typescriptOverRust.toFixed(2)}x |`), '',
    'Aggregate medians pool the measured samples within each workload.', '',
    '| Question | Workload | TypeScript median | Rust median | TS / Rust |', '|---|---|---:|---:|---:|',
    ...cases.map(row => `| ${row.id} | ${row.kind} | ${row.medianSeconds.typescript.toFixed(4)}s | ${row.medianSeconds.rust.toFixed(4)}s | ${row.typescriptOverRust.toFixed(2)}x |`), '',
    'This measures runtime parity and local speed. It does not establish a change in search accuracy or predict end-to-end Jev speed.', '',
  ].join('\n'));
  console.log(`Report: ${join(directory, 'report.md')}`);
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
