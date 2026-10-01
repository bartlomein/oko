<p align="center">
  <img src="docs/images/oko.png" alt="Oko: a pixel-art eye with a blue iris" width="280">
</p>

# Oko

**Help your coding agent find code faster and use fewer tokens.**

Oko helps Codex, Claude Code, and OpenCode spend less time searching and fewer
tokens reading irrelevant code. It delivers relevant source snippets through
MCP, and answers a code question as you send it, so your agent can get to the
task sooner. Gains vary by task and coding tool.

| Benchmark | What it measures | Oko |
| --- | --- | --- |
| [Our agent sessions](#agent-sessions) | Codex, OpenCode and Claude Code on nine tasks, with and without Oko | 60–63% fewer tokens, 47–58% faster, 75–86% fewer tool calls |
| [Sense's agent benchmark](#against-sense) | Claude Opus 4.7 on six multi-step tasks, run against Sense the same evening | Cited recall 0.88 against Sense's 0.63, at half the cost |
| [Agent Retrieval Bench](#agent-retrieval-bench) | 345 code-retrieval tasks in six languages | Right file first more often than any published method (MRR 0.37 against 0.24); top-20 recall 0.68, next to the best embedding model's 0.70 |
| [SWE-Explore](#swe-explore) | 848 real issues: the code that fixing agents needed | One search ranks the code about as well as agents that explore for many turns (nDCG 0.82, line precision 0.56) |

Each benchmark's method and caveats are in [its section below](#benchmarks).

Oko runs locally and uses [TypeSafe AI’s Jev](https://typesafe.ai/) to rank selected
source snippets. Use it through your coding agent or directly from your terminal.

[Install](#install) · [Connect your coding tool](#connect-your-coding-tool) · [Benchmarks](#benchmarks) · [CLI examples](#use-in-your-terminal) · [Documentation](#documentation)

```sh
oko ask "where is authentication handled?"
```

No Node.js or Rust runtime needed for downloaded binaries. Oko needs a TypeSafe AI
API key for Jev ranking; local keyword search also works without a key.

## Install

Run this on macOS or Linux to install the latest release:

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

To upgrade later, run `oko upgrade`: it installs the latest release and
refreshes every project you set up. Oko tells you when one is out.

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
sessions faster, not just cheaper. Setup also adds a prompt hook: when you ask a
code question, Oko's answer is in the agent's context before its first turn, so
it skips a round of searching. Start a new session afterwards. Repeat setup for
each project.

**In Codex, approve the hook once.** Codex runs no hook you have not reviewed.
The first time you open the project after setup, it shows **Hooks need review**:
choose **Trust all and continue** (or **Review hooks** to read Oko's first). It asks
once per project, and again only if a later Oko release changes the hook. If you
skipped it, `/hooks` brings it back. Claude Code and OpenCode need no extra step.

For a key-free connection, add `--no-jev`. See [what setup changes](docs/mcp.md#set-up-a-project).
The manual steps below do the same by hand, without the guidance.

### Claude Code by hand

From the project you want to search:

```sh
cd /path/to/your/project
oko auth login
claude mcp add-json --scope local oko \
  "{\"type\":\"stdio\",\"command\":\"$(command -v oko)\",\"args\":[\"mcp\",\"--root\",\"$PWD\"],\"alwaysLoad\":true}"
claude mcp list
```

Start a new Claude Code session and check `/mcp` for Oko. This connection is
private to you and scoped to this project; repeat the command for other projects.
Skip `oko auth login` if you already saved your key.

### OpenCode by hand

Save your key once with `oko auth login`, then add this to `opencode.json` in
your project root (merge the `oko` entry if the file exists):

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
search. A function, class or method named in the question is always found;
literal text can still be searched with native grep.

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

### Agent sessions

We give the same coding task to the same agent twice, once with its built-in
search and once with Oko set up by `oko setup`, and compare the whole session:
time, the agent's tokens, and tool calls. Nine tasks on pinned commits of
[Astro](https://github.com/withastro/astro), [HTTPX](https://github.com/encode/httpx),
and [ripgrep](https://github.com/BurntSushi/ripgrep): six ask for code spread over
several places, three ask for a small edit checked by tests. The wording never
names the file or function. Each task runs 3 times per setup with Codex, OpenCode,
and Claude Code: **162 sessions**, measured on the 0.7.0 code. With Oko, the
agent starts with Oko's answer to the prompt and usually needs no search of its
own: 63 of the 81 sessions with Oko made no Oko call.

| Coding tool | | Without Oko | With Oko | Average | Best task |
| --- | --- | --- | --- | --- | --- |
| Codex | Time | 48.2 s | 25.8 s | **47% faster** | 77% faster |
| | Tokens | 72,247 | 28,751 | **60% fewer** | 77% fewer |
| | Tool calls | 3.5 | 0.9 | **75% fewer** | 100% fewer |
| OpenCode | Time | 25.7 s | 13.1 s | **49% faster** | 63% faster |
| | Tokens | 28,513 | 10,624 | **63% fewer** | 85% fewer |
| | Tool calls | 5.5 | 0.7 | **86% fewer** | 100% fewer |
| Claude Code | Time | 12.0 s | 5.0 s | **58% faster** | 84% faster |
| | Tokens | 56,513 | 22,008 | **61% fewer** | 75% fewer |
| | Tool calls | 4.4 | 0.7 | **85% fewer** | 100% fewer |

With Oko, Codex passed 27 of 27, OpenCode 27 and Claude Code 25; without it,
26, 27 and 26. Three of the four misses are one Astro edit task, where the agent
also changed an identical copy of the function in another file (Claude Code
twice with Oko, Codex once without); the fourth is a Claude Code answer without
Oko citing a line past the end of a file. Tokens and tool calls fell on all nine
tasks for every tool, and time on all nine for Codex and OpenCode and eight for
Claude Code. Against Oko without the prompt hook, as in earlier releases, tokens
fell a further 35–40%. The other Claude Code hooks `oko setup` installs are not
part of these sessions, which use no subagents. It is a small benchmark with
Codex CLI 0.155 and OpenCode 1.18 (`gpt-5.6-sol`) and Claude Code 2.1
(`claude-sonnet-5`) at low reasoning effort; your results will vary.
[Full results and methodology](docs/benchmark-results.md#agent-sessions), the
[runner, tasks, and checks](scripts/benchmark-public/), and the
[raw results](benchmarks/published/0.7.0/) are in this repository.

### Against Sense

[Sense](https://github.com/luuuc/sense) publishes an agent benchmark: six
multi-step tasks on real repositories answered by Claude Opus 4.7, scored and
judged by its own harness. We ran it with Oko and with Sense on the same
evening, five runs per task each.

| | Oko | Sense |
| --- | --- | --- |
| Cited recall (the harness's headline) | **0.883** | 0.625 |
| Blended score | **0.922** | 0.727 |
| Relations stated correctly | **0.943** | 0.733 |
| Sessions finished within budget and time | **30 of 30** | 21 of 30 |
| Cost, 30 sessions | **$25.19** + about $0.31 of Jev | $50.55 |
| Mean time per session | **161 s** | 212 s |

Cited recall and relations come from one task (Discourse, 24 locations) run
five times. With this Claude Code version (2.1.286), Sense went over its budget
in 8 of 30 sessions and ran to the time limit in one; the harness scores what
those sessions wrote. In our earlier paired run, on an older Claude Code, Sense
scored 0.875. [Details](docs/benchmark-results.md#senses-agent-benchmark-against-sense).

### Agent Retrieval Bench

[Agent Retrieval Bench](https://arxiv.org/abs/2607.24882) has 345 tasks from 25
repositories in six languages: given a repository and a signal from a coding
workflow (a failing test's output, a pull request, a review comment, a code
change), find the files a developer needs next. Oko's numbers are the mean of
three runs on the 0.7.0 code, scored with the benchmark's own code; the
baselines were run locally on the same tasks and reproduce the published
figures.

| Method | Right file in top 20 | First right file ranks high (MRR) | Needed code within 8k tokens |
| --- | --- | --- | --- |
| **Oko** | 0.68 | **0.37** | **0.49** |
| Qwen3-Embedding-8B (GPU, index) | **0.70** | 0.23 | 0.37 |
| RepoMap | 0.64 | 0.22 | 0.38 |
| Qwen3-Embedding-4B (GPU, index) | 0.63 | 0.24 | 0.34 |
| Lexical | 0.49 | 0.16 | 0.27 |
| BM25 | 0.45 | 0.15 | 0.21 |

Each task is one search, so setup's prompt hook plays no part here. Per-task
tables, the held-out split, what was tuned on what, and the limits are in
[the details](docs/benchmark-results.md#agent-retrieval-bench).

### SWE-Explore

[SWE-Explore](https://arxiv.org/abs/2606.07297) has 848 real issues from
SWE-bench Verified, Pro and Multilingual in ten languages. The answer is the
code that successful agents read while fixing each issue; an explorer returns
five ranked regions. The agent rows are the paper's published results; they
explore for many turns with a frontier model, while Oko makes one search of
about a second.

| Method | Right file in top 5 | Line precision | Ranking (nDCG@500) |
| --- | --- | --- | --- |
| Claude Code (agent) | **0.67** | 0.60 | 0.94 |
| Mini-SWE-Agent (agent) | 0.64 | 0.53 | 0.89 |
| LocAgent (agent) | 0.54 | 0.64 | **0.95** |
| CoSIL (agent) | 0.54 | 0.58 | 0.82 |
| **Oko, one search** | 0.40 | 0.56 | 0.82 |
| AutoCodeRover (agent) | 0.28 | **0.68** | 0.72 |
| TF-IDF | 0.14 | 0.10 | 0.22 |
| BM25 | 0.07 | 0.05 | 0.12 |

On ranking and precision one Oko search sits among the agents; on finding every
file an issue needs it does not, because five regions from one search cannot
cover the 4.3 files an issue needs on average. [Details](docs/benchmark-results.md#swe-explore).

## Privacy

File discovery and initial search run locally. Jev ranking sends your question and
selected source snippets to TypeSafe AI; with setup's prompt hook, a prompt that
reads as a code question is searched as you send it, so its first paragraph is
the question. Once a day Oko asks GitHub for the latest release number, and
nothing else; `--no-jev` and `OKO_NO_UPDATE_CHECK=1` turn that off. Deep mode can send more snippets across
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
