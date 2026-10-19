<p align="center">
  <img src="docs/images/oko.png" alt="Oko: a pixel-art eye with a blue iris" width="200">
</p>

<h1 align="center">Oko</h1>

<p align="center">
  <strong>Help your coding agent find code faster and use fewer tokens.</strong>
</p>

<p align="center">
  <a href="https://github.com/bartlomein/oko/actions/workflows/ci.yml"><img src="https://github.com/bartlomein/oko/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/bartlomein/oko/releases/latest"><img src="https://img.shields.io/github/v/release/bartlomein/oko" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/bartlomein/oko" alt="MIT license"></a>
</p>

Oko gives Claude Code, Codex, Cursor, OpenCode and Pi a better code search. When
you ask your agent about your code, Oko has already found the relevant source,
so the agent spends fewer rounds searching and reading files it does not need.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/agent-sessions-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/images/agent-sessions-light.svg">
    <img alt="Bar chart: with Oko, Claude Code spends 58% less time, 61% fewer tokens and 85% fewer tool calls; Codex 47%, 60% and 75%; OpenCode 49%, 63% and 86%; Pi 52%, 78% and 83%; Cursor 6%, 40% and 65%." src="docs/images/agent-sessions-light.svg" width="760">
  </picture>
  <br>
  <sub>Nine coding tasks on Astro, HTTPX and ripgrep, three runs each, with and without Oko. <a href="#results">Results</a> · <a href="docs/benchmark-results.md#agent-sessions">Method</a></sub>
</p>

- **Faster:** about half the time in Claude Code, Codex, OpenCode and Pi.
- **Cheaper:** 40–78% fewer tokens with every tool we measured.
- **Fewer detours:** 65–86% fewer tool calls.
- **Better ranking:** puts the right file first more often than any published method on [Agent Retrieval Bench](#other-benchmarks).
- **Local first:** search runs on your machine; only the question and selected snippets go to the ranker ([privacy](docs/privacy.md)).

## Quick start

```sh
curl -fsSL https://raw.githubusercontent.com/bartlomein/oko/main/install.sh | sh
cd your-project
oko setup --client claude     # or codex, cursor, opencode, pi, all
```

Then use your coding tool as usual. Setup asks for a [TypeSafe AI](https://typesafe.ai/)
key the first time, connects your tool to Oko for this project, and checks the
connection. Run it once per project; `oko upgrade` keeps everything current.

<details>
<summary><strong>Codex:</strong> approve Oko's hook once</summary>

The first time you open the project, Codex shows **Hooks need review**. Choose
**Trust all and continue**. If you skipped it, `/hooks` brings it back.
</details>

<details>
<summary><strong>Cursor:</strong> turn Oko on once</summary>

Cursor adds a project's tools switched off. Open **Customize > MCPs**, click
**oko** and turn it on.
</details>

<details>
<summary><strong>Pi:</strong> trust the project</summary>

Pi 1.0 or later. Trust the project when Pi asks, then check `/mcp`.
[Pi setup](docs/clients.md#pi-10-by-hand).
</details>

<details>
<summary><strong>Claude Code, OpenCode:</strong> nothing more to do</summary>

Start a new session after setup.
</details>

<details>
<summary>Platforms, manual install, no-key mode</summary>

macOS (Apple Silicon and Intel) and Linux (ARM64 and x64, glibc 2.35+). The
installer puts `oko` in `~/.local/bin`; add it to your `PATH` if needed.
[Other install options](docs/installation.md) · [Connect a tool by hand](docs/clients.md) ·
[Keys and configuration](docs/configuration.md). For search without a key,
add `--no-jev`: local keyword ranking only.
</details>

## Results

The same task given to the same coding tool twice, once with its own search and
once with Oko. Means per session, 27 sessions per side.

| Coding tool | Time | Tokens | Tool calls | Tasks passed (without → with) |
| --- | --- | --- | --- | --- |
| Claude Code | **58% less**<br><sub>12.0 → 5.0 s</sub> | **61% less**<br><sub>56,513 → 22,008</sub> | **85% less**<br><sub>4.4 → 0.7</sub> | 26 → 25 of 27 |
| Codex | **47% less**<br><sub>48.2 → 25.8 s</sub> | **60% less**<br><sub>72,247 → 28,751</sub> | **75% less**<br><sub>3.5 → 0.9</sub> | 26 → 27 of 27 |
| OpenCode | **49% less**<br><sub>25.7 → 13.1 s</sub> | **63% less**<br><sub>28,513 → 10,624</sub> | **86% less**<br><sub>5.5 → 0.7</sub> | 27 → 27 of 27 |
| Pi | **52% less**<br><sub>31.4 → 15.0 s</sub> | **78% less**<br><sub>41,886 → 9,261</sub> | **83% less**<br><sub>6.5 → 1.1</sub> | 25 → 25 of 27 |
| Cursor | **6% less**<br><sub>52.3 → 49.2 s</sub> | **40% less**<br><sub>182,940 → 109,520</sub> | **65% less**<br><sub>8.0 → 2.8</sub> | 26 → 24 of 27 |

<sub>Models: Claude Sonnet 5 (Claude Code), GPT-5.6 Sol (Codex, OpenCode, Pi), Grok 4.7 High (Cursor), at their default or low effort. Each tool counts tokens its own way, so compare within a row. Small benchmark; your results will vary. [Method, per-task results and caveats](docs/benchmark-results.md#agent-sessions) · [raw data](benchmarks/published/).</sub>

## Other benchmarks

| Benchmark | What it tests | Oko | Best comparison |
| --- | --- | --- | --- |
| [Sense's agent benchmark](docs/benchmark-results.md#senses-agent-benchmark-against-sense) | Six multi-step tasks, Claude Opus 4.7, run against Sense the same evening | Found **88%** of the locations to cite, at **half the cost** | Sense: 63% |
| [Agent Retrieval Bench](docs/benchmark-results.md#agent-retrieval-bench) | 345 tasks: find the files a developer needs next | Right file ranked first most often: **0.37** | Best published method: 0.24 |
| [SWE-Explore](docs/benchmark-results.md#swe-explore) | 848 real issues: rank the code fixing agents needed | One search ranks it within the range of agents that explore for many turns: **0.82** | Agents: 0.72–0.95 |

## Where Oko helps less

- **Cursor's own search is already fast,** so Oko saves less time there: mostly tokens and tool calls.
- **Duplicated code can mislead it.** When two files hold identical copies of a function, Oko may show the wrong one first; one Astro edit task in our benchmark fails this way.
- **Big changes across many files:** one search returns five places, so an issue touching many files still needs the agent's own exploring.

## Use it from your terminal

```sh
oko ask "where is authentication handled?"
oko ask "how are retries handled?" --json
oko stats                     # what ranking has cost you
```

More in [search options](docs/search.md).

## How it works

Oko searches your project locally (keywords, symbols and file structure), then
asks [Jev](https://typesafe.ai/), TypeSafe AI's relevance model, which of the
candidates actually answer the question, and returns those excerpts with exact
line numbers. `oko setup` connects it to your tool as an MCP server, adds short
guidance to the instructions your tool reads, and installs a hook that answers a
code question as you send it.
[What setup changes](docs/mcp.md#set-up-a-project) · [Privacy](docs/privacy.md).

## Documentation

| I want to… | Guide |
| --- | --- |
| Install, upgrade or troubleshoot | [Installation](docs/installation.md) |
| Connect a tool by hand, or understand the MCP tool | [Client setup](docs/clients.md) · [MCP reference](docs/mcp.md) |
| Configure keys, `.env` or caching | [Configuration](docs/configuration.md) |
| Search from the terminal, or rank my own documents | [Search](docs/search.md) · [Document ranking](docs/ranking.md) |
| See every benchmark and how it was run | [Benchmark results](docs/benchmark-results.md) |
| Know what leaves my machine | [Privacy](docs/privacy.md) |

## Contributing

Bug reports and ideas are welcome: [open an issue](https://github.com/bartlomein/oko/issues)
with your OS and `oko --version`, without keys or private code. To build and
test, see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE). Dependency attributions are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
