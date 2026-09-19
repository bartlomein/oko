# Public-repository pilot

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
