# Oko

**Help your coding agent find code faster and use fewer tokens.**

Oko helps Codex, Claude Code, and OpenCode spend less time searching and fewer
tokens reading irrelevant code. It delivers relevant source snippets through
MCP so your agent can get to the task sooner. Gains vary by task and coding tool.

Oko runs locally and uses [TypeSafe AI’s Jev](https://typesafe.ai/) to rank selected
source snippets. Use it through your coding agent or directly from your terminal.

[Install](#install) · [Connect your coding tool](#connect-your-coding-tool) · [Benchmarks](#benchmarks) · [CLI examples](#use-in-your-terminal) · [Documentation](#documentation)

```sh
oko ask "where is authentication handled?"
```

No Node.js or Rust runtime needed for downloaded binaries. Oko needs a TypeSafe AI
API key for Jev ranking; local keyword search also works without a key.

## Install

Run this on macOS or Linux to install the current prerelease:

```sh
curl -fsSL https://raw.githubusercontent.com/bartlomein/oko/main/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

The installer detects your processor, downloads Oko and its bundled ripgrep,
verifies the checksum, and installs them without administrator access. Add the
`export PATH` line to `~/.zshrc` or `~/.bashrc` to keep `oko` available in new
terminals. Existing installations from other sources are left untouched.

Supported downloads: macOS Apple Silicon and Intel; Linux ARM64 and x64 with
glibc 2.35+ (Ubuntu 22.04 or newer). Alpine and Windows are not supported by the
installer. macOS binaries are not yet signed or notarized.

Prefer to inspect the script, install manually, or select a version?
See [installation options](docs/installation.md).

Continue with `oko setup` below—it prompts for your
[TypeSafe AI](https://typesafe.ai/) key. For terminal use or a manual connection,
save the key with `oko auth login`. Oko uses your operating system’s credential
store; headless Linux can use an environment variable or ignored `.env` instead.
See [key configuration](docs/configuration.md).

## Connect your coding tool

Run setup from the project you want to search, naming your coding tool:

```sh
cd /path/to/your/project
oko setup                     # Codex app or CLI
oko setup --client claude     # Claude Code
oko setup --client opencode   # OpenCode 1.x
oko setup --client all        # all three
```

Setup installs stable copies, connects the tool to Oko for this project, adds
search guidance to the instructions that tool reads, and checks the connection.
The guidance matters: in our benchmark it is what makes Codex and OpenCode
sessions faster, not just cheaper. Start a new session afterwards (in Codex, trust
the project if prompted). Repeat setup for each project.

For a key-free connection, add `--no-jev`. See [what setup changes](docs/mcp.md#set-up-a-project).
The manual steps below do the same by hand, without the guidance.

### Claude Code by hand

From the project you want to search:

```sh
cd /path/to/your/project
oko auth login
claude mcp add --transport stdio --scope local oko -- "$(command -v oko)" mcp --root "$PWD"
claude mcp list
```

Start a new Claude Code session and check `/mcp` for Oko. This connection is
private to you and scoped to this project; repeat the command for other projects.
Skip `oko auth login` if you already saved your key.

### OpenCode by hand

Save your key once with `oko auth login`. For **OpenCode 1.x**, add this to
`opencode.json` in your project root. If the file already exists, merge the `oko`
entry into its existing `mcp` section and preserve your other settings.

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "oko": {
      "type": "local",
      "command": ["oko", "mcp"],
      "enabled": true
    }
  }
}
```

Oko searches the current project. If OpenCode cannot find the executable, replace
`oko` in `command` with the full path printed by `command -v oko`.

<details>
<summary>OpenCode 2.x configuration</summary>

Version 2 places servers under `mcp.servers` and connects them automatically:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "oko": {
        "type": "local",
        "command": ["oko", "mcp"]
      }
    }
  }
}
```

</details>

Run `opencode mcp list` from your project to check the connection, then start a
new OpenCode session. Use `opencode --version` if you are unsure which format to use.
See the [OpenCode MCP docs](https://opencode.ai/docs/mcp-servers/) for more options.

### Try it

Ask your agent:

> Use Oko to find where authentication is handled in this project.

Your coding tool starts Oko automatically. You do not need to run a server yourself.
Connecting Oko makes it available; it does not force the agent to use it for every
search. Exact names and literal text can still be searched with native grep.

## Use in your terminal

Oko searches the directory you run it from:

```sh
cd /path/to/your/project
oko ask "where is authentication handled?"
oko ask "how are retries handled?" --json
oko ask "how does authentication work?" --deep --max-steps 5
oko ask "where is authentication handled?" --no-jev
```

Start with normal search. Deep mode can investigate further, but makes additional
Jev requests and can take longer. `--no-jev` uses local keyword ranking only.

To replace, check, or remove your saved key, use `oko auth login`,
`oko auth status`, or `oko auth logout`.

## Benchmarks

In a **108-session pilot** on Astro, HTTPX, and ripgrep, we compared each coding
tool with and without Oko on the same six search and six small editing tasks.
These are **average seconds and agent tokens per task**, not isolated search timings.

| Coding tool | Without Oko | Warm Oko | Cold Oko |
| --- | --- | --- | --- |
| Codex — time | 27.7 s | 25.7 s (**7.4% less**) | 28.9 s (**4.0% more**) |
| Codex — tokens | 76,022 | 64,336 (**15.4% fewer**) | 61,073 (**19.7% fewer**) |
| OpenCode — time | 25.7 s | 21.1 s (**18.0% less**) | 23.7 s (**8.0% less**) |
| OpenCode — tokens | 35,772 | 19,252 (**46.2% fewer**) | 19,624 (**45.1% fewer**) |
| Claude Code — time | 11.0 s | 9.1 s (**17.1% less**) | 9.1 s (**17.1% less**) |
| Claude Code — tokens | 29,093 | 25,772 (**11.4% fewer**) | 23,274 (**20.0% fewer**) |

Warm means a prebuilt disk index; its preparation is excluded from timing. Cold
starts with an empty Oko cache. Tokens include cached input and exclude Jev,
so these percentages are not dollar savings. One observation per task and
condition; results vary, and cold Oko was slower for Codex in this run.

Automated grading passed 92/108 sessions. Review found correct source evidence
and passing focused edit checks in the flagged cases, with one response-format
violation remaining. See [results, methodology, and grading limitations](docs/benchmark-results.md)
and the [reproduction instructions](scripts/benchmark-public/README.md).

## Privacy

File discovery and initial search run locally. Jev ranking sends your question and
selected source snippets to TypeSafe AI. Deep mode can send more snippets across
multiple requests. **Use `--no-jev` for local-only searches.** Oko does not edit your
source files. Prepared search data is cached outside your repository;
[configuration](docs/configuration.md) explains key storage and cache controls.

## Documentation

| I want to… | Guide |
| --- | --- |
| Build from source, upgrade, or troubleshoot installation | [Installation](docs/installation.md) |
| Connect an agent or understand the MCP tool | [Client setup](docs/clients.md) · [MCP reference](docs/mcp.md) |
| Configure keys, `.env`, or caching | [Configuration](docs/configuration.md) |
| Use search options or rank my own documents and records | [Search](docs/search.md) · [Document ranking](docs/ranking.md) |
| Contribute, evaluate results, or prepare a release | [Development and benchmarks](docs/benchmarks.md) · [Releasing](docs/releasing.md) |

## Help and contributing

Found a bug or have an idea? [Open an issue](https://github.com/bartlomein/oko/issues).
Include your OS, `oko --version`, and steps to reproduce, without API keys or
private source code. For code changes, build from source and run:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The Rust tests need no API key or private repository. Live benchmarks are optional.

## License

[MIT](LICENSE). Dependency attributions are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
