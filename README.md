<p align="center">
  <img src="docs/images/oko.png" alt="Oko: a pixel-art eye with a blue iris" width="280">
</p>

# Oko

**Help your coding agent find code faster and use fewer tokens.**

Oko helps Codex, Claude Code, and OpenCode spend less time searching and fewer
tokens reading irrelevant code. It delivers relevant source snippets through
MCP so your agent can get to the task sooner. Gains vary by task and coding tool.

| Benchmark | What it measures | Oko 0.6.1 |
| --- | --- | --- |
| [Sense's agent benchmark](#against-sense) | Claude Opus 4.7 on six multi-step tasks, run against Sense on the same day | Cited recall 0.91 against Sense's 0.88, at about 40% lower cost (measured on 0.6.0) |
| [Agent Retrieval Bench](#agent-retrieval-bench) | 345 code-retrieval tasks in six languages | Right file first more often than any published method (MRR 0.37 against 0.24); top-20 recall 0.68, next to the best embedding model's 0.70 |
| [SWE-Explore](#swe-explore) | 848 real issues: the code that fixing agents needed | One search ranks the code about as well as agents that explore for many turns (nDCG 0.81, line precision 0.56) |
| [Our agent sessions](#agent-sessions) | Codex, OpenCode and Claude Code on nine tasks, with and without Oko | 31–40% fewer tokens, 46–62% fewer tool calls, 19–44% faster |

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

### Agent Retrieval Bench

[Agent Retrieval Bench](https://arxiv.org/abs/2607.24882) has 345 tasks from 25
repositories in six languages: given a repository and a signal from a coding
workflow (a failing test's output, a pull request, a review comment, a code
change), find the files a developer needs next. Oko's numbers are the mean of
three runs, scored with the benchmark's own code; the baselines were run
locally on the same tasks and reproduce the published figures.

| Method | Right file in top 20 | First right file ranks high (MRR) | Needed code within 8k tokens |
| --- | --- | --- | --- |
| **Oko 0.6.1** | 0.68 | **0.37** | **0.50** |
| Oko 0.6.0 | **0.70** | **0.37** | **0.50** |
| Qwen3-Embedding-8B (GPU, index) | **0.70** | 0.23 | 0.37 |
| RepoMap | 0.64 | 0.22 | 0.38 |
| Qwen3-Embedding-4B (GPU, index) | 0.63 | 0.24 | 0.34 |
| Lexical | 0.49 | 0.16 | 0.27 |
| BM25 | 0.45 | 0.15 | 0.21 |

0.6.1 shows Jev more of the first candidates' code, which is what makes
agents' first answers more complete. The first right file ranks as high as in
0.6.0 and the needed code within 8k tokens is the same; top-20 recall is 0.02
lower, most of it on failing-test tasks, where files the ranker already rated
low now fall just past rank 20. 0.6.0 rerun the same week scored 0.69.

Per-task tables, the held-out split, what was tuned on what, and the limits
are in [the details](docs/benchmark-results.md#agent-retrieval-bench).

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
| **Oko 0.6.1, one search** | 0.40 | 0.56 | 0.81 |
| Oko 0.6.0, one search | 0.40 | 0.56 | 0.81 |
| AutoCodeRover (agent) | 0.28 | **0.68** | 0.72 |
| TF-IDF | 0.14 | 0.10 | 0.22 |
| BM25 | 0.07 | 0.05 | 0.12 |

On ranking and precision one Oko search sits among the agents; on finding every
file an issue needs it does not, because five regions from one search cannot
cover the 4.3 files an issue needs on average. [Details](docs/benchmark-results.md#swe-explore).

### Against Sense

[Sense](https://github.com/luuuc/sense) publishes an agent benchmark: six
multi-step tasks on real repositories answered by Claude Opus 4.7, scored and
judged by its own harness. We ran it with Oko and with Sense on the same day,
five runs per task each.

| | Oko 0.6.0 | Sense |
| --- | --- | --- |
| Cited recall (the harness's headline) | **0.908** | 0.875 |
| Blended score | **0.933** | 0.931 |
| Relations stated correctly | 0.933 | **1.000** |
| Cost, 30 sessions | **$31** | $50 |
| Mean time per session | **210 s** | 242 s |

The lead is small, and cited recall comes from one task (Discourse, 24
locations) run five times; Sense states the relations between locations more
exactly. Oko's lead holds however the failed sessions are counted, and Oko was
faster on five of the six tasks (not on Next.js, where one Oko session ran to
the time limit). [Details](docs/benchmark-results.md#senses-agent-benchmark-against-sense).

We did not rerun this comparison for 0.6.1. On two of the six tasks (Axum and
Discourse, five runs each), 0.6.1 and a 0.6.0 build ran the same evening:
cited recall 0.91 against 0.79, with 60–80% fewer whole-file reads.

### Agent sessions

We give the same coding task to the same agent twice, once with its built-in
search and once with Oko set up by `oko setup`, and compare the whole session:
time, the agent's tokens, and tool calls. Nine tasks on pinned commits of
[Astro](https://github.com/withastro/astro), [HTTPX](https://github.com/encode/httpx),
and [ripgrep](https://github.com/BurntSushi/ripgrep): six ask for code spread over
several places, three ask for a small edit checked by tests. The wording never
names the file or function. Each task runs 3 times per setup with Codex, OpenCode,
and Claude Code: **162 sessions**, measured on Oko 0.6.1.

| Coding tool | | Without Oko | With Oko | Average | Best task |
| --- | --- | --- | --- | --- | --- |
| Codex | Time | 25.1 s | 19.7 s | **21% faster** | 45% faster |
| | Tokens | 70,512 | 48,814 | **31% fewer** | 65% fewer |
| | Tool calls | 3.3 | 1.8 | **46% fewer** | 79% fewer |
| OpenCode | Time | 22.5 s | 18.3 s | **19% faster** | 51% faster |
| | Tokens | 29,429 | 19,375 | **34% fewer** | 68% fewer |
| | Tool calls | 5.7 | 2.3 | **60% fewer** | 86% fewer |
| Claude Code | Time | 14.0 s | 7.9 s | **44% faster** | 62% faster |
| | Tokens | 61,384 | 36,800 | **40% fewer** | 61% fewer |
| | Tool calls | 4.9 | 1.9 | **62% fewer** | 84% fewer |

With Oko, Codex passed 26 of 27, OpenCode 26 and Claude Code 22; without it,
27, 26 and 23. Four of the seven misses with Oko are one edit task where Astro
has two identical copies of the function to change and the agent edits the
other one. The other three are Claude Code's: an edit that also changed
documentation and an answer citing a line past the end of a file (mistakes it
made four times without Oko), and one answer that missed two of four
locations. Time, tokens and tool calls each fell on seven to nine of the nine
tasks for every tool. Against 0.6.0,
Codex and OpenCode send fewer follow-up Oko searches (a second search in 4 and
8 of 18 search sessions, from 11 and 15), because the first answer more often
holds everything. Claude Code updated itself between runs (2.1.282 → 2.1.285)
and uses more tokens with and without Oko than before, so compare it within
this run. The Claude Code hooks `oko setup` installs are not part of these
sessions, which use no subagents. It is a small benchmark with Codex CLI 0.155
and OpenCode 1.18 (`gpt-5.6-sol`) and Claude Code 2.1 (`claude-sonnet-5`) at
low reasoning effort; your results will vary.
[Full results and methodology](docs/benchmark-results.md#agent-sessions), the
[runner, tasks, and checks](scripts/benchmark-public/), and the
[raw results](benchmarks/published/0.6.1/) are in this repository.

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
