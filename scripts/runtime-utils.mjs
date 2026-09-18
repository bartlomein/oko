import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, stat } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const rustBinary = resolve(root, 'target/release/oko');

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
