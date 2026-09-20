# Benchmarking and development

## Shareable benchmark evidence

All new benchmark runners should target the versioned
[`oko-benchmark/v1` format](../benchmarks/oko-benchmark/v1/README.md). It keeps
Rust-inside Oko measurements separate from Python-outside client timing, retains
failed-run denominators, reports median/p95 successful latency, and never guesses
missing token usage. The Twenty runner is the first integrated writer; existing
historical scripts may keep their local artifacts while migrating their outer
reports to the shared module.

[← Back to Oko](../README.md)

## Public repository comparison

The [public repository runner](../scripts/benchmark-public/README.md) compares
Codex, OpenCode, and Claude Code on pinned Astro, HTTPX, and ripgrep checkouts.
Each repository has two search tasks and two small edits, tested without Oko,
with a cold cache, and with a prebuilt disk cache (108 sessions). It reports
elapsed time, agent token usage, and task checks separately. The default command
prints the plan; model sessions require `--execute`. Read the methodology and
grading limits before interpreting results or making performance claims.

## Documenso fast comparison

The [Documenso runner](../scripts/benchmark-documenso/README.md) uses 3 read-only
searches and 2 small edits across Codex, OpenCode, and Claude Code, each with Oko
off/on (30 sessions). It reuses the Twenty runner's adapters and grading, with
separate frozen source and results. Run `python3 scripts/benchmark-documenso/runner.py`
to preview the plan; add `--execute` to launch sessions.

## Twenty read/edit comparison

The [Twenty runner](../scripts/benchmark-twenty/README.md) runs 10 read-only
questions and 5 small edits across Codex, OpenCode, and Claude Code, each with
Oko off and on (90 sessions). It uses a frozen public checkout and a disposable
copy per session. Add `--fast` for 3 read-only searches and 2 edits (30 sessions).
The default command only prints the plan; `--execute` launches
model sessions. See the [questions](../scripts/benchmark-twenty/tasks.md), setup,
grading limits, and pilot instructions before running.

## Measure cache latency without model calls

For a quick correctness smoke test (about a few seconds after building):

```sh
python3 scripts/test-cache.py
```

It creates an 81-file temporary fixture and checks cold/warm searches, an
immediate same-length edit with restored modification time, deletion, and server
restart. It reports reads, reused contents, and rebuilt files. No real repository
is edited and no model calls are made. Unsupported filesystems may use the safe
full-read fallback; the output says when content reuse was not exercised.
Timings on this tiny fixture are diagnostic, not a project performance benchmark.

To include real Jev ranking, explicitly opt in:

```sh
python3 scripts/test-cache.py --live --report /tmp/oko-live-cache.json
```

This makes three normal Jev requests in one MCP session: cold, warm, and after
an immediate edit. It sends only generated synthetic source, stops on failure,
and never retries. It checks the returned source and reports cache time separately
from client-side ranking time (HTTP, provider wait, and response parsing).
The environment's `TYPESAFE_API_KEY` takes priority over a single-line key in
Oko's `.env` (`--env-file` overrides the path), then Oko's saved OS credential.
Keys are not included in output or reports. The model defaults to `jev-1.13.0`;
`--jev-model` overrides it. Offline mode remains the default.

```sh
cargo build --release --locked --bin oko
python3 scripts/profile-cache.py --root /path/to/project > /tmp/oko-cache-profile.json
```

This read-only measurement uses MCP with `--no-jev` and a temporary cache outside
the searched project. It measures an empty-cache request, five repeated requests
in the same server, then a restarted server using the disk cache and five more
repeats. The report separates these states and checks that returned results stay
identical. It includes cache timings and source-read/reuse counters when the
binary supports them. `--binary`, `--question`, and `--repeats` are configurable.
Keep the workspace unchanged during measurement. No credentials are required.

These are local retrieval timings, not complete coding-agent timings. “Cold”
means an empty Oko cache, not an empty operating-system file cache. MCP startup
is reported separately. Fresh-cache agent trials measure first-use cost;
repeat searches in a long-lived MCP server measure ongoing development use.
Report both separately rather than silently warming every trial.

## Accuracy benchmarks

Benchmarks are optional. Normal use does not require Telemetry Studio or another
private checkout. `cargo test --locked` requires no API key or external checkout. The published code
accuracy results use a small, repeatedly used Telemetry Studio development
fixture; they are not a general claim of matching Codex or OpenCode. The code
benchmark requires a matching checkout and validates its source hashes. The
item benchmark uses synthetic records and is portable.

The application includes native Rust benchmark commands:

```sh
oko benchmark --repo /path/to/telemetry-studio --repeats 1
oko benchmark-items --repeats 1
```

These commands use the same credential priority as searches, including saved
keys. Run them from Oko's directory to save reports under `benchmarks/results/`
(ignored). Each command makes five Jev requests per repeat.
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

Optional Node.js benchmark runners read keys from the shell or this repository's
`.env`; they do not read the OS credential store.

### Compare against Codex CLI

See [the measured Codex comparison](../benchmarks/codex-comparison.md) for the first
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
Choose a model supported by your installed CLI and account. The runner records
the actual CLI version in each report.

```sh
node --test scripts/compare-codex.test.mjs
```

### Compare against Codex and OpenCode

See [the expanded comparison](../benchmarks/agent-comparison.md) for methodology
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
