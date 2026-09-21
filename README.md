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

We give the same coding task to the same agent twice: once with only its built-in
search tools, and once with Oko connected and set up with `oko setup`. Then we
compare how long the whole session took, how many tokens the agent used, and how
many tool calls (searches, file reads, edits) it made.

The tasks come from three open-source projects, each pinned to a fixed commit so
every session sees the same code: [Astro](https://github.com/withastro/astro)
(TypeScript), [HTTPX](https://github.com/encode/httpx) (Python), and
[ripgrep](https://github.com/BurntSushi/ripgrep) (Rust). Each project has three
tasks, nine in all:

- **6 search tasks**, where the agent has to find code spread over several places
  and report the exact locations. For example, in HTTPX: *"Trace how response
  Content-Encoding values select decoders, how multiple decoders are combined, and
  why decoding runs in reverse application order."* The answer is checked against
  the known locations.
- **3 small edit tasks**, where the agent has to find the right function and
  change it. For example, in ripgrep: *"Extend replacement capture-name parsing to
  accept ASCII hyphens in named references, both $first-name and ${first-name}."*
  The edit is checked by running tests against the changed file.

The task wording never names the file or function, which is the situation Oko is
built for. Every task runs 3 times per setup with Codex, OpenCode, and Claude
Code. The latest run is **243 sessions**; it also measured a third setup, Oko
without its guidance, which is in the [full results](docs/benchmark-results.md).

| Coding tool | | Without Oko | With Oko | Average | Best task |
| --- | --- | --- | --- | --- | --- |
| Codex | Time | 25.7 s | 21.0 s | **18% less** | 54% less |
| | Tokens | 71,923 | 48,823 | **32% fewer** | 69% fewer |
| | Tool calls | 3.4 | 2.0 | **40% fewer** | 79% fewer |
| OpenCode | Time | 22.8 s | 19.3 s | **15% less** | 53% less |
| | Tokens | 29,413 | 17,017 | **42% fewer** | 80% fewer |
| | Tool calls | 5.8 | 2.6 | **56% fewer** | 87% fewer |
| Claude Code | Time | 10.6 s | 7.3 s | **31% less** | 52% less |
| | Tokens | 23,960 | 19,133 | **20% fewer** | 46% fewer |
| | Tool calls | 3.7 | 1.6 | **57% fewer** | 77% fewer |

**Average** is the mean session across all 9 tasks. **Best task** is the task
where Oko helped that tool most. Tool calls went down on every task for every
tool: one Oko search replaces several greps and file reads, and fewer steps is
where the time and tokens are saved. Time and tokens did not improve on every
task: on a few, a session with Oko was slower or used more tokens, and those are
included in the average.

How we keep it fair:

- **Real sessions, not search timings.** The clock covers the whole session,
  including the agent's thinking and Oko's own search time.
- **Same everything else.** Same task wording, model, and settings for both
  setups, on a pinned copy of each repository, with no project instructions
  other than Oko's guidance.
- **The setups run back to back and take turns going first**, so a slow minute
  at the model provider does not favor one of them.
- **Tokens are the agent's own**, cached input included. Oko's ranking calls to
  Jev are not counted, so these are not dollar savings.

It is a small benchmark on three repositories with Codex CLI 0.155 and OpenCode
1.18 (`gpt-5.6-sol`) and Claude Code 2.1 (`claude-sonnet-5`), all at low
reasoning effort. Your results will vary. See [full results and methodology](docs/benchmark-results.md)
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
