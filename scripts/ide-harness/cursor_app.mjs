#!/usr/bin/env node
// Asks one question in the Cursor app's agent chat and prints what happened as
// JSON. Cursor is an Electron app: started with --remote-debugging-port, its
// window can be driven over the Chrome DevTools Protocol. This starts Cursor
// that way on the project when it is not already listening, opens a new chat,
// types the prompt, and reads the answer once it stops changing.
//
//   node cursor_app.mjs --cwd PROJECT --prompt TEXT [--port 9333]
//
// Cursor must be signed in. A Cursor already running without the debugging
// port cannot be driven: quit it first. Cursor reopens its last project, so
// the project is opened with Cursor's command (which lists it under Recents)
// and then picked in the repository chip above the chat box. The chat's
// layout is Cursor 3's agent window; a redesign can break these selectors.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { chromium } from 'playwright-core';

const APP = '/Applications/Cursor.app';
const BUSY = /Thinking|Planning next moves|Exploring|Generating|Running/;

function parseArgs(argv) {
  const args = { port: '9333' };
  for (let i = 0; i < argv.length; i += 2) {
    const key = { '--cwd': 'cwd', '--prompt': 'prompt', '--port': 'port' }[argv[i]];
    if (!key || argv[i + 1] === undefined) throw new Error(`Bad argument ${argv[i]}`);
    args[key] = argv[i + 1];
  }
  if (!args.cwd || !args.prompt) throw new Error('--cwd and --prompt are required');
  return args;
}

async function listening(port) {
  try {
    return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok;
  } catch {
    return false;
  }
}

async function cursorWindow(args) {
  if (!(await listening(args.port))) {
    const running = spawn('pgrep', ['-f', `${APP}/Contents/MacOS/Cursor`]);
    if (await new Promise((resolve) => running.on('exit', (code) => resolve(code === 0)))) {
      throw new Error(`Cursor is running without --remote-debugging-port=${args.port}; quit it and run again.`);
    }
    spawn(`${APP}/Contents/MacOS/Cursor`, [`--remote-debugging-port=${args.port}`, args.cwd], { detached: true, stdio: 'ignore' }).unref();
    for (let i = 0; i < 60 && !(await listening(args.port)); i++) await new Promise((r) => setTimeout(r, 500));
  }
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${args.port}`);
  const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().startsWith('vscode-file'));
  if (!page) throw new Error('No Cursor window found');
  return { browser, page };
}

/** The repository the chat box is on: the chip two lines above "This Mac". */
async function chatRepository(page) {
  const lines = (await page.evaluate(() => document.body.innerText)).split('\n').map((line) => line.trim());
  const at = lines.lastIndexOf('This Mac');
  return at >= 2 ? lines[at - 2] : null;
}

async function pickRepository(page, cwd) {
  const name = path.basename(cwd);
  spawnSync(`${APP}/Contents/Resources/app/bin/cursor`, [cwd], { stdio: 'ignore' });
  await page.waitForTimeout(2500);
  // A menu or dialog left open would swallow the clicks below.
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
  if (chosen !== name) throw new Error(`Cursor's chat is on ${chosen}, not ${name}.`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const report = { client: 'cursor-app', answer: '', error: null };
  try {
    report.cursorVersion = JSON.parse(fs.readFileSync(`${APP}/Contents/Resources/app/package.json`, 'utf8')).version;
  } catch {
    report.cursorVersion = null;
  }
  let browser;
  try {
    let page;
    ({ browser, page } = await cursorWindow(args));
    await page.waitForTimeout(1500);
    await pickRepository(page, args.cwd);
    report.repository = await chatRepository(page);
    // A project's hooks load in the background once it is picked.
    await page.waitForTimeout(3000);
    await page.locator('[contenteditable="true"]').filter({ visible: true }).first().click();
    await page.keyboard.type(args.prompt, { delay: 2 });
    await page.keyboard.press('Enter');
    let last = '';
    for (let i = 0; i < 60; i++) {
      await page.waitForTimeout(3000);
      const text = await page.evaluate(() => document.body.innerText);
      const after = text.split(args.prompt).at(-1) ?? '';
      if (text === last && /Just now/.test(after) && !BUSY.test(after)) break;
      last = text;
    }
    // The reply sits between the "Thought" line and the timestamp under it.
    const after = last.split(args.prompt).at(-1) ?? '';
    const reply = after.split('Just now')[0];
    report.answer = reply.replace(/^[\s\S]*?Thought[^\n]*\n(?:[^\n]*\n)?/, '').trim();
  } catch (error) {
    report.error = String(error?.message ?? error).split('\n')[0];
  } finally {
    await browser?.close().catch(() => {});
  }
  process.stdout.write(JSON.stringify(report) + '\n');
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exit(2);
});
