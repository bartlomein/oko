# Public-repository benchmark results

[← Back to Oko](../README.md#benchmarks)

This pilot measured complete coding-agent sessions on 12 tasks: two searches and
two small edits in each of Astro, HTTPX, and ripgrep. Three clients each ran every
task without Oko, with cold Oko, and with warm Oko: **108 sessions**, one observation
per task/client/condition. It is a small pilot, not evidence of universal speedups
or statistical significance.

## Results

Each cell totals the same 12 tasks. The README divides these totals by 12 to
show arithmetic means, not medians. Percentage changes use unrounded values.

| Client | Without Oko | Warm Oko | Cold Oko |
| --- | --- | --- | --- |
| Codex — seconds | 333.0 | 308.3 | 346.3 |
| Codex — agent tokens | 912,258 | 772,028 | 732,875 |
| OpenCode — seconds | 308.8 | 253.3 | 284.2 |
| OpenCode — agent tokens | 429,260 | 231,021 | 235,482 |
| Claude Code — seconds | 131.5 | 109.0 | 109.0 |
| Claude Code — agent tokens | 349,110 | 309,259 | 279,287 |

All attempts are included, including automated grading failures. Token counts
include cached input and exclude Jev usage. Provider billing differs across
cached input, uncached input, cache writes, and output: token reductions are not
cost savings. Compare conditions within each client, not model quality across clients.

## Configuration and tasks

| Client | Version | Requested model | Effort |
| --- | --- | --- | --- |
| Codex | 0.155.0 | `gpt-5.6-sol` | low |
| OpenCode | 1.18.31 | `openai/gpt-5.6-sol` | low |
| Claude Code | 2.1.278 | `claude-sonnet-5` | low |

Oko used Jev `jev-1.13.0`. The tested development binary predates the `v0.2.1`
version bump; its SHA-256 and the frozen repository, task, and implementation hashes
are recorded in the [sanitized per-session data](../benchmarks/public-pilot.json).
This was not a benchmark of the downloaded release archives.

The [task manifest](../scripts/benchmark-public/tasks.json) contains exact prompts,
source commits, expected search anchors, and edit contracts. Tasks cover URL/path
utilities and redirects in Astro, proxy/multipart utilities in HTTPX, and size
parsing and hidden-file traversal in ripgrep. These are hypothetical small edits;
several share utility modules and do not represent every subsystem.

## Timing and isolation

Each session started with a fresh disposable checkout and agent conversation.
Personal skills, memories, and repository agent configuration were excluded by
the runner's `blank-slate-v1` policy. Oko sessions were instructed to search with
Oko first; native follow-up tools remained available. Native sessions had no Oko calls.

Timing covers the client process, MCP startup, provider interaction, searches,
and edits. Checkout preparation and independent post-session grading are excluded.
For edits, this is time to a submitted patch, not a fully validated production change.

Cold Oko started with an empty dedicated cache. Warm Oko loaded a prebuilt disk
index into a fresh MCP process. Warm-up used a fixed task-independent lexical
query, made no provider requests, and was excluded from timing. Median excluded
warm-up was 0.915 seconds for Astro, 0.042 for HTTPX, and 0.095 for ripgrep.
All 72 Oko sessions matched the required initial cache state.

Warm does not mean cached Jev answers or an already-running MCP server. Subsequent
queries could reuse in-memory state in either Oko condition. Provider prompt
caches were not cleared. Condition order was balanced: each condition appeared
first, second, and third four times per client. Timing differences also reflect
model/tool choices; the cold-to-warm difference cannot be attributed entirely to caching.

## Correctness and limitations

The frozen automated grader passed **92/108** sessions. All 16 flagged sessions
were reviewed; original automated grades are preserved in the published data:

- Eight search responses contained the correct hidden-file wiring and matcher,
  but did not include unrelated neighboring lines required by an overly broad anchor.
- One search response contained the correct locations but included prose outside
  the required JSON. That response-format violation remains.
- Seven size-suffix edits also updated the related error message outside the
  frozen allowed scope. Each saved patch was reapplied to a clean source copy and
  passed independent executable checks.

After source review, all 54 search responses contained the required evidence.
All 54 edits passed focused behavior checks, including seven post-run checks.
These are module-level checks, not full application tests; manual review is not
equivalent to 108 automated passes. No model sessions were repeated to repair grades.

One MCP launcher startup failed before any provider call. The launcher was fixed
and checked offline; the completed native session was retained without rerunning it.
Original repositories and frozen source artifacts were unchanged.

## Reproduce

Follow the [runner instructions](../scripts/benchmark-public/README.md) for
prerequisites, pinned checkouts, preparation, and execution. Running the full suite
starts 108 paid agent sessions; the default command only prints the plan.

The [published data](../benchmarks/public-pilot.json) includes every session's
duration, token breakdown, original automated pass/fail, and cache verification,
plus sanitized model/version/hash metadata. Raw provider logs, local paths,
authentication links, and generated source snapshots are not published. The
manual review findings are summarized above; raw traces and patches are not part
of this public dataset, so it does not independently reproduce that review.
