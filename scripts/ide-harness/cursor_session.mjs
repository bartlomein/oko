#!/usr/bin/env node
// One timed agent session in the Cursor app, for the Cursor benchmark. Cursor
// must already be running with --remote-debugging-port, started on this project
// (cursor_bench.py does both): Cursor loads a project's MCP servers only for
// the folder it starts with. Picks the project in the repository chip, turns its Oko MCP server on
// when asked to, opens a new chat, sends the prompt, and waits for Cursor's own
// transcript (~/.cursor/projects/<project>/agent-transcripts) to record the end
// of the turn. Tool calls and the final answer come from that transcript.
//
//   node cursor_session.mjs --cwd PROJECT --prompt TEXT [--enable-oko] [--timeout 300]
//
// Prints one JSON object. Command approvals Cursor asks for are accepted, so a
// session never waits on a dialog; the benchmark prompt forbids builds and tests.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { chromium } from 'playwright-core';

const APP = '/Applications/Cursor.app';
const PORT = 9333;
const APPROVALS = ['Run', 'Run command', 'Allow', 'Accept', 'Approve'];

function parseArgs(argv) {
  const args = { timeout: 300, enableOko: false };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (flag === '--enable-oko') args.enableOko = true;
    else if (flag === '--cwd') args.cwd = argv[++i];
    else if (flag === '--prompt') args.prompt = argv[++i];
    else if (flag === '--timeout') args.timeout = Number(argv[++i]);
    else throw new Error(`Unknown argument ${flag}`);
  }
  if (!args.cwd || !args.prompt) throw new Error('--cwd and --prompt are required');
  return args;
}

const lines = async (page) => (await page.evaluate(() => document.body.innerText)).split('\n').map((l) => l.trim());

/** The repository the chat box is on: the chip two lines above "This Mac". */
async function chatRepository(page) {
  const all = await lines(page);
  const at = all.lastIndexOf('This Mac');
  return at >= 2 ? all[at - 2] : null;
}

async function pickRepository(page, cwd) {
  const name = path.basename(cwd);
  spawnSync(`${APP}/Contents/Resources/app/bin/cursor`, [cwd], { stdio: 'ignore' });
  await page.waitForTimeout(2500);
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await page.getByText('New Chat', { exact: true }).first().click();
  await page.waitForTimeout(1500);
  const current = await chatRepository(page);
  if (current !== name) {
    await page.getByText(current, { exact: true }).last().click();
    await page.waitForTimeout(1500);
    await page.getByText(name, { exact: true }).last().click();
    await page.waitForTimeout(2500);
  }
  const chosen = await chatRepository(page);
  if (chosen !== name) throw new Error(`Cursor's chat is on ${chosen}, not ${name}`);
}

/** Customize > MCPs: turn the project's oko server on if it is off. The list
 * shows a project's servers only when the project is ticked in the scope
 * filter left of "Plugins" (it reads "<user> +N" with N projects ticked). */
async function enableOko(page, cwd) {
  await page.getByText('Customize', { exact: true }).first().click();
  await page.waitForTimeout(1500);
  await page.getByText('MCPs', { exact: true }).first().click();
  let all = [];
  for (let attempt = 0; attempt < 2 && !all.includes('oko'); attempt++) {
    if (attempt === 1) {
      const filter = all[all.indexOf('Plugins') - 1];
      await page.getByText(filter, { exact: true }).last().click();
      await page.waitForTimeout(1500);
      // Only inside the open menu: the sidebar lists the same project names.
      // The menu shows a few workspaces and "N more".
      const menu = page.getByRole('menu').last();
      const name = menu.getByText(path.basename(cwd), { exact: true });
      if (!(await name.count())) {
        await menu.getByText(/^\d+ more$/).last().click();
        await page.waitForTimeout(1000);
      }
      await name.last().click();
      await page.waitForTimeout(1000);
      await page.keyboard.press('Escape');
    }
    for (let i = 0; i < 8 && !all.includes('oko'); i++) {
      await page.waitForTimeout(1000);
      all = await lines(page);
    }
  }
  const at = all.indexOf('oko');
  const status = at >= 0 ? all.slice(at + 1, at + 3).join(' ') : 'missing';
  if (/Disabled/.test(status)) {
    await page.getByText('oko', { exact: true }).first().click();
    await page.waitForTimeout(1500);
    await page.locator('[role="switch"]').filter({ visible: true }).first().click();
    await page.waitForTimeout(3000);
    await page.getByText('Done', { exact: true }).first().click().catch(() => {});
    await page.waitForTimeout(2000);
  }
  const after = await lines(page);
  const index = after.indexOf('oko');
  return index >= 0 ? after.slice(index + 1, index + 3).join(' ') : 'missing';
}

function transcriptsDir(cwd) {
  const slug = path.resolve(cwd).split(path.sep).filter(Boolean).map((part) => part.replace(/^\./, '')).join('-');
  return path.join(os.homedir(), '.cursor', 'projects', slug, 'agent-transcripts');
}

function transcriptFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir).map((id) => path.join(dir, id, `${id}.jsonl`)).filter((file) => fs.existsSync(file));
}

function readTranscript(file) {
  const result = { tools: [], answer: '', ended: null };
  for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
    if (!line.trim()) continue;
    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }
    if (entry.type === 'turn_ended') result.ended = entry.status ?? 'ended';
    if (entry.role !== 'assistant') continue;
    let text = '';
    for (const block of entry.message?.content ?? []) {
      if (block.type === 'tool_use') result.tools.push(block.name === 'CallDynamicTool' ? `mcp:${block.input?.namespace}:${block.input?.toolName}` : block.name);
      if (block.type === 'text') text += block.text;
    }
    if (text) result.answer = text; // the last assistant text is the final answer
  }
  return result;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const report = { cwd: args.cwd, answer: '', tools: [], seconds: null, error: null };
  let browser;
  let step = 'connect';
  try {
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
    const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().startsWith('vscode-file'));
    if (!page) throw new Error('No Cursor window found');
    step = 'pick repository';
    await pickRepository(page, args.cwd);
    if (args.enableOko) {
      step = 'turn Oko on';
      report.okoServer = await enableOko(page, args.cwd);
      step = 'new chat after Customize';
      await page.getByText('New Chat', { exact: true }).first().click();
      await page.waitForTimeout(1500);
      if ((await chatRepository(page)) !== path.basename(args.cwd)) await pickRepository(page, args.cwd);
    }
    step = 'send prompt';
    const all = await lines(page);
    report.model = all.slice(all.lastIndexOf('This Mac') + 1).find(Boolean) ?? null;
    const dir = transcriptsDir(args.cwd);
    const before = new Set(transcriptFiles(dir));
    await page.locator('[contenteditable="true"]').filter({ visible: true }).first().click();
    await page.keyboard.insertText(args.prompt);
    const started = Date.now();
    await page.keyboard.press('Enter');
    let file;
    while (Date.now() - started < args.timeout * 1000) {
      await page.waitForTimeout(1000);
      file ??= transcriptFiles(dir).find((candidate) => !before.has(candidate));
      if (file && readTranscript(file).ended) break;
      for (const label of APPROVALS) {
        const button = page.getByRole('button', { name: label, exact: true }).filter({ visible: true });
        if (await button.count()) {
          await button.first().click().catch(() => {});
          report.approvals = (report.approvals ?? 0) + 1;
        }
      }
    }
    report.seconds = (Date.now() - started) / 1000;
    if (!file) throw new Error('No transcript appeared for the session');
    const transcript = readTranscript(file);
    Object.assign(report, { answer: transcript.answer, tools: transcript.tools, ended: transcript.ended, transcript: file });
    if (!transcript.ended) report.error = `Timed out after ${args.timeout} s`;
  } catch (error) {
    report.error = `${step}: ${String(error?.message ?? error).split('\n')[0]}`;
  } finally {
    await browser?.close().catch(() => {});
  }
  process.stdout.write(JSON.stringify(report) + '\n');
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exit(2);
});
