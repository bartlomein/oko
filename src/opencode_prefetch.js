// Managed by oko setup: Oko answers a code question before the agent's first
// turn. `oko setup --client opencode --no-prefetch` removes this file.
//
// OpenCode has no prompt hook in its configuration; `chat.message` runs before
// the first model call and its added parts reach the model. Plugins cannot call
// OpenCode's own MCP connection, so this keeps one Oko process of its own and
// asks it the same way Claude Code's and Codex's hooks do: `search` with the
// prompt and a hidden `prefetch` argument. Oko decides; most prompts get nothing.
import { spawn } from "node:child_process";

const COMMAND = __OKO_COMMAND__;
const ENV = __OKO_ENV__;
// OpenCode has no hook timeout of its own: the prompt waits at most this long.
const PATIENCE_MS = 5000;

let server;
let lastRequest = 0;

function start() {
  if (server) return server;
  const child = spawn(COMMAND[0], COMMAND.slice(1), {
    env: { ...process.env, ...ENV },
    stdio: ["pipe", "pipe", "ignore"],
  });
  const waiting = new Map();
  let buffered = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buffered += chunk;
    for (let end = buffered.indexOf("\n"); end >= 0; end = buffered.indexOf("\n")) {
      const line = buffered.slice(0, end);
      buffered = buffered.slice(end + 1);
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      const resolve = waiting.get(message.id);
      if (resolve) {
        waiting.delete(message.id);
        resolve(message);
      }
    }
  });
  const stop = () => {
    if (server && server.child === child) server = undefined;
    for (const resolve of waiting.values()) resolve(undefined);
    waiting.clear();
  };
  child.on("exit", stop);
  child.on("error", stop);
  child.stdin.on("error", stop);
  const write = (message) => child.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
  const request = (method, params) =>
    new Promise((resolve) => {
      const id = ++lastRequest;
      waiting.set(id, resolve);
      write({ id, method, params });
    });
  const ready = request("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "oko-opencode-prefetch", version: "1" },
  }).then((reply) => {
    if (reply) write({ method: "notifications/initialized" });
    return reply;
  });
  server = { child, ready, request };
  return server;
}

// OpenCode's ascending part id: `prt_`, 12 hex digits of time, 14 base62 characters.
let lastTime = 0;
let counter = 0;
function partId() {
  const now = Date.now();
  if (now !== lastTime) {
    lastTime = now;
    counter = 0;
  }
  counter += 1;
  const time = (BigInt(now) * 4096n + BigInt(counter)) & 0xffffffffffffn;
  const alphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
  let random = "";
  for (let i = 0; i < 14; i++) random += alphabet[Math.floor(Math.random() * alphabet.length)];
  return "prt_" + time.toString(16).padStart(12, "0") + random;
}

async function answer(prompt, sessionID) {
  const oko = start();
  if (!(await oko.ready)) return undefined;
  const reply = await oko.request("tools/call", {
    name: "search",
    arguments: { question: prompt, prefetch: `opencode:${sessionID}` },
  });
  const text = reply?.result?.content?.[0]?.text;
  if (!text) return undefined;
  return JSON.parse(text)?.hookSpecificOutput?.additionalContext;
}

export const OkoPrefetch = async () => ({
  "chat.message": async (input, output) => {
    try {
      const prompt = output.parts
        .filter((part) => part.type === "text" && !part.synthetic)
        .map((part) => part.text)
        .join("\n")
        .trim();
      if (!prompt) return;
      let timer;
      const context = await Promise.race([
        answer(prompt, input.sessionID),
        new Promise((resolve) => {
          timer = setTimeout(resolve, PATIENCE_MS);
        }),
      ]);
      clearTimeout(timer);
      if (!context) return;
      output.parts.push({
        id: partId(),
        sessionID: input.sessionID,
        messageID: output.message.id,
        type: "text",
        text: context,
        synthetic: true,
      });
    } catch {
      // A prefetch that fails adds nothing; the prompt goes through as typed.
    }
  },
});
