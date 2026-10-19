#!/usr/bin/env node
// Runs one prompt through Claude Code the way its VS Code extension does: the
// extension's own bundled binary, started by the same Agent SDK version with
// the options the extension passes, and prints what happened as JSON.
//
//   node claude_vscode.mjs --cwd PROJECT --prompt TEXT [--extension DIR]
//                          [--setting-sources user,project,local]
//
// The extension's options were read from extension.js (spawnClaude). Two are
// left out: its appended VS Code system prompt and its IDE MCP server, which
// only exist inside the editor. Read and search tools are disallowed so the
// answer shows what was in context before the first turn.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { query } from '@anthropic-ai/claude-agent-sdk';

const BLOCKED_TOOLS = ['Read', 'Grep', 'Glob', 'Bash', 'Task', 'Agent', 'WebFetch', 'WebSearch', 'mcp__oko__search'];

function parseArgs(argv) {
  const args = { settingSources: 'user,project,local' };
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    const key = { '--cwd': 'cwd', '--prompt': 'prompt', '--extension': 'extension', '--setting-sources': 'settingSources' }[flag];
    if (!key) throw new Error(`Unknown argument ${flag}`);
    args[key] = value;
  }
  if (!args.cwd || !args.prompt) throw new Error('--cwd and --prompt are required');
  return args;
}

/** The newest installed Claude Code extension, unless one is named. */
export function findExtension(explicit) {
  if (explicit) return explicit;
  const root = path.join(os.homedir(), '.vscode', 'extensions');
  const found = fs.existsSync(root)
    ? fs.readdirSync(root).filter((name) => name.startsWith('anthropic.claude-code-'))
    : [];
  const version = (name) => name.replace('anthropic.claude-code-', '').split('-')[0].split('.').map(Number);
  found.sort((a, b) => {
    const [x, y] = [version(a), version(b)];
    for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
    return 0;
  });
  if (!found.length) throw new Error(`No Claude Code extension under ${root}`);
  return path.join(root, found.at(-1));
}

/** The environment the extension gives Claude Code, without this shell's own Claude session. */
function extensionEnv() {
  const env = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (key === 'CLAUDECODE' || key.startsWith('CLAUDE_CODE_')) continue;
    env[key] = value;
  }
  env.CLAUDE_CODE_ENTRYPOINT = 'claude-vscode';
  return env;
}

async function* oneMessage(text) {
  yield { type: 'user', message: { role: 'user', content: [{ type: 'text', text }] }, parent_tool_use_id: null, session_id: '' };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const extension = findExtension(args.extension);
  const manifest = JSON.parse(fs.readFileSync(path.join(extension, 'package.json'), 'utf8'));
  const binary = path.join(extension, 'resources', 'native-binary', process.platform === 'win32' ? 'claude.exe' : 'claude');
  const bundledSdk = fs.readFileSync(path.join(extension, 'extension.js'), 'utf8').match(/CLAUDE_AGENT_SDK_VERSION="([^"]+)"/)?.[1];
  const usedSdk = JSON.parse(fs.readFileSync(new URL('./node_modules/@anthropic-ai/claude-agent-sdk/package.json', import.meta.url), 'utf8')).version;
  const report = {
    client: 'claude-code-vscode', extensionVersion: manifest.version, extensionSdk: bundledSdk, harnessSdk: usedSdk,
    settingSources: args.settingSources, hooks: [], toolUses: [], deniedTools: [], answer: '', error: null,
  };
  const stderr = [];
  try {
    const run = query({
      prompt: oneMessage(args.prompt),
      options: {
        cwd: args.cwd,
        pathToClaudeCodeExecutable: binary,
        env: extensionEnv(),
        permissionMode: 'default',
        systemPrompt: { type: 'preset', preset: 'claude_code' },
        settingSources: args.settingSources ? args.settingSources.split(',') : [],
        includePartialMessages: true,
        includeHookEvents: true,
        enableFileCheckpointing: true,
        disallowedTools: BLOCKED_TOOLS,
        maxTurns: 2,
        canUseTool: async (name) => {
          report.deniedTools.push(name);
          return { behavior: 'deny', message: 'Answer from what is already in context.' };
        },
        extraArgs: { 'enable-auth-status': null, 'no-chrome': null, 'replay-user-messages': null },
        stderr: (line) => stderr.push(line),
      },
    });
    for await (const message of run) {
      if (message.type === 'system' && message.subtype === 'init') {
        report.claudeVersion = message.claude_code_version;
        report.mcpServers = message.mcp_servers;
      } else if (message.type === 'system' && String(message.subtype).startsWith('hook_')) {
        report.hooks.push({ event: message.hook_event ?? message.hook_name, subtype: message.subtype, outcome: message.outcome, exitCode: message.exit_code, outputBytes: (message.output ?? message.stdout ?? '').length });
      } else if (message.type === 'assistant') {
        for (const block of message.message.content) {
          if (block.type === 'text') report.answer += block.text;
          if (block.type === 'tool_use') report.toolUses.push(block.name);
        }
      } else if (message.type === 'result') {
        report.result = { subtype: message.subtype, turns: message.num_turns, costUsd: message.total_cost_usd };
      }
    }
  } catch (error) {
    report.error = String(error?.message ?? error);
    report.stderrTail = stderr.join('').split('\n').slice(-20).join('\n');
  }
  process.stdout.write(JSON.stringify(report) + '\n');
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exit(2);
});
