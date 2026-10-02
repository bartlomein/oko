// Managed by oko setup: Oko answers a code question before the agent's first
// turn. `oko setup --client pi --no-prefetch` removes this file.
//
// Pi 1.0's event context cannot execute its MCP tools. Like the OpenCode
// plugin, this owns a separate Oko process; its excerpt memory is therefore
// separate from the native MCP connection. No processes start at load time.
import { spawn } from "node:child_process";

const COMMAND = __OKO_COMMAND__;
const ENV = __OKO_ENV__;
const PATIENCE_MS = 5000;

export default function OkoPrefetch(pi) {
  let server;
  let lastRequest = 0;

  function stop() {
    const current = server;
    if (!current) return;
    server = undefined;
    current.close();
  }

  function start() {
    if (server) return server;
    const child = spawn(COMMAND[0], COMMAND.slice(1), {
      env: { ...process.env, ...ENV },
      stdio: ["pipe", "pipe", "ignore"],
    });
    const waiting = new Map();
    let buffered = "";
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      if (server?.child === child) server = undefined;
      for (const resolve of waiting.values()) resolve(undefined);
      waiting.clear();
      child.stdin.destroy();
      child.kill("SIGKILL");
    };
    child.on("exit", close);
    child.on("error", close);
    child.stdin.on("error", close);
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      buffered += chunk;
      // Oko's bounded context should never approach this size.
      if (buffered.length > 1024 * 1024) return close();
      for (let end = buffered.indexOf("\n"); end >= 0; end = buffered.indexOf("\n")) {
        const line = buffered.slice(0, end);
        buffered = buffered.slice(end + 1);
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          close();
          return;
        }
        const resolve = waiting.get(message.id);
        if (resolve) {
          waiting.delete(message.id);
          resolve(message);
        }
      }
    });
    const write = (message) => {
      if (!closed) child.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
    };
    const request = (method, params) =>
      new Promise((resolve) => {
        if (closed) return resolve(undefined);
        const id = ++lastRequest;
        waiting.set(id, resolve);
        write({ id, method, params });
      });
    const ready = request("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "oko-pi-prefetch", version: "1" },
    }).then((reply) => {
      if (!reply?.result) return false;
      write({ method: "notifications/initialized" });
      return true;
    });
    server = { child, close, ready, request };
    return server;
  }

  // Reset excerpt memory when the transcript changes. In particular, a tree
  // navigation must not stub an answer that only exists on an abandoned branch.
  for (const event of ["session_start", "session_before_switch", "session_before_fork", "session_tree", "session_compact", "session_shutdown"]) {
    pi.on(event, stop);
  }

  pi.on("before_agent_start", async (event, ctx) => {
    if (!event.prompt?.trim() || ctx.signal?.aborted) return;
    let timer;
    const signal = ctx.signal;
    const abort = () => stop();
    try {
      const oko = start();
      signal?.addEventListener("abort", abort, { once: true });
      timer = setTimeout(() => oko.close(), PATIENCE_MS);
      if (!(await oko.ready)) {
        oko.close();
        return;
      }
      const reply = await oko.request("tools/call", {
        name: "search",
        arguments: {
          question: event.prompt,
          prefetch: `pi:${ctx.sessionManager.getSessionId()}`,
        },
      });
      // Shutdown, cancellation, or navigation may have invalidated this result.
      if (server !== oko || signal?.aborted || reply?.result?.isError) return;
      const text = reply?.result?.content?.find((part) => part.type === "text")?.text;
      if (!text) return;
      const content = JSON.parse(text)?.hookSpecificOutput?.additionalContext;
      if (typeof content !== "string" || !content) return;
      return { message: { customType: "oko-prefetch", content, display: false } };
    } catch {
      stop();
      // A failed prefetch adds nothing; Pi can still use Oko's native MCP tool.
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
    }
  });
}
