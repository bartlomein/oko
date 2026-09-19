# Documenso fast benchmark

**3 read-only searches + 2 edits**, across Codex, OpenCode, and Claude Code, each
with Oko off/on: **30 sequential sessions**. The default is already the fast suite;
`--fast` is an alias for the same five tasks. [Read the questions](tasks.md).

## Run from the Oko repository

```sh
# Freeze the clean checkout and local CLI/binary versions; no model calls.
python3 scripts/benchmark-documenso/prepare.py ~/dev/documenso

# Offline fixture, grading, plan, and adapter tests.
python3 scripts/benchmark-documenso/selftest.py

# Inspect the 30-session plan; no model calls.
python3 scripts/benchmark-documenso/runner.py

# Run the benchmark.
python3 scripts/benchmark-documenso/runner.py --execute
```

The fixture is pinned to Documenso commit
`e658cc581878f52c03b3e6a8f7ffd613aead4e69`. Preparation rejects checkout/target drift
and never resets or edits the source repository. Requires Python 3.12+, Git,
ripgrep, the three authenticated CLIs on PATH, and a built `target/release/oko`.
Use `prepare.py --oko /path/to/oko` to select another binary. Do not prepare again
while a run is active; preparation replaces this suite's frozen snapshot/settings.

The searches cover PDF page counting, next-recipient selection, and recipient
initials. The edits change document-search debounce from 500 to 300 ms and the
default first reminder delay from five to three days. The agent receives only
the question and benchmark instructions, never expected paths or patches.

## Options

### Small Sol / low-effort comparison

One read (`pdf-page-count`) and one edit (`document-search-delay`), each with Oko
off/on. Codex and OpenCode request Sol; Claude Code requests Sonnet 5. All use
low effort. Sonnet is a separate model, not a claim of equivalent capability.

```sh
python3 scripts/benchmark-documenso/prepare.py ~/dev/documenso \
  --codex-model gpt-5.6-sol --opencode-model openai/gpt-5.6-sol \
  --claude-model claude-sonnet-5 --effort low

# Inspect the 12-session plan; no model calls.
python3 scripts/benchmark-documenso/runner.py --pilot

# Launch only when ready.
python3 scripts/benchmark-documenso/runner.py --pilot --execute
```

Add `--clients codex,opencode` to either runner command for eight sessions.
Preparation pins the selected Oko binary, including the current local rebuild.
This pilot retains a fresh cache per session: it measures first-use performance,
not the warm-cache speedup of a long-lived MCP server. Search correctness, exact
edit patches, elapsed time, and agent token usage are recorded independently.

```sh
# One read + one edit across all six conditions: 12 sessions.
python3 scripts/benchmark-documenso/runner.py --pilot --execute

# Restrict clients or conditions.
python3 scripts/benchmark-documenso/runner.py --clients codex,claude --condition both --execute

# Repeat each question/condition three times: 90 sessions.
python3 scripts/benchmark-documenso/runner.py --repeats 3 --execute

# Resume with the same selection/repeat options, without repeating saved sessions.
python3 scripts/benchmark-documenso/runner.py --execute --resume benchmarks/results/documenso/results-EXAMPLE
```

## Cold, warm, and post-edit conversations

The separate cache-session runner uses **six conversations and 18 turns**:
Codex, OpenCode, and Claude Code, each with Oko off/on. It uses the Sol/Sonnet
low-effort settings prepared above and the currently frozen Oko binary.

```sh
# Preview only: no model calls.
python3 scripts/benchmark-documenso/cache-session.py

# Test the bridge and resume arguments offline, using the release binary.
python3 scripts/benchmark-documenso/cache-session-selftest.py

# Explicitly launch the six conversations.
python3 scripts/benchmark-documenso/cache-session.py --execute
```

Each conversation asks the PDF page-count question, then the different
next-recipient question, then requests the document-search debounce edit.
After saving the edit, the agent must search again and verify the new value.
Each condition has its own disposable checkout; the original stays untouched.

The CLIs resume the exact conversation ID across three invocations. A private
local Unix socket keeps one Oko process alive across those invocations. The
runner records that process's calls, source evidence, target-file hash at each
call, cache validation mode, and ranking timings. It rejects a missing warm
memory hit or absent post-edit search returning the updated source. Periodic
full verification is reported honestly; retaining preparation does not always
mean zero content reads. This bridge requires local Unix-socket support.

Reports under `benchmarks/results/documenso/cache-session-*/` include per-turn
seconds, raw CLI token usage, correctness, cache/Jev timings, and a Markdown
comparison table. CLI startup is included; checkout and initial bridge startup
are separate from turn timing. There is one sample per phase/condition, so this
is a smoke comparison, not a statistical performance claim. Compare Oko on/off
within each phase: cold and warm questions differ in difficulty. Agent conversation
and provider prompt caches also affect later turns. Token counter scope differs
by CLI; the report preserves raw usage and does not total it across turns.

Conversations persist in each CLI's local storage to support resume. Agent
or bridge execution failures stop the run without retries and preserve partial
artifacts. Completed turns with formatting, answer, or post-edit retrieval misses
are recorded as failed checks while the remaining conditions continue.
`--resume benchmarks/results/documenso/cache-session-EXAMPLE --execute` skips
saved conditions without repeating calls. A native conversation stopped after
read-only turns by the older strict grader can restore the identical checkout
and resume its remaining turns. An interrupted Oko conversation cannot resume
its in-memory server and is not silently restarted as a warm run.
Do not run another benchmark or rerun preparation concurrently. Add
`--clients codex,opencode` to restrict the runner to four conversations.

## Independent-session results and limits

Artifacts live under `benchmarks/results/documenso/`, separate from Twenty.
Each run saves `results-*/report.json`, `report.md`, and per-session logs, usage,
patches, and grades. Each session uses a disposable full-repository checkout and
a fresh Oko cache. If the source has `.opencode/` without a `.gitignore`, the
runner seeds the observed OpenCode setup gitignore before committing each
disposable baseline, consistently across all clients. Later modifications still
fail normal grading. Results survive cleanup failures, which are recorded as warnings.

This runner reuses the [Twenty benchmark engine](../benchmark-twenty/README.md),
including its tested CLI adapters, grading, timeout handling, and resume behavior.
Changes to the shared engine should pass both suites' self-tests.

The default models match the existing benchmark: Codex `gpt-6-astra`, OpenCode
`openai/gpt-6-astra`, and Claude Code `claude-opus-5[1m]`, all at medium effort.
Preparation accepts `--codex-model`, `--opencode-model`, `--claude-model`, and
`--effort` (`low`, `medium`, or `high`), and `--timeout` overrides. Offline tests do not establish current model access or
live MCP compatibility.

Compare Oko on/off within each client. Read questions use top-one/top-five source
range overlap; invalid answers count as errors and the next session proceeds.
Edits use exact patch matching; alternative patches need review. These are
localized tasks, not broad feature work or application correctness tests. No app
builds, dependency installs, database services, or E2E tests are run.

Infrastructure failures stop the run and preserve partial reports. Provider
caches are not cleared; token counters differ across clients and do not include
Jev usage. Timing includes model/tool latency but excludes checkout/grading.
Run benchmarks one at a time to avoid contention. The original checkout remains
outside the agents' working directories and its state is checked before/after.

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
