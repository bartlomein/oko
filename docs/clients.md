# Connect your coding tool

[← Back to Oko](../README.md)

[Install Oko](../README.md#install) first and run `oko auth login`.
Use absolute paths for both the executable and project so the connection also
works when a GUI app has a different PATH. Examples below use placeholders;
replace them with your actual paths. `command -v oko` shows your installed binary.

Oko uses local stdio MCP: the coding tool starts the process. There is no separate
HTTP server to deploy. Oko exposes one tool, `search`.

## Codex

```sh
cd /path/to/your/project
oko setup
```

This configures Codex for the current project and checks MCP tool discovery.
Open the project in Codex, trust it if prompted, and start a new session.
See [setup details](mcp.md#set-up-codex-for-a-project) for changed files, backups,
credential handling, and optional flags.

## Claude Code

From the project directory, add a connection scoped to this project and user:

```sh
cd /path/to/your/project
claude mcp add --transport stdio --scope local oko -- /absolute/path/to/oko mcp --root "$PWD"
claude mcp list
```

Quote the executable path if it contains spaces. Start a new Claude Code session
and check `/mcp` for Oko. The `local` scope keeps this machine’s paths out of your
shared project configuration. See [Claude Code’s MCP documentation](https://code.claude.com/docs/en/mcp).

## OpenCode

Merge this entry into your project’s `opencode.json` (preserve existing settings):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "oko": {
      "type": "local",
      "command": ["/absolute/path/to/oko", "mcp", "--root", "/absolute/path/to/project"],
      "enabled": true
    }
  }
}
```

Start a new OpenCode session and check the connection with `opencode mcp list`.
Keep machine-specific configuration out of shared commits.
This example follows the [OpenCode MCP guide](https://opencode.ai/docs/mcp-servers/).
If you use OpenCode v2, its [v2 configuration](https://opencode.ai/v2/docs/mcp-servers)
places entries under `mcp.servers`; omit `enabled` there (connections start by default).

## Verify a real search

Ask your agent:

> Use Oko to find where authentication is handled in this project.

Check that it calls Oko’s `search` tool. A connected status confirms tool discovery;
it does not validate your TypeSafe key or guarantee the agent will choose Oko.

For ongoing guidance, add the contents of [`src/guidance.md`](../src/guidance.md)
to your project’s agent instructions (`AGENTS.md` for OpenCode, `CLAUDE.md` for
Claude Code). Codex setup already adds it. It says when to use Oko, that excerpts
are exact file contents, and when to stop searching, with good and bad examples;
project instructions are where agents look for this, not tool descriptions.

For local-only operation, append `--no-jev` to the server arguments. This needs no
TypeSafe key and disables deep mode. For server environments without a credential
store, put the key in the configured project root’s ignored `.env` or provide it
in the process environment; do not embed it in shared MCP configuration.
