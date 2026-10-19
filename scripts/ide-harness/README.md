# VS Code extension and Cursor harness

Checks that Oko's prompt hook reaches the model inside the Claude Code and
Codex VS Code extensions, without opening VS Code, and inside the Cursor app.
Each extension is a window around a program it starts; the harness starts the
same program the same way and asks it a question. Cursor is driven through its
own window.

```sh
cargo build --release --locked
python3 scripts/ide-harness/run.py            # both clients, prints PASS/FAIL per client
python3 scripts/ide-harness/run.py --clients codex --out results.json
python3 scripts/ide-harness/run.py --clients claude,codex,cursor
```

Exit status 0 means every client passed.

## What it does

1. Builds two copies of a two-file sample project in `~/.cache/oko-ide-harness/`
   and runs `oko setup` from the given build in each: `project` with the prompt
   hook, `control` with `--no-prefetch`. Setup copies Oko into
   `~/.cache/oko-ide-harness/install`, so your installed Oko and its
   `projects.json` are left alone.
2. Asks both projects the same question with read and search tools out of
   reach: which file defines the parcel-dispatch function, or `NO CONTEXT`.
3. **PASS** when the hooked project names `shipping.rs` and the control says
   `NO CONTEXT`. **INCONCLUSIVE** when the control also finds it (the check
   cannot tell the hook's effect apart). **FAIL** when the hook's context
   did not reach the model.

| Client | How it is run | Copied from the extension |
|---|---|---|
| Claude Code | `claude_vscode.mjs`: the extension's bundled `resources/native-binary/claude` through `@anthropic-ai/claude-agent-sdk`, pinned to the SDK version the extension bundles | `settingSources: user, project, local`, `CLAUDE_CODE_ENTRYPOINT=claude-vscode`, preset system prompt, streamed input, permission mode `default`, its extra CLI flags |
| Codex | `codex_vscode.py`: the extension's bundled `codex -c features.code_mode_host=true app-server --analytics-default-enabled`, JSON-RPC over stdio | `CODEX_INTERNAL_ORIGINATOR_OVERRIDE=codex_vscode`, `initialize` as client "VS Code", project and hook trust written as "Trust all and continue" writes them |

| Cursor | `cursor_app.mjs`: the Cursor app started with `--remote-debugging-port=9333` and driven with `playwright-core` over the Chrome DevTools Protocol: open the project with Cursor's command, pick it in the repository chip, start a new chat, type, read the answer | Everything: it is the app. The control is the same project set up again with `--no-prefetch`, since Cursor's window is tied to one project |

Cursor needs to be signed in, and a Cursor already running without the
debugging port must be quit first; the harness starts it. Its project is
`~/.cache/oko-ide-harness/oko-cursor-probe`, kept between runs. Cursor's layout
can change between releases and break the selectors; the error then names the
step. Its project MCP server stays off until turned on under Customize > MCPs;
the check does not need it.

The Claude probe uses the newest `anthropic.claude-code-*` folder under
`~/.vscode/extensions`. The Codex probe uses an installed `openai.chatgpt-*`
extension, or else downloads the latest Marketplace build for this platform
once into `~/.cache/oko-ide-harness/codex-extension/`.

Not copied: the chat window itself, Claude's VS Code system-prompt addition and
its IDE MCP server, which exist only inside the editor. A bug that lives in the
webview would not show here.

## What lands outside the cache

- `~/.claude.json`: `claude mcp add-json --scope local` entries for the two
  harness projects.
- `~/.codex/config.toml`: trust for the two harness projects and their hooks.
- Cursor's own state: the probe project in its Recents and its chats.

The TypeSafe key is copied from this repository's `.env` into each harness
project's `.env`, so a new build does not wait on a macOS Keychain prompt.

Runs use your Claude and ChatGPT logins and your TypeSafe key; one run costs a
few cents. The SDK version in `package.json` should follow the extension's
bundled one (`CLAUDE_AGENT_SDK_VERSION` in its `extension.js`); each report
records both.

## Results, 2026-10-18 (Oko 0.7.3)

| Client | Extension | Prompt hook | Hooked answer | Control |
|---|---|---|---|---|
| Claude Code | 2.1.301 | ran | `src/shipping.rs:7` | NO CONTEXT |
| Codex | 26.51007.21434 (codex 0.162.0-alpha) | ran once the project and hook were trusted | `src/shipping.rs:7` | NO CONTEXT |
| Cursor app | 3.25.9 | ran (one Jev request logged for the project) | `src/shipping.rs, line 7` | NO CONTEXT |

Untrusted, Codex lists no project hooks and the hook does not run: users must
accept "Trust all and continue", as the installation guide says.
