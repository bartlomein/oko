import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { access, readFile, readdir, stat } from 'node:fs/promises';
import { constants } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const reference = resolve(root, 'reference/typescript');
export const rustBinary = resolve(root, 'target/release/oko');
export const parityBinary = resolve(root, 'target/release/oko-parity');
export const runtimes = ['typescript', 'rust'];

export function runProcess(command, args, { cwd = root, input, timeout = 60_000 } = {}) {
  return new Promise((resolveResult, reject) => {
    const started = performance.now();
    const child = spawn(command, args, { cwd, stdio: ['pipe', 'pipe', 'pipe'] });
    const stdout = [];
    const stderr = [];
    let size = 0;
    let failure;
    const timer = setTimeout(() => { failure = new Error(`${command} timed out`); child.kill(); }, timeout);
    const collect = destination => data => {
      size += data.length;
      if (size > 64 * 1024 * 1024) {
        failure = new Error(`${command} output exceeded 64 MiB`);
        child.kill();
      } else destination.push(data);
    };
    child.stdout.on('data', collect(stdout));
    child.stderr.on('data', collect(stderr));
    child.stdin.on('error', error => { if (error.code !== 'EPIPE') failure = error; });
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', code => {
      clearTimeout(timer);
      if (failure) return reject(failure);
      resolveResult({ code, stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'), seconds: (performance.now() - started) / 1000 });
    });
    child.stdin.end(input);
  });
}

export async function requireArtifacts({ diagnostic = false } = {}) {
  for (const path of [resolve(reference, 'dist/cli.js'), rustBinary, ...(diagnostic ? [parityBinary] : [])]) {
    try { await access(path, path.endsWith('.js') ? constants.R_OK : constants.X_OK); }
    catch { throw new Error(`Missing built artifact: ${path}. Run cargo build --release and npm --prefix reference/typescript run build first.`); }
  }
  const result = await runProcess('rg', ['--version']);
  if (result.code !== 0) throw new Error('ripgrep is required for runtime comparisons.');
}

export async function runCli(runtime, args, cwd) {
  return runtime === 'typescript'
    ? runProcess(process.execPath, [resolve(reference, 'dist/cli.js'), ...args], { cwd })
    : runProcess(rustBinary, args, { cwd });
}

export async function runJson(runtime, args, cwd) {
  const result = await runCli(runtime, args, cwd);
  if (result.code !== 0) throw new Error(`${runtime} failed (exit ${result.code}): ${result.stderr.trim()}`);
  try { return { ...result, value: JSON.parse(result.stdout) }; }
  catch { throw new Error(`${runtime} returned invalid JSON`); }
}

export function assertParity(left, right, label) {
  try { assert.deepStrictEqual(left, right); }
  catch (error) { throw new Error(`Runtime parity failed: ${label}\n${error.message}`); }
}

export function median(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

export function hash(bytes) { return createHash('sha256').update(bytes).digest('hex'); }

export async function gitSnapshot(directory) {
  const run = async args => {
    const result = await runProcess('git', ['-C', directory, ...args]);
    if (result.code !== 0) throw new Error(`Cannot snapshot Git repository ${directory}: ${result.stderr.trim()}`);
    return result.stdout.trim();
  };
  return { commit: await run(['rev-parse', 'HEAD']), status: await run(['status', '--porcelain']) };
}

// Snapshot every rg-visible, size-eligible file, including bytes later rejected as
// binary or invalid UTF-8. Hashing is outside measured runs and never saved as text.
export async function workspaceSnapshot(directory) {
  const git = await gitSnapshot(directory);
  const listing = await runProcess('rg', ['--files', '-0'], { cwd: directory });
  if (listing.code !== 0 && listing.code !== 1) throw new Error(`Cannot list ${directory}: ${listing.stderr.trim()}`);
  const files = listing.stdout.split('\0').filter(Boolean).sort();
  const sourceHashes = {};
  for (const name of files) {
    const absolute = resolve(directory, name);
    const info = await stat(absolute);
    if (info.isFile() && info.size <= 256 * 1024) sourceHashes[name] = hash(await readFile(absolute));
  }
  return { ...git, visibleFileCount: files.length, sourceHashes };
}

export async function implementationSnapshot({ diagnostic = false } = {}) {
  const sourceHashes = {};
  const walk = async directory => {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await walk(path);
      else if (entry.isFile()) sourceHashes[path.slice(root.length + 1)] = hash(await readFile(path));
    }
  };
  for (const directory of [resolve(root, 'src'), resolve(reference, 'src'), resolve(reference, 'dist')]) await walk(directory);
  for (const path of [resolve(root, 'Cargo.toml'), resolve(root, 'Cargo.lock'), resolve(reference, 'package.json'),
    resolve(reference, 'package-lock.json'), rustBinary, ...(diagnostic ? [parityBinary] : [])]) {
    sourceHashes[path.slice(root.length + 1)] = hash(await readFile(path));
  }
  return { ...await gitSnapshot(root), sourceHashes };
}
