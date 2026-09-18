import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir, access, realpath } from 'node:fs/promises';
import { constants } from 'node:fs';
import { resolve, isAbsolute } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseEnv } from 'node:util';
import { root, rustBinary, hash, median, workspaceSnapshot, runProcess } from './runtime-utils.mjs';

export function grade(results, expected) {
  const hit = (result, functions) => expected.some(target => {
    const range = functions ? target.function : target;
    return range && result.path === target.path && result.startLine <= range.endLine && result.endLine >= range.startLine;
  });
  return { top1: !!results[0] && hit(results[0], false), top5: results.some(r => hit(r, false)),
    functionTop1: !!results[0] && hit(results[0], true), functionTop5: results.some(r => hit(r, true)) };
}
export function validateResults(value) {
  if (!Array.isArray(value.results) || value.results.length > 5) throw new Error('Invalid results array');
  return value.results.map(({ path, startLine, endLine }) => {
    if (typeof path !== 'string' || !path || isAbsolute(path) || path.split(/[\\/]/).includes('..') ||
        !Number.isInteger(startLine) || !Number.isInteger(endLine) || startLine < 1 || endLine < startLine || endLine - startLine >= 120) {
      throw new Error('Invalid result path or line range');
    }
    return { path, startLine, endLine };
  });
}
export function summarize(runs, modes = ['oko', 'codex']) {
  return modes.map(mode => {
    const rows = runs.filter(r => r.mode === mode);
    return { mode, runs: rows.length, errors: rows.filter(r => r.error).length,
      ...Object.fromEntries(['top1', 'top5', 'functionTop1', 'functionTop5'].map(k => [k, rows.filter(r => !r.error && r[k]).length])),
      medianSeconds: median(rows.filter(r => !r.error).map(r => r.seconds)) };
  });
}
// Kill the process group on timeout so a stalled agent does not leave shell tools running.
export function execute(command, args, { cwd, env, timeout = 180_000 }) {
  return new Promise(resolveResult => {
    const start = performance.now();
    const child = spawn(command, args, { cwd, env, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', bytes = 0, error, finished = false;
    const stop = () => {
      try { if (process.platform !== 'win32') process.kill(-child.pid, 'SIGKILL'); else child.kill('SIGKILL'); } catch { /* already exited */ }
    };
    const interrupted = () => { error = 'Interrupted'; stop(); };
    process.once('SIGINT', interrupted); process.once('SIGTERM', interrupted);
    const timer = setTimeout(() => { error = 'Timed out'; stop(); }, timeout);
    const finish = code => {
      if (finished) return;
      finished = true; clearTimeout(timer);
      process.removeListener('SIGINT', interrupted); process.removeListener('SIGTERM', interrupted);
      resolveResult({ code, stdout, seconds: (performance.now() - start) / 1000, error });
    };
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      bytes += chunk.length;
      if (bytes > 32 * 1024 * 1024) { error = 'Output limit exceeded'; stop(); }
      else stdout += chunk.toString();
    });
    // Provider errors can contain request bodies; do not persist raw stderr.
    child.stderr.resume();
    child.on('error', () => { error = 'Could not start command'; finish(null); });
    child.on('close', finish);
  });
}
export function codexOutput(stdout) {
  const events = stdout.split('\n').filter(Boolean).map(line => JSON.parse(line));
  if (events.some(e => e.type === 'turn.failed' || e.type === 'error')) throw new Error('Codex reported a failed turn');
  const complete = events.findLast(e => e.type === 'turn.completed');
  if (!complete) throw new Error('Codex did not complete');
  const messages = events.filter(e => e.type === 'item.completed' && e.item?.type === 'agent_message');
  const last = messages.at(-1)?.item.text;
  const commands = events.filter(e => e.type === 'item.completed' && e.item?.type === 'command_execution');
  return { results: validateResults(JSON.parse(last)), usage: complete.usage ?? null,
    commandCount: commands.length, commands: commands.map(e => e.item.command),
    toolTypes: [...new Set(events.filter(e => e.type === 'item.completed').map(e => e.item?.type))] };
}
export function validateFixture(fixture) {
  if (!Array.isArray(fixture.cases) || !fixture.cases.length) throw new Error('Fixture requires cases');
  const ids = new Set();
  for (const item of fixture.cases) {
    if (typeof item.id !== 'string' || !item.id || ids.has(item.id) || typeof item.question !== 'string' || !item.question.trim()) throw new Error('Invalid or duplicate fixture case');
    ids.add(item.id);
    if (!Array.isArray(item.expected) || !item.expected.length) throw new Error('Fixture requires expected locations');
    for (const target of item.expected) {
      validateResults({ results: [target] });
      if (!/^[a-f0-9]{64}$/.test(target.sha256)) throw new Error('Fixture requires source hashes');
      if (target.function && (!Number.isInteger(target.function.startLine) || !Number.isInteger(target.function.endLine) || target.function.startLine < 1 || target.function.startLine > target.startLine || target.function.endLine < target.endLine)) throw new Error('Invalid function range');
    }
  }
}
export function rotateModes(modes, offset) {
  const start = offset % modes.length;
  return [...modes.slice(start), ...modes.slice(0, start)];
}
export function openCodeConfig() {
  return { share: 'disabled', agent: { oko_benchmark: {
    description: 'Read-only code location benchmark', mode: 'primary',
    permission: { '*': 'deny', read: { '*': 'allow', '*.env': 'deny', '*.env.*': 'deny' }, glob: 'allow', grep: 'allow' }
  } } };
}
export function openCodeOutput(stdout) {
  const events = stdout.split('\n').filter(Boolean).map(line => JSON.parse(line));
  if (events.some(e => e.type === 'error')) throw new Error('OpenCode reported an error');
  const complete = events.findLast(e => e.type === 'step_finish');
  if (complete?.part?.reason !== 'stop') throw new Error('OpenCode did not complete');
  const finalText = events.filter(e => e.type === 'text' && e.part?.messageID === complete.part.messageID).map(e => e.part.text).join('');
  const calls = events.filter(e => e.type === 'tool_use');

  return { results: validateResults(JSON.parse(finalText)),
    usage: events.filter(e => e.type === 'step_finish').map(e => e.part.tokens),
    commandCount: calls.length, toolErrors: calls.filter(e => e.part?.state?.status === 'error').length, toolTypes: [...new Set(calls.map(e => e.part?.tool))],
    sessionID: complete.sessionID };
}
export async function main(defaults = {}) {
  const [repoArg, ...args] = process.argv.slice(2);
  if (!repoArg || args.length % 2) throw new Error('Usage: node scripts/compare-codex.mjs REPO [--repeats 3] [--model gpt-5.5] [--effort medium] [--codex-bin PATH] [--opencode-bin PATH] [--tools oko,codex,opencode] [--fixture PATH]');
  const options = { repeats: 3, model: 'gpt-5.5', effort: 'medium', 'codex-bin': 'codex',
    'opencode-bin': 'opencode', tools: 'oko,codex', fixture: 'benchmarks/telemetry-studio.json', ...defaults };
  for (let i = 0; i < args.length; i += 2) {
    if (!['--repeats', '--model', '--effort', '--codex-bin', '--opencode-bin', '--tools', '--fixture'].includes(args[i])) throw new Error(`Unknown option ${args[i]}`);
    options[args[i].slice(2)] = args[i] === '--repeats' ? Number(args[i + 1]) : args[i + 1];
  }
  if (!Number.isInteger(options.repeats) || options.repeats < 1 || options.repeats > 10) throw new Error('Repeats must be 1–10');
  const modes = options.tools.split(',');
  if (!modes.length || new Set(modes).size !== modes.length || modes.some(m => !['oko', 'codex', 'opencode'].includes(m))) throw new Error('Tools must be a unique list of oko,codex,opencode');
  const repo = await realpath(resolve(repoArg));
  const fixturePath = resolve(options.fixture);
  const schemaPath = resolve(root, 'benchmarks/codex-output.schema.json');
  const fixtureBytes = await readFile(fixturePath);
  const fixture = JSON.parse(fixtureBytes);
  validateFixture(fixture);
  for (const item of fixture.cases) for (const target of item.expected) {
    const file = await realpath(resolve(repo, target.path));
    if (!file.startsWith(repo + '/')) throw new Error('Fixture path escapes repository');
    if (hash(await readFile(file)) !== target.sha256) throw new Error(`Stale fixture: ${target.path}`);
  }
  if (modes.includes('oko')) await access(rustBinary, constants.X_OK);
  const versions = {};
  for (const mode of modes.filter(m => m !== 'oko')) {
    const version = await runProcess(options[`${mode}-bin`], ['--version']);
    if (version.code !== 0) throw new Error(`${mode} CLI is unavailable`);
    versions[mode] = version.stdout.trim();
  }
  let fileEnv = {};
  try { fileEnv = parseEnv(await readFile(resolve(root, '.env'), 'utf8')); }
  catch (e) { if (e.code !== 'ENOENT') throw e; }
  const key = process.env.TYPESAFE_API_KEY ?? fileEnv.TYPESAFE_API_KEY;
  if (modes.includes('oko') && !key?.trim()) throw new Error('TYPESAFE_API_KEY required in the shell or Oko .env');
  const okoEnv = { ...process.env, TYPESAFE_API_KEY: key };
  const codexEnv = { ...process.env };
  delete codexEnv.TYPESAFE_API_KEY;
  const opencodeEnv = { ...codexEnv, OPENCODE_CONFIG_CONTENT: JSON.stringify(openCodeConfig()) };
  const startedAt = new Date().toISOString();
  const directory = resolve(root, 'benchmarks/results', `comparison-${startedAt.replaceAll(':', '-')}`);
  await mkdir(directory, { recursive: true });
  const before = await workspaceSnapshot(repo);
  const artifactHash = modes.includes('oko') ? hash(await readFile(rustBinary)) : null;
  const schemaHash = hash(await readFile(schemaPath));
  const prompt = question => `Find the implementation in this repository that answers this question:\n${question}\n\nThis is read-only code location, not implementation work. Search and inspect the repository using your usual local tools. Do not use Oko, web search, external repositories, saved memories, or benchmark answers. Return up to five distinct locations in descending relevance, with repository-relative paths and inclusive 1-based line numbers. Choose the smallest relevant implementation ranges, at most 120 lines each. Return an empty results array if there is no match. Do not edit files or run tests/builds. Return only a JSON object matching this schema, without Markdown fences: {"results":[{"path":"relative/file","startLine":1,"endLine":2}]} (results may be empty).`;
  const runs = [];
  const report = { startedAt, repo, options, versions, node: process.version,
    platform: process.platform, arch: process.arch, fixtureHash: hash(fixtureBytes), artifactHash, schemaHash,
    repository: before, repositoryUnchanged: null, promptTemplate: prompt('<QUESTION>'),
    timing: 'Fresh process to exit; model reasoning, local tools, and network included. No warmup; rotating tool order; caches not cleared.',
    configuration: 'Codex user config ignored; model/effort pinned; ephemeral read-only sessions; repository instructions still apply. CLI saved authentication reused. OpenCode uses a fresh session, no external plugins, and a read-only agent with read/glob/grep tools; bash, edits, web and delegation denied. Same model and reasoning effort in both CLIs.',
    caveats: [fixture.description,
      'This compares complete code-location workflows, not underlying search-engine speed.',
      'Errors count as misses; median time excludes errors. No automatic retries.',
      'Provider caching and different system prompts/tools still affect results. Token counters differ by CLI; no cost comparison.',
      'OpenCode permissions are tool restrictions, not an OS sandbox. The target snapshot is verified after the run.'], runs };
  async function save() {
    report.summary = summarize(runs, modes);
    report.groups = Object.fromEntries([...new Set(fixture.cases.map(c => c.group ?? 'unspecified'))].map(group => [group, summarize(runs.filter(r => r.group === group), modes)]));
    await writeFile(resolve(directory, 'report.json'), JSON.stringify(report, null, 2) + '\n');
    await writeFile(resolve(directory, 'report.md'), [
      '# Oko vs Codex CLI vs OpenCode', '', `Agent model: ${options.model}, reasoning: ${options.effort}. Versions: ${JSON.stringify(versions)}.`,
      `${options.repeats} repeats per question. ${report.timing}`, ...report.caveats, '',
      `Repository unchanged: ${report.repositoryUnchanged ?? 'run in progress'}. Completed: ${report.completed ?? false}.`, '',
      '| Tool | Exact first | Exact top five | Function first | Function top five | Median seconds | Errors |',
      '|---|---:|---:|---:|---:|---:|---:|',
      ...report.summary.map(s => `| ${s.mode} | ${s.top1}/${s.runs} | ${s.top5}/${s.runs} | ${s.functionTop1}/${s.runs} | ${s.functionTop5}/${s.runs} | ${s.medianSeconds?.toFixed(3) ?? 'N/A'} | ${s.errors} |`), '',
      ...Object.entries(report.groups).flatMap(([group, rows]) => [
        `## ${group}`, '', '| Tool | Exact first | Exact top five | Median seconds | Errors |', '|---|---:|---:|---:|---:|',
        ...rows.map(s => `| ${s.mode} | ${s.top1}/${s.runs} | ${s.top5}/${s.runs} | ${s.medianSeconds?.toFixed(3) ?? 'N/A'} | ${s.errors} |`), ''
      ]),
      '| Question | Repeat | Tool | First | Top five | Seconds | Error |', '|---|---:|---|---|---|---:|---|',
      ...runs.map(r => `| ${r.id} | ${r.repeat} | ${r.mode} | ${r.top1} | ${r.top5} | ${r.seconds.toFixed(3)} | ${r.error ?? ''} |`), '',
    ].join('\n'));
  }
  await save();
  console.log(`Report: ${directory}/report.md\nRunning ${fixture.cases.length * options.repeats} trials per tool.`);
  trials: for (let repeat = 1; repeat <= options.repeats; repeat++) {
    for (const [index, item] of fixture.cases.entries()) {
      for (const mode of rotateModes(modes, index + repeat - 1)) {
        const commandArgs = mode === 'oko' ? ['ask', item.question, '--json'] : mode === 'opencode' ?
          ['run', '--pure', '--model', `openai/${options.model}`, '--variant', options.effort, '--agent', 'oko_benchmark', '--format', 'json', '--dir', repo, prompt(item.question)] : ['exec', '--ignore-user-config',
          '--ephemeral', '--sandbox', 'read-only', '--model', options.model,
          '-c', `model_reasoning_effort="${options.effort}"`, '--output-schema', schemaPath,
          '--json', '--cd', repo, prompt(item.question)];
        const output = await execute(mode === 'oko' ? rustBinary : options[`${mode}-bin`], commandArgs,
          { cwd: repo, env: mode === 'oko' ? okoEnv : mode === 'opencode' ? opencodeEnv : codexEnv });
        const run = { mode, id: item.id, group: item.group ?? 'unspecified', repeat, seconds: output.seconds, results: [], ...grade([], item.expected) };
        try {
          if (output.error || output.code !== 0) throw new Error(output.error ?? `CLI exited ${output.code}`);
          if (mode === 'codex') Object.assign(run, codexOutput(output.stdout));
          else if (mode === 'opencode') Object.assign(run, openCodeOutput(output.stdout));
          else {
            const value = JSON.parse(output.stdout);
            if (value.ranking !== 'jev') throw new Error('Oko did not use Jev');
            run.results = validateResults(value);
          }
          for (const result of run.results) {
            const file = await realpath(resolve(repo, result.path));
            if (!file.startsWith(repo + '/')) throw new Error('Returned path escapes repository');
            const text = await readFile(file, 'utf8');
            if (result.endLine > text.split(/\r\n|\n|\r/).length) throw new Error('Returned range exceeds file length');
          }
          Object.assign(run, grade(run.results, item.expected));
        } catch (error) { run.error = error.message; }
        runs.push(run);
        await save();
        console.log(`${mode.padEnd(5)} ${item.id.padEnd(20)} ${repeat}/${options.repeats} first=${run.top1} top5=${run.top5} ${run.seconds.toFixed(2)}s${run.error ? ` ERROR: ${run.error}` : ''}`);
        if (run.error) break trials;
      }
    }
  }
  report.completed = runs.length === fixture.cases.length * options.repeats * modes.length && !runs.some(r => r.error);
  report.repositoryUnchanged = JSON.stringify(before) === JSON.stringify(await workspaceSnapshot(repo));
  report.artifactsUnchanged = (!modes.includes('oko') || artifactHash === hash(await readFile(rustBinary))) && schemaHash === hash(await readFile(schemaPath)) && report.fixtureHash === hash(await readFile(fixturePath));
  await save();
  if (!report.repositoryUnchanged || !report.artifactsUnchanged || runs.some(r => r.error)) process.exitCode = 1;
}
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
