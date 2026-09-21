# Public-repository benchmarks

## Reproduce the README numbers

The README table comes from the `branch` suite with the guidance condition: nine
tasks × three clients × three setups (without Oko, Oko, Oko with the guidance
`oko setup` installs) × three repetitions = 243 timed sessions. You need Rust,
ripgrep, Node.js, a TypeSafe key, and signed-in `codex`, `opencode`, and `claude`
commands. The first three commands are free; `--execute` makes paid model and Jev
calls and takes about an hour and a half.

```sh
python3 scripts/benchmark-public/runner.py --suite branch --guided --skip-previous --prepare
python3 scripts/benchmark-public/selftest.py
python3 scripts/benchmark-public/runner.py --suite branch --guided --skip-previous --check
python3 scripts/benchmark-public/runner.py --suite branch --guided --skip-previous --execute
```

Results land in `benchmarks/results/public-branch/results-*/` as `report.md` and
`report.json` (every session, with its tool calls, tokens, and timing). The tasks
and their expected answers are in [`tasks-branch.json`](tasks-branch.json); the
edit checks are in [`validate.py`](validate.py). To try fewer clients, add
`--clients claude`. Models and providers are not deterministic, so expect your
numbers to differ somewhat from ours.

## Branch comparison: next run

The `branch` suite compares **native search, previous Oko, and current Oko** on
**nine new tasks × three clients × three repetitions = 243 timed sessions**.
Six additional short memory-canary sessions run first. Preparation and checks
make no paid calls; only `--execute` starts client/Jev requests.

```sh
python3 scripts/benchmark-public/runner.py --suite branch --prepare
python3 scripts/benchmark-public/selftest.py
python3 scripts/benchmark-public/runner.py --suite branch --check
# Start only when ready for the paid run:
python3 scripts/benchmark-public/runner.py --suite branch --execute
```

Preparation builds immutable Git snapshots with `cargo build --release --locked
--offline`, leaving the working checkout untouched. The default previous ref is
`main`; the current ref is `HEAD`. At setup these resolved to `f08c98b` and
`887043a`, respectively: this comparison measures compact responses plus startup
preparation together. Use `--baseline-ref perf/slim-mcp-response` during preparation
to isolate just the startup-preparation change. Full commit IDs, executable hashes,
compiler versions, fixtures, validator dependency, runner code, and CLI versions
are frozen. A cached build is verified, never silently overwritten.

Both Oko versions use **warm disk cache** by default: each session gets its own
cache built by that session's exact binary, outside the agent timer. No index is
shared between builds. For a cold comparison, prepare and execute with
`--cache-policy cold`. `--repeats` defaults to three for this suite. Conditions
rotate so each occupies every position once for each task/client across three
repetitions; repository/client order is interleaved. `--clients codex` selects
81 timed sessions plus two canary sessions. Preparation/execute are mutually
locked, and changed frozen settings stop execution.

### New tasks

| Repository | Cross-file search 1 | Cross-file search 2 | Edit |
|---|---|---|---|
| Astro | Image probing authorization before/after redirects and shared allowlist | Action-path decoding, own-property and forbidden-key guards | Empty first forwarded-header value returns undefined |
| HTTPX | Content-Encoding selection and reversed decoder chain | Async auth generator lifecycle and conditional body reads | Unknown status reason-phrase fallback |
| ripgrep | Capture slicing, dollar escaping, named expansion | Per-search printed-byte accounting | Hyphens in replacement capture names |

These are hypothetical, bounded edits, not upstream bug claims. Each edit contract
must fail on the original and pass on the reference change. Rust validation compiles
the complete real interpolation module (including existing tests) against the actual
`memchr` library produced by the pinned Oko build. Its hash and rustc version are
checked before execution. Source anchors cover the requested decisions, not unrelated
surrounding lines. The new suite is saved separately as `tasks-branch.json` and
`benchmarks/results/public-branch/`; old run artifacts are not rewritten.

### Memory isolation and its limits

Every timed session has a fresh checkout and conversation. Codex gets a fresh CLI
state directory with memories disabled. OpenCode gets separate config, data, state,
and cache directories; only its login file is linked. Claude auto-memory and session
persistence are disabled. Repository instructions/skills are stripped from temporary
checkouts; personal memory is never deleted.

Offline preparation captures Codex/Claude startup requests at a localhost mock
provider and checks for planted instruction/skill markers; it checks OpenCode's
resolved configuration. This is a startup/configuration check, not a live recall test.
Before paid timed tasks, each client receives a random private code in one fresh
conversation. A second fresh conversation must answer `NO_MEMORY` when asked for the
previous code. Any leak, tool use, provider error, or unexpected reply blocks the run.
The six canary sessions and their usage are reported separately. This tests automatic
carryover, not hostile attempts to read unrelated host files. Provider prompt caches
remain enabled and are **not** conversational memory.

### Measurements and grading

The report separates uncached input, cache reads, cache writes, output, and additional
Jev input/output usage. Agent totals are explicitly derived from components when
providers omit a total; raw provider fields are retained. Unknown values remain
unavailable. These counts are not dollar costs. Oko search, Jev, startup preparation,
offline warm-up, tool-call counts, and observable model-round counts are recorded
separately; nested durations must not be added together. Codex model-round counts are
unavailable from its current JSON events.

Failed attempts remain in timing/usage summaries. Per-task timing tables use medians
across repetitions, and JSON keeps every paired task/repetition. The legacy hidden-file
anchor now covers the actual wiring line; its lowercase-size edit explicitly allows
updating the error message as well as parser docs. Archived grades remain unchanged.

After a clean interruption, resume with the same suite/cache/repeat/client options:

```sh
python3 scripts/benchmark-public/runner.py --suite branch --execute \
  --resume benchmarks/results/public-branch/results-EXAMPLE
```

Saved infrastructure failures, incomplete canaries, or existing unrecorded sessions
require inspection; they are never automatically retried.

## Retrieval replay: what Oko returns, without agents

Agent sessions are fast only when one Oko call returns all the code a task
needs; otherwise the agent keeps searching. `replay/replay.py` measures that directly.
It collects the distinct questions agents really sent to Oko in earlier
branch-suite runs (about 170), sends each to every build, and scores the share
of the task's expected locations inside one response, plus response size and
Oko/Jev time. No agent sessions run.

```sh
python3 scripts/benchmark-public/replay/replay.py                 # plan and Jev-call count; sends nothing
python3 scripts/benchmark-public/replay/replay.py --no-jev        # free, offline: keyword ranking only
python3 scripts/benchmark-public/replay/replay.py --execute       # paid: one Jev call per question per build
python3 scripts/benchmark-public/replay/replay.py --execute --build old=PATH --build new=target/release/oko
python3 scripts/benchmark-public/replay/selftest.py
```

- Builds default to those frozen by the last branch or smoke `--prepare`; the
  first build is the baseline. `--tasks` and `--limit N` (questions per task)
  reduce cost. Results go to `benchmarks/results/public-replay`.
- Search tasks are scored against their expected anchors, edit tasks against the
  lines the edit must change. An anchor counts only if one excerpt contains all
  of it.
- `--no-jev` checks packaging (excerpt sizes, result counts, response bytes), not
  relevance.
- Jev is not deterministic, so compare means over many questions and the
  difference between gained and lost coverage, not single rows.
- This cannot show how an agent reacts: turns, trust, or final answers. Use the
  smoke or branch suite for that.

## Guided condition: what `oko setup`'s guidance does

Blank-slate sessions strip every project instruction, including the guidance
`oko setup` writes, so they cannot show its effect. `--guided` (branch or smoke)
adds a condition that is the current build plus `src/guidance.md`, delivered
through each client's own channel for standing instructions: `AGENTS.md` in the
trial's `CODEX_HOME`, OpenCode `instructions`, Claude `--append-system-prompt`.
The task prompt differs only in the clause that excludes `AGENTS.md`. The report
pairs `current -> guided`. With the branch suite, `--skip-previous` runs native,
current and guided only, which keeps a full run at 243 sessions.

## Smoke suite: minutes, not hours

The branch suite is 243 sessions. While iterating on a change, use the smoke
suite instead: the same frozen builds, graders, isolation, and report, reduced to
the previous and current Oko builds on four branch tasks with Claude alone
(24 sessions at three repeats).

```sh
python3 scripts/benchmark-public/runner.py --suite smoke --prepare
python3 scripts/benchmark-public/runner.py --suite smoke --check
# Paid calls:
python3 scripts/benchmark-public/runner.py --suite smoke --execute
```

- Native search is not rerun. It does not change between Oko builds; take it
  from the latest branch-suite run.
- Default tasks are `astro-image-probe-authorization` and
  `ripgrep-capture-expansion` (where a build lost expected anchors),
  `astro-action-key-guards` (where Oko clearly helps), and the edit
  `ripgrep-capture-hyphen` as a control. Choose others with
  `--tasks id,id`; add clients with `--clients claude,codex`.
- Builds and the validator dependency are shared with the branch suite under
  `benchmarks/results/public-branch`, so only a new commit is compiled. Results
  go to `benchmarks/results/public-smoke`.
- The two-session memory canary is skipped; the offline isolation preflight
  still runs during `--prepare`.
- These tasks were picked because they separated builds before. The suite shows
  direction and catches regressions; it supports no speed or quality claim.
  Run the branch suite before reporting anything.

## Legacy cold/warm pilot

Compare Codex, OpenCode, and Claude Code without Oko, with cold Oko, and with warm Oko on **12 tasks / 108 sessions**. This is a small navigation-and-edit benchmark, not a general coding leaderboard. No model calls happen unless `--execute` is supplied.

## Tasks

| Repository | Search | Cross-file search | Edit 1 | Edit 2 |
|---|---|---|---|---|
| Astro | Internal URL classification | Request handler → trailing-slash redirect | Strip query at the first `?` | Preserve dotted directories and dotfiles when removing extensions |
| HTTPX | Proxy pattern precedence | Multipart encoder → length/header selection | Handle empty/lone quoted strings | Handle closed file-like objects without raising |
| ripgrep | Size parsing and overflow | CLI hidden flag → traversal ignore matcher | Accept lowercase size suffixes | Convert size parse errors to `InvalidInput` |

Tasks are hypothetical requested changes, not claims that upstream maintainers consider these bugs. They are deliberately small. Several tasks share utility modules, so the suite does not sample every subsystem. Each edit runs on the original baseline, independently of other edits.

The exact questions, reviewed source anchors, reference changes, and full-file hashes are in `tasks.json`. Models receive only the question, condition, and scope instructions—not reference answers. Search tasks require all expected source anchors, not just one matching file. Broad ranges over 120 lines are rejected. These are retrieval tasks, not prose-explanation grading.

## Prepare (no paid calls)

Clone `astro`, `httpx`, and `ripgrep` under `~/dev` at the commits in `tasks.json`. Preparation refuses dirty or different checkouts; it never resets them.

```sh
python3 scripts/benchmark-public/runner.py --prepare
python3 scripts/benchmark-public/selftest.py
python3 scripts/benchmark-public/runner.py
```

Use `--repositories /path/to/parent` if needed. Prerequisites: Python 3.12+, Node supporting `--experimental-strip-types` (tested with 22.14), `rustc`, `rg`, all three authenticated clients, and the built Oko release binary. No package installation, repository build, or external service is needed for the focused validators.

Default models match the previous fast comparisons: Codex `gpt-5.6-sol`, OpenCode `openai/gpt-5.6-sol`, Claude `claude-sonnet-5`, low effort. Preparation accepts `--codex-model`, `--opencode-model`, `--claude-model`, `--effort`, `--timeout`, and `--oko`. Execution uses the frozen settings, not newly supplied model flags. CLI versions, binary hash, task hash, runner hash, and commit hashes are checked before execution. Reprepare after intentional changes.

Preparation executes every edit contract twice: it must fail against original code and pass against the reference change. Reference changes happen in temporary copies only. Test results and immutable source archives are saved under ignored `benchmarks/results/public/`.

## Run (paid calls)

```sh
python3 scripts/benchmark-public/runner.py --execute
```

This starts 108 client sessions: 36 native, 36 cold Oko, and 36 warm Oko. Oko sends selected public source excerpts and search questions to TypeSafe/Jev. The coding clients send context to their configured providers. Oko loads its key through the existing credential-isolating launcher; keys are not written into benchmark configuration. Keep result directories private: client authentication links and raw logs are not suitable for wholesale publication.

Each session gets a fresh disposable full checkout and a fresh agent conversation. The three conditions are:

| Condition | Index preparation | What the timed session includes |
|---|---|---|
| Native | No Oko | Normal agent workflow |
| Cold Oko | Empty dedicated cache | Index creation, MCP startup, Jev requests, agent work |
| Warm Oko | Prebuilt disk index | Index loading/validation, MCP startup, Jev requests, agent work |

Warm-up runs a fixed, task-independent lexical query with `--no-jev` before the agent timer starts. It makes **zero provider calls** and does not precompute the answer to the task. Its duration and cache metadata are saved in `warmup.json` and the aggregate report. A fresh MCP process then loads the same index. This is **warm disk cache**, not a persistent in-memory server or a cached Jev response.

The runner verifies that the first Oko search reports `cold` for the cold condition, and `disk` with zero rebuilt files and positive file reuse for the warm condition. Missing/mismatched metadata blocks the run instead of silently labeling it warm. Subsequent searches in either Oko condition may benefit from in-memory reuse. The offline self-test verifies that an immediate source edit is reflected in a subsequent search after warm-up.

Repositories are interleaved and client order rotates. All six condition-order permutations are used evenly; each condition runs first, second, and third four times per client. Personal skills, memories, and repository agent configuration are excluded using the shared `blank-slate-v1` harness. Native follow-up searches remain allowed. Provider prompt caches are not cleared.

Run only one copy of this benchmark at a time. Do not reprepare while running. No automatic retries of failed or interrupted paid sessions. After a clean interruption between saved sessions:

```sh
python3 scripts/benchmark-public/runner.py --execute --resume benchmarks/results/public/results-EXAMPLE
```

An existing session directory without a matching saved report entry blocks resumption for manual inspection, avoiding duplicate paid calls. `--clients codex` selects a 36-session subset; the default is all three clients.

## Grading and reporting

- Every edit must pass independent executable input/output checks against its actual edited module. Astro loads the full path utility module with Node's type stripping. HTTPX loads the real utility/type modules without importing network transports. ripgrep compiles the real size module with Rust, including its existing unit tests and extra contract checks.
- Alternative implementations can pass; byte-for-byte patch matching is not required. Edits outside the intended function/implementation range or in other files fail scope checks and require review. Agents are instructed not to add tests, build, or install dependencies. Grader code is outside their checkout.
- Timing covers the client process, including MCP startup, model interaction, editing, and its own checks. Checkout, offline warm-up, and independent post-session validation are excluded; validation duration is recorded separately. This is **time to a submitted patch**, not a full production-ready change.
- Reports retain failed attempts and timeouts. Compare success rates alongside speed. `report.json` includes cached/uncached input, cache writes, output, per-task timing, prompts in task fixtures, and raw tool traces in trial directories. Agent tokens exclude Jev; do not label token reductions as cost savings.
- `report.md` includes aggregate and three-condition per-task timings. Warm-up durations are shown separately; `report.json` retains the observed cache metadata for every Oko call. With one run per task, do not claim statistical significance or universal speedups. Publish the frozen configuration, all task results including losses, success rates, and limitations. Redact credentials/local personal data before sharing logs.

`--prepare`, `--execute`, and the self-tests leave the original clones untouched. Generated workspaces are removed after each session; diffs, validation output, timing, and client logs are retained in ignored results directories. No commits or pushes are performed.
