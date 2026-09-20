# Twenty: 10 read-only questions + 5 edits

Every question runs in nine client/condition combinations:
Codex, OpenCode, and Claude Code, each with `native`, `oko-cold`, and `oko-warm`.
One repeat is **135 sequential sessions**. The pilot uses one read and one
backend edit: 18 sessions. The `--fast` preset uses three read-only searches and
two edits: **45 sessions**.
Its searches cover frontend rich-text previews, backend metadata pagination, and
shared locale direction. Its edits change frontend preview debounce and backend
email retry delay. It reuses the frozen questions and grading without shortening
timeouts or changing model settings.

[Questions](tasks.md) cover frontend search, rich text, webhook settings, backend
pagination, Gmail retry handling, webhook routing, email backoff, and shared
formatting/localization. Five edits change three frontend display/timing constants
and two backend retry/batch constants. This is a localized retrieval-and-edit
benchmark, not an evaluation of general feature implementation.

## Run from the Oko repository

Requires Python 3.12+, Git, ripgrep, authenticated `codex`, `opencode`, and `claude`
CLIs on PATH, and a built Oko binary. Preparation performs no model calls and writes
only ignored artifacts under `benchmarks/results/twenty/`.

```sh
python3 scripts/benchmark-twenty/prepare.py ~/dev/twenty
python3 scripts/benchmark-twenty/selftest.py
python3 scripts/benchmark-twenty/runner.py
```

The last command **only prints the plan**. To launch model sessions:

```sh
# Pilot: 18 sessions; paid runs perform this same pilot as a canary first
python3 scripts/benchmark-twenty/runner.py --pilot --execute

# Fast: 3 read-only searches + 2 edits, 45 sessions
python3 scripts/benchmark-twenty/runner.py --fast --execute

# Full benchmark: 135 sessions
python3 scripts/benchmark-twenty/runner.py --execute

# Canary only: no main run after the pilot gate
python3 scripts/benchmark-twenty/runner.py --canary-only --execute
```

Optional subsets and repeats:

```sh
python3 scripts/benchmark-twenty/runner.py --clients codex,opencode --condition both --execute
python3 scripts/benchmark-twenty/runner.py --clients claude --condition native --execute
python3 scripts/benchmark-twenty/runner.py --repeats 3 --execute
```

Omit `--execute` to inspect any preset without model calls. `--fast` and `--pilot`
are mutually exclusive. Fast and full paid runs execute the pilot canary first
and stop without starting the main run when a client, condition, metric, provider
evidence, schema, or privacy check fails. Non-native canary rows must record a
positive Oko call count consistent with the agent tool count and at least one
safe provider call; failed provider calls still count as instrumentation evidence,
and provider-call count is not required to equal Oko MCP-call count. Oko's tool
result is agent-facing source text, so cache state, timings, and provider-call
summaries come from the per-trial `oko-metrics.jsonl` that the launcher selects
with `OKO_METRICS_FILE`; the runner attaches each line to its Oko tool call. Include
`--fast` when resuming a fast run; resume also
requires a passed canary. Run timed benchmarks one at a time so competing
sessions do not distort latency.

## Frozen inputs and models

The committed fixture targets Twenty commit
`a0a55e326b3963d10b7843f65ba90e558deed7da`. Preparation refuses another commit,
a dirty checkout, or changed target hashes. To benchmark a newer revision, review
and update the questions, expected ranges/patches, commit, and hashes in
`tasks.json` before preparing again. Never regenerate expected answers from agent
outputs.

The default models are `gpt-6-astra` in Codex,
`openai/gpt-6-astra` in OpenCode, and `claude-opus-5[1m]` in Claude Code, all at
medium effort. Override with `prepare.py --codex-model ... --opencode-model ...
--claude-model ...`; use `--oko /path/to/oko` or `--timeout 300` as needed.
Model access is checked by a live session, not by the offline tests. CLI versions,
archive/fixture/binary hashes, and requested models are recorded. Execution rejects
version or input drift. Do not re-prepare while a benchmark is running.

## Execution and results

Each session gets a fresh full-repository checkout from the frozen archive and
its own Git baseline. `native` disables Oko. `oko-cold` uses a fresh cache and
requires observed cache status `cold`. `oko-warm` prebuilds the exact disposable
workspace/cache before the timed agent process, then requires observed status
`disk`, zero rebuilt files, and at least one reused file; `memory` does not count
as warm. Client/condition order rotates between questions and repeats. The
original Twenty checkout is never used as an agent working directory.

The Oko subprocess alone receives the Oko credential, from Oko's `.env` or its
normal OS credential store. The parent strips `TYPESAFE_*` and `OKO_*` variables
from agent environments. No credentials are written into benchmark settings.

Agents may read/search and, for edit tasks, change the disposable checkout. They
are instructed not to run tests/builds, install dependencies, access live product
services, delegate, or use the web. Codex uses its read-only/workspace-write
sandbox; OpenCode uses a restricted tool-permission agent; Claude uses restricted
mode with explicit read/edit tools and strict MCP configuration. These mechanisms
are not identical; OpenCode permissions are not an OS sandbox. Repository
instructions remain part of the task context.

Each run creates `benchmarks/results/twenty/results-*/report.json` and `report.md`.
Local report groups are separated by client, requested model, requested effort,
requested condition, observed cache state, and task kind; changing the model or
effort never silently merges sessions.
Per-session artifacts include raw events, stderr, final response, usage counters,
Oko/tool call counts, grading, patches, and untracked file contents. Treat raw logs
as local artifacts. Disposable workspaces are removed after artifacts are saved. Cleanup failures are recorded as warnings and retain the workspace without discarding a completed session.

Every run also writes `shareable/benchmark.json`, a single self-contained
artifact in the versioned `oko-benchmark/v1` format. It includes the manifest,
reproducibility settings, canary result, records, summary, and report text.
Upload **only `shareable/benchmark.json`**. The adjacent
`run-manifest.json`, `records.jsonl`, `summary.json`, and `report.md` are local
compatibility views. The artifact is privacy- and schema-validated before it is
written and never copies raw events, stderr, prompts, source bodies, tool
arguments, evidence, credentials, headers, or absolute paths. Agent usage and
Oko/Jev usage remain separate; partial provider usage remains partial, and a
total is `null` unless the provider explicitly reports it.

Future runners can import `scripts/benchmark_observability.py` and call
`perf_counter_ns()`, `make_record()`, and `write_bundle()`; the concise adoption
example and schema references are in [`benchmarks/oko-benchmark/v1/README.md`](../../benchmarks/oko-benchmark/v1/README.md).
Source-checkout state and frozen input hashes are rechecked at the end.

Read questions are graded by top-one/top-five overlap with source-verified line
ranges; malformed answers and any source edits are errors. Edit questions use
exact patch matching; alternative formatting or extra files require review,
not an automatic claim of semantic failure. Application correctness is **not**
established by this grader. Infrastructure failures/timeouts stop execution, preserve partial
reports, and return a nonzero exit code. Invalid answers remain recorded as misses;
remaining questions continue. There are no hidden retries. Completed runs
can still contain misses or edits needing review.

Compare Oko on/off **within each client**. Claude uses a different model, system
prompts and tools differ, and provider caches are not cleared. Timing includes
agent process startup and model/tool latency but excludes checkout/grading. Raw
provider token counters are retained; Jev tokens/cost are not included in agent
token totals. Preparation and offline tests do not establish live model or MCP
compatibility; use the pilot before interpreting a full run.

To continue after a runner interruption (using the same pilot/client/condition/repeat options):

```sh
python3 scripts/benchmark-twenty/runner.py --execute --resume benchmarks/results/twenty/results-EXAMPLE
```

Resume recovers saved completed session artifacts, including invalid answers, including a result written before the aggregate report was updated. It never silently retries a failed model session. Infrastructure failures require investigation before starting another run. Git automatic maintenance is disabled for disposable checkout creation so background housekeeping does not race cleanup.

## Blank-slate agent runs

All presets use `blank-slate-v1`: personal skills, instructions, hooks, plugins,
and repository agent configuration are excluded. The original repository and
user settings are never edited. Disposable checkouts omit `.agents`, `.codex`,
`.claude`, `.opencode`, instruction files, `SKILL.md`, and harness config files
before creating the grading baseline; `isolation.json` records removed paths.

Codex uses a dedicated per-conversation CLI state directory, links only existing
file-based login state, disables host skill discovery and discovered skill paths,
and sets the project instruction budget to zero. OpenCode uses an empty global
configuration directory, skips project/external skill discovery, and disables the
skill tool. Claude uses restricted settings, excludes instruction/rules paths,
disables skills/commands, hooks, and auto-memory, and allows only explicit MCP.
Built-in system prompts, authentication, and enforced platform policies remain.

Old conversations cannot be resumed into this policy. Start a fresh run; cache
comparisons still resume their own three turns within the new isolated session.
Provider prompt caching and model-response variability are not eliminated.

Verify the installed clients without paid model calls:

```sh
python3 scripts/benchmark-twenty/isolation-smoke.py
```

This sends Codex/Claude startup requests only to a localhost mock provider and
checks OpenCode's resolved configuration. Planted instruction/skill sentinels
must be absent. Rerun this check after upgrading any CLI. The built-in Codex
skill catalog is disabled too; the built-in coding system prompt remains.
