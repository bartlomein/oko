# Oko

Oko is a Rust CLI and library for finding relevant code and ranking supplied text.
The application and its tests are Rust. Node.js is used only by optional
developer benchmark runners.

See [the measured Rust comparison](benchmarks/rust-comparison.md): local code
search was effectively unchanged; supplied-item CLI overhead fell from about
40 ms to 5 ms, with matching behavior on the tested fixtures.

## Install and run

Build with a current stable Rust toolchain (edition 2024) and install
[ripgrep](https://github.com/BurntSushi/ripgrep) for code discovery:

```sh
cargo install --path . --bin oko --locked
oko --help
cd /path/to/your/project
oko ask "where is authentication handled?"
oko ask "where is authentication handled?" --json
oko ask "where is authentication handled?" --no-jev
oko ask "how does authentication work?" --intent explanation
```

For a local release build instead, run `cargo build --release --locked --bin oko`.
The executable is `target/release/oko` (`oko.exe` on Windows). Build for each
operating system and CPU architecture you distribute to. No JavaScript wrapper
is required. Code search requires `rg` on PATH; supplied-item ranking does not.

Put your TypeSafe AI API key in `.env` **in the directory where you run Oko**:

```dotenv
TYPESAFE_API_KEY=your-api-key
```

Use `.env.example` as a template. Existing shell variables take precedence,
including an explicitly empty value. Values are literal, without `$VARIABLE`
expansion. `.env` and `.env.*` are ignored by Git except `.env.example`.
When invoking Oko from another repository, put the key in that repository's
ignored `.env` or export `TYPESAFE_API_KEY` in your shell.

`TYPESAFE_BASE_URL` and `TYPESAFE_DEFAULT_MODEL` may be set in the process
environment. Their defaults are `https://api.typesafe.ai` and `jev-latest`.
These two settings are not read from `.env`.

## Code search

Oko discovers files with `rg --files`, reads UTF-8 text files up to 256 KiB,
rejects binary/invalid UTF-8 content, and builds a deterministic shortlist.
Search splits snake_case and camelCase names, ignores common question words,
and matches English word forms using the Porter stemmer.

Candidates use BM25 with `k1=1.2` and `b=0.75`: uncommon query words carry
more weight, repeated words have diminishing returns, and document length is
normalized. Content and path are scored independently, then combined as
`content + 0.3 * path`. These are relevance scores, not probabilities.
Deep search caches term counts once and uses full-snapshot statistics even when
filtering to source files. This scorer needs no additional model or service.
The [BM25 validation](benchmarks/bm25.md) scored 42/45 first-result hits and
45/45 top-five hits in both ordinary and deep modes on the existing fixture,
with medians of 1.35 s and 1.95 s respectively. These development results match
the earlier agent comparison on this fixture, not general accuracy parity.

Recognized Rust, JavaScript/TypeScript `function`, and Python `def` declarations
start sections of up to 120 lines, keeping nearby comments with their functions.
This is a declaration heuristic, not an AST parser. Longer sections are split;
other syntax uses 40-line windows. Results preserve source text, relative paths,
and inclusive 1-based line ranges.

Normal `ask` sends the question and up to 30 shortlisted code chunks to TypeSafe
AI for one Jev request. It never sends the full repository. A 32,000-byte request
budget drops trailing candidates while retaining complete chunks. This is a
byte budget, not a token count. Requests have a 10-second timeout and no retries.
Missing credentials or provider failures exit nonzero.

`--no-jev` skips the API and returns lexical ranking. JSON goes to stdout;
diagnostics go to stderr. Intent-aware instructions refine the default code ranking.

## Jev-only investigation

```sh
oko ask --deep --max-steps 5 "Where is authentication handled?"
oko ask --deep --json "Where are optional values interpolated?"
```

`--deep` adds a search/read/decision loop using the same `TYPESAFE_API_KEY`.
No Codex, OpenCode, OpenAI or Anthropic connection is used. Normal `ask` remains
the fast single-pass path.

Oko proposes searches from the question's words and adjacent word pairs, with
source-code searches for implementation intent. It also offers nearby chunks
and definitions of symbols observed in results. Jev chooses the next offered
action or finishes, then reranks newly discovered evidence with existing results.
Jev selects typed actions: it does not generate arbitrary queries or shell commands.
This is a constrained investigation loop, not a full OpenCode-style coding agent.

`--max-steps N` optionally limits local search/read actions, including the initial
search. A step can require a ranking call and an action-selection call; JSON
reports the actual `jevCalls`. No step cap is imposed when omitted. The loop
also stops when Jev chooses to finish or all offered actions are exhausted.
Actions that expose the same evidence are deduplicated. An empty answer cannot
finish while untried actions remain; Jev must choose another action. Actions do
not repeat. Each provider call retains the existing ten-second timeout,
32 KB request budget and no automatic retries. No total wall-time or token-cost
budget is implemented. Use a step limit when bounding API use matters.

The repository is read once into a snapshot using the same ignored-file, UTF-8,
256 KiB file-size and chunking rules as ordinary search. Local actions only use
that snapshot; there are no edits, shell commands or reads of model-provided paths.
Progress goes to stderr. JSON includes `investigation` with steps, call count,
action trace, omitted candidates and stop reason. `complete` is true only when
Jev chooses to finish; a budget stop returns current findings with `complete:false`.
That flag is the controller's stopping decision, not proof the answer is correct.
`--deep` cannot be combined with `--no-jev` or used with generic `rank` inputs.

Run the synthetic live smoke benchmark with `node scripts/benchmark-investigation.mjs`
after building the release binary. It generates toy source and distractor docs,
uses Jev, and saves reports under `benchmarks/results/`. On September 18, 2026,
deep search found the expected function first in 3/3 repeats versus 0/3 for
single-pass search; median times were 1.17 s and 0.47 s respectively. Deep search
used two steps and three Jev calls per run, stopping with actions exhausted.
This is a development case, not a held-out accuracy evaluation.

For a real-repository comparison using only Jev:

```sh
node scripts/compare-investigation.mjs /path/to/repository /path/to/fixture.json
```

The fixture uses the same source-hashed expected locations as the agent comparison.
The runner compares ordinary search with five-step deep search over three repeats,
rotates execution order, and records accuracy, elapsed time, Jev calls and action
traces. Repository snippets are sent to Jev. Results are saved under
`benchmarks/results/deep-comparison-*/`; source and executable snapshots are
checked before and after the run.

[Real-repository validation](benchmarks/jev-investigation.md): caching processed
words reduced deep mode's median time from 19.23 s to 1.82 s on the 15-question
fixture. The follow-up scored 34/45 first-result hits and 41/45 top-five hits
for deep mode versus 35/45 and 42/45 for ordinary search (1.19 s median).
Deep mode remains experimental; this optimization targets speed, not accuracy.

## Ranking intent

Both `ask` and `rank` accept `--intent implementation|explanation|general`:

| Intent | Prefers | Default for |
|---|---|---|
| `implementation` | Code that performs the behavior, ahead of docs, examples, tests, or callers | `ask` |
| `explanation` | Content explaining how or why something works, including docs and comments | Explicit selection |
| `general` | Items that best answer the question, with the original generic instructions | `rank` and the Rust library |

The command chooses the default; Oko does not ask AI to guess the intent.
Intent only changes the instructions within the existing Jev request. A normal
nonempty ranking still makes one call, with the same 32,000-byte budget and no
retries. Longer instructions can leave slightly less room for candidates. No
file types are excluded and candidate discovery is unchanged. `--no-jev` makes
zero calls and bypasses intent-based ranking, preserving its existing results.

```sh
oko ask "where is authentication handled?" --intent general
oko rank --input articles.json "how does authentication work?" --intent explanation
```

For library callers, set `RankOptions.intent` to `RankingIntent::Implementation`,
`RankingIntent::Explanation`, or `RankingIntent::General` (the default). Existing
callers using `..Default::default()` keep general ranking. Full struct literals
must supply the new field. `prepare_request` retains general behavior;
`ranking::prepare_request_with_intent` exposes explicit request preparation.

Initial validation on 2026-09-18: code locations improved from 4/5 to 5/5 correct
first results in one live pass (1.19 s median), generic tickets remained 5/5, and
all four synthetic implementation/explanation checks passed. These are small
development smoke tests; the full three-repeat Codex comparison below predates
intent support and has not been rerun with the new default.

## Rank documents or database results

Provide a JSON array of items:

```json
[{"id":"ticket-42","text":"Charged twice for the same order.","source":"tickets/42"}]
```

```sh
oko rank --input examples/items.json "Who needs a refund?" --json
```

Files are limited to 1 MiB and 30 items. IDs must be unique, nonempty strings of
at most 200 UTF-16 code units (the existing JavaScript contract); text must be
nonempty. `source` is optional. Unknown fields are discarded.

Results preserve IDs and sources, and return up to five items whose scores beat
the `none` choice. `omittedCount` counts items dropped by the request byte budget,
not items outside the top five. Supply candidates in your existing search order.
`--no-jev` preserves that order with zero scores; it is an input-order baseline,
not keyword search. Scores are relative to the candidates, not universal
relevance probabilities. Oko does not fetch URLs or connect to databases.

Any programming language can invoke the CLI and consume its JSON output.
Rust applications can use the library directly:

```rust,no_run
use oko::{rank_items, RankItem, RankOptions};

fn main() -> anyhow::Result<()> {
    let items = vec![RankItem {
        id: "ticket-42".into(),
        text: "Charged twice for the same order.".into(),
        source: Some("tickets/42".into()),
    }];
    let result = rank_items("Who needs a refund?", &items, &RankOptions {
        api_key: std::env::var("TYPESAFE_API_KEY").ok(),
        ..Default::default()
    })?;
    println!("{:?}", result.results);
    Ok(())
}
```

The library is synchronous and returns `Result`; async callers should run it on
a blocking worker. It does not load `.env` itself. Use `no_jev: true` to preserve
input order without a request. `parse_items` validates unknown JSON first.


## Accuracy benchmarks

The application includes native Rust benchmark commands:

```sh
oko benchmark --repo /path/to/telemetry-studio --repeats 1
oko benchmark-items --repeats 1
```

Run these from Oko's directory to load its `.env` and save reports under
`benchmarks/results/` (ignored). Each command makes five Jev requests per repeat.
Code benchmarking validates source hashes against the embedded
`benchmarks/telemetry-studio.json` fixture before requesting Jev. It reads the
target repository without editing it. Item benchmarking uses only synthetic
support tickets, including a no-match question.

Reports contain errors, median end-to-end CLI time, and accuracy. Code metrics
separate function-range overlap and exact implementation-range overlap, for both
the first result and top five. Merely returning the correct file does not count.
Both Rust benchmarks time fresh CLI processes; the historical TypeScript item
benchmark measured API ranking time only, so those times are not comparable.
Five questions are a development smoke test, not proof of general accuracy.

To check code-versus-explanation preference on two synthetic candidate sets
(four Jev requests), run `node scripts/benchmark-intents.mjs`. The questions and
candidate contents remain the same while the requested intent changes. Its
fixture is `benchmarks/ranking-intents.json`; reports are ignored under
`benchmarks/results/intents-*/`. The ticket benchmark checks general ranking.

Add `--no-jev` to either benchmark for an offline baseline. Code benchmarks also
accept `--baseline /path/to/report.json` to rescore an earlier report; repository
commit, working-tree status, and expected-source hashes must match. Repeats are
limited to 1–10. Errors count as misses and produce a nonzero exit status.

## Development

### Compare against Codex CLI

See [the measured Codex comparison](benchmarks/codex-comparison.md) for the first
three-repeat run: Oko 1.22 s median and 12/15 correct first, Codex 19.20 s and
15/15 correct first. Both returned the correct location in the top five on all
trials. These are five repeated development questions, not a general benchmark.

```sh
cargo build --release --locked --bin oko
node scripts/compare-codex.mjs /path/to/telemetry-studio --repeats 3 --model gpt-6-astra --effort medium
```

This developer runner requires Node.js 20.12+ and an authenticated Codex CLI that
supports the selected model. It compares Oko with Jev against fresh, read-only
`codex exec` sessions on the same five verified questions. Three repeats make
15 Oko/Jev requests and 15 Codex sessions. Oko's key comes from this repository's
`.env` or the shell; Codex reuses its saved authentication.

Codex receives only the question and output instructions, without expected
answers. Its final response follows `benchmarks/codex-output.schema.json`.
Both tools return up to five locations with ranges capped at 120 lines. The
runner grades exact/function overlap, first/top-five accuracy, failures, and
end-to-end time. Codex usage and executed commands are saved for inspection;
raw source snippets and provider stderr are not saved. Reports are ignored under
`benchmarks/results/comparison-*/`. Trials rotate tool order, run sequentially, and stop
on the first error. Timeouts are 180 seconds per invocation, with no retries.

The CLI model and reasoning setting are pinned; user config is ignored and
sessions are ephemeral. Repository instructions still apply. The target
repository snapshot and Oko binary are checked before/after the run. Results
compare complete code-location workflows, including AI and network time, not
raw search-engine performance. These development questions are not held out.

Use `--codex-bin /absolute/path/to/codex` to select another installed executable.
On this machine, the shell's CLI 0.143.0 rejects `gpt-6-astra`; the compatible
desktop-bundled executable is `/Applications/ChatGPT.app/Contents/Resources/codex`
(0.153.4). The runner records the actual CLI version in each report.

```sh
node --test scripts/compare-codex.test.mjs
```

### Compare against Codex and OpenCode

See [the expanded comparison](benchmarks/agent-comparison.md) for methodology
and recorded results.

```sh
cargo build --release --locked --bin oko
node scripts/compare-agents.mjs /path/to/telemetry-studio --repeats 3 --model gpt-5.5 --effort medium
```

This runner uses **the same model and reasoning setting in both agent CLIs**:
Codex receives `gpt-5.5`; OpenCode receives `openai/gpt-5.5` with the `medium`
variant. Both CLIs must be installed and authenticated for that model. Oko
continues to use Jev; it is the system being compared, not a third GPT agent.
Override executable paths with `--codex-bin PATH` and `--opencode-bin PATH`.
No global configuration is changed.

The expanded fixture has 15 source-verified questions: the original five
`development` questions and ten `new` questions written before this comparison.
Reports show those groups separately. Three repeats make 45 trials per tool,
135 total. These are mostly localized function lookups in one Rust project,
not a broad evaluation of coding agents or search across documents/databases.

OpenCode runs fresh sessions with external plugins disabled and a dedicated
read-only agent using its native read, glob, and grep tools. Shell, edits,
web, and delegation are denied. These are tool permissions, not an OS sandbox.
Codex uses its read-only sandbox and usual local shell tools. Same model does
not make system prompts, tool access, provider caching, or context identical.
OpenCode stores local sessions; Codex sessions are ephemeral. No sessions are
shared. The runner stores parsed locations, step token counters, and tool types,
not raw source/tool outputs.

Each tool rotates through execution positions across repeats. Trials run
sequentially with a 180-second timeout and no automatic retries. Errors count
as misses and stop the run; incomplete reports are explicitly marked. All
returned paths/ranges are checked, and repository snapshots and source hashes
are verified. Median latency excludes failed trials. “Exact” accuracy means
line-range overlap with the verified implementation, not an exact boundary match.

Use `--tools codex,opencode` to run just those agents, or `--fixture PATH` for
a different repository's JSON questions and source-verified expected locations.
The default expanded fixture is `benchmarks/telemetry-studio-expanded.json`.
The offline runner tests require neither the target repository nor API keys:

```sh
node --test scripts/compare-codex.test.mjs
```

### Rust checks

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Third-party parser/stemmer notices are in `THIRD_PARTY_NOTICES.md`.
