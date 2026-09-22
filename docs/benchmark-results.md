# Benchmark results

[← Back to Oko](../README.md#benchmarks)

Three benchmarks: two public retrieval benchmarks that score what Oko returns,
and our own agent benchmark that times whole coding sessions. Raw per-task
results for the retrieval benchmarks are in
[`benchmarks/published/0.5.0/`](../benchmarks/published/0.5.0/); the harnesses
and commands are in [the runner README](../scripts/benchmark-public/README.md#retrieval-benchmarks).

## Agent Retrieval Bench

[Agent Retrieval Bench](https://arxiv.org/abs/2607.24882) (code:
[eyuansu62/agent-retrieval-bench](https://github.com/eyuansu62/agent-retrieval-bench))
has 345 positive tasks from 25 repositories in Python, Go, Rust, TypeScript,
Java, and JavaScript. Each gives a repository at a commit and a signal from a
coding workflow, and asks for the files a developer needs next:

- **Failing test → broken file** (101 tasks): a test command and its failure
  output; find the source file at fault.
- **Review comment → context** (80): a review comment on a pull request; find
  the files the reviewer is pointing at.
- **Pull request → tests to update** (106): a pull request's description and
  changed files; find the tests that need updating.
- **Edit → files it ripples into** (58): a code change and its intent; find the
  other files that must change with it.

**Method.** The harness (`scripts/benchmark-public/replay/arb.py`) rebuilds each
repository from the benchmark's released corpus, so Oko searches exactly the
files the published baselines searched, sends the task's signal to Oko's
`search` tool once (trimmed to Oko's 4,096-byte question limit; 47 of 345 are
trimmed), and scores the ranked list, Oko's excerpts followed by every judged
candidate by relevance, with the benchmark's own metric code at commit
`07014c98`. We ran the benchmark's RepoMap, lexical, and BM25 baselines locally
on the same tasks; they reproduce the published numbers within 0.003. The
embedding rows below are the published results; we did not rerun them.

**What was tuned on what.** We split the tasks by a hash of their id into a
164-task dev half and a 181-task held-out half, tuned on the dev half only, and
ran the held-out half once when the design was frozen (experiment build:
Recall@20 0.61, MRR 0.31 on the held-out half; 0.61 / 0.34 on all 345). Two
changes came after that: connected files are judged by their own criteria, and a
question that asks for tests no longer has tests excluded. Both were chosen on
other data, the dev half and our own repositories, and then measured here. The
second change lifts the "tests to update" task a lot (Recall@5 0.14 → 0.38),
because 37 of its 46 dev questions carry the benchmark's own summary line
"N existing test files changed"; Oko reads that as a question about tests. That
is a fair reaction to the text, but it leans on this benchmark's wording, so we
say so. Without that task type the three-run mean is unchanged from the frozen
first look.

**Stability.** The 0.5.0 build was run three times on all 345 tasks with
`jev-1.13.0`. The runs agree within 0.005 on every ranking metric and 0.013 on
BCY@8k; every number below is the mean of the three.

### All 345 tasks

| Method | Recall@5 | Recall@20 | MRR | BCY@8k |
| --- | ---: | ---: | ---: | ---: |
| **Oko 0.5.0** | **0.45** | 0.64 | **0.39** | **0.48** |
| Qwen3-Embedding-8B (published) | | **0.70** | 0.23 | 0.37 |
| RepoMap | 0.32 | 0.64 | 0.22 | 0.38 |
| Qwen3-Embedding-4B (published) | | 0.63 | 0.24 | 0.34 |
| pplx-embed-v1-4b (published, provisional) | | 0.61 | 0.23 | 0.35 |
| nomic-embed-code (published) | | 0.52 | 0.20 | 0.28 |
| Lexical | 0.23 | 0.49 | 0.16 | 0.27 |
| jina-code-embeddings-0.5b (published) | | 0.48 | 0.19 | 0.28 |
| BM25 | 0.19 | 0.45 | 0.15 | 0.21 |

Recall@k is the share of a task's needed files among the first k ranked. MRR is
the reciprocal rank of the first needed file (1.0 = always first). BCY@8k is the
benchmark's budgeted context yield: the share of needed files whose text fits
when ranked files are packed into 8,000 tokens; Oko leads it at every budget
(4k: 0.39 against RepoMap's 0.20; 32k: 0.64 against 0.63).

Against RepoMap on the same 345 tasks, Oko's MRR lead is +0.17 with a 95%
bootstrap range of [+0.13, +0.22] by task and [+0.06, +0.27] with each
repository treated as one unit; Recall@5 +0.13, [+0.07, +0.19] by task; Recall@20
is a tie, [−0.05, +0.06].

### By task type, and by split

| Task | n | Oko R@5 | RepoMap R@5 | Oko R@20 | RepoMap R@20 | Oko MRR | RepoMap MRR |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Failing test → broken file | 101 | 0.71 | 0.46 | 0.82 | 0.85 | 0.67 | 0.27 |
| Review comment → context | 80 | 0.31 | 0.19 | 0.53 | 0.50 | 0.32 | 0.16 |
| Pull request → tests to update | 106 | 0.38 | 0.26 | 0.57 | 0.56 | 0.26 | 0.20 |
| Edit → files it ripples into | 58 | 0.28 | 0.36 | 0.63 | 0.60 | 0.22 | 0.23 |
| All 345 | 345 | 0.45 | 0.32 | 0.64 | 0.64 | 0.39 | 0.22 |
| Held-out half | 181 | 0.45 | 0.34 | 0.65 | 0.64 | 0.37 | 0.21 |
| Dev half | 164 | 0.44 | 0.30 | 0.64 | 0.63 | 0.41 | 0.22 |
| Without gin-gonic/gin | 257 | 0.33 | 0.27 | 0.54 | 0.55 | 0.29 | 0.19 |

Oko is strongest when the signal is an error message and weakest on the ripple
task, where RepoMap's import graph helps and Oko's one-hop links do not reach
far enough. One repository, gin-gonic/gin, contributes 88 of the 345 tasks (the
benchmark's own README notes this); without it Oko still leads MRR and Recall@5
and RepoMap leads Recall@20.

**Limits.** Scoring is per file: Oko returns functions, and gets no credit here
for the exact function. The queries are raw logs, review comments, and pull
request text, not the questions an agent would ask. Oko showed no excerpt on
about a third of the tasks (nothing reached its relevance cutoff); the ranked
list is still scored, but an agent would have to open the listed candidates.
No GPU or index is involved; each Oko search made three Jev requests.

## SWE-Explore

[SWE-Explore](https://arxiv.org/abs/2606.07297) (code:
[Qiushao-E/SWE-Explore-Bench](https://github.com/Qiushao-E/SWE-Explore-Bench))
has 848 real issues from SWE-bench Verified (451), SWE-bench Pro (215), and
SWE-bench Multilingual (182), across 64 repositories in ten languages. The
answer for each issue is the code that successful agents read while fixing it,
as line regions (4.3 files and 4.7 regions per issue on average), and an
explorer returns five ranked regions. We ran the benchmark's scorer at commit
`9281148b` on the 0.5.0 build, once, and its own BM25 and TF-IDF explorers on the
same inputs; ours match the paper's Table 6 within 0.01. Agent rows are the
paper's. Nothing was tuned on this benchmark.

| Method | Right file in top 5 | Right region in top 5 | Line precision | Line recall | nDCG@500 | First useful hit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Claude Code (agent, published) | 0.67 | | 0.60 | | 0.94 | |
| Mini-SWE-Agent (agent, published) | 0.64 | | 0.53 | | 0.89 | |
| LocAgent (agent, published) | 0.54 | | 0.64 | | 0.95 | |
| CoSIL (agent, published) | 0.54 | | 0.58 | | 0.82 | |
| **Oko 0.5.0, one call** | 0.41 | 0.34 | 0.52 | 0.15 | 0.81 | 0.84 |
| AutoCodeRover (agent, published) | 0.28 | | 0.68 | | 0.72 | |
| Oko, keyword ranking only | 0.21 | 0.16 | 0.19 | 0.05 | 0.36 | 0.41 |
| TF-IDF (benchmark's explorer) | 0.14 | 0.11 | 0.10 | 0.04 | 0.22 | 0.23 |
| BM25 (benchmark's explorer) | 0.07 | 0.06 | 0.05 | 0.02 | 0.12 | 0.13 |

The agents explore for many turns with a frontier model; Oko is one search of
about a second with no model. On ranking (nDCG, first useful hit) and line
precision Oko sits in the agents' tier; on covering all of an issue's files it
does not, because five regions from one call cannot reach 4.3 files. Its first
region is in a right file 83% of the time. Ordering the five slots so that each
names a different file raises "right file" to 0.50 but halves precision, since
the second to fifth files are usually wrong; we report the by-score ordering.

By source: SWE-bench Verified 0.46 right file; Multilingual 0.42 (where the
benchmark's BM25 and TF-IDF score near zero, because their chunker stops at
3,000 chunks per repository); Pro 0.29, with the highest line precision (0.55).
Raw rows for every run are in
[`benchmarks/published/0.5.0/swe-explore/`](../benchmarks/published/0.5.0/swe-explore/).

### A deeper search was measured and dropped

A second round for multi-line questions was built and measured on all 848
issues against the 0.5.0 build, in three forms, and none of them helped, so
none shipped. Letting the further keyword matches that Jev rates relevant
join the excerpts (no extra request): right file 0.501 against 0.503, nDCG
0.825 against 0.821, first useful hit 0.857 against 0.855, one region per
file. Judging every candidate beside the shortlist as "related code" and
showing what passes: worse on every metric (right file 0.476). A further
round over up to ninety files linked to the matches by import, definition,
test name or directory, judged against the question: nDCG 0.860 but the same
files found, for one to three more requests and a second of latency; of
33,000 linked files judged, 758 scored as relevant, and those reaching the
top five were right 14–19% of the time against 50% for first-round files.
Judging the connected files against the question also lowered Agent
Retrieval Bench (dev half, Recall@20 0.649 to 0.636) because tests and
callers stop being listed. The code is on the `feat/deep-search` branch;
the rows are in `benchmarks/results/swe-explore/` for the runs named
`deep-*`. A 120-issue sample of the same runs showed gains of 0.03 in nDCG
that the full set did not; decide on the full set.

## Agent sessions

## Latest run: 243 sessions, Oko 0.5.0 (September 2026)

The README reports this run. Nine tasks (six code-location questions and three
small edits across Astro, HTTPX, and ripgrep), three clients, three repeats, and
three setups: **without Oko**, **Oko**, and **Oko with the guidance `oko setup`
installs**. The README reports the first and last
(162 of the 243 sessions), since `oko setup` installs the guidance; the middle one
is what a manual connection without the guidance gets. Oko starts with a
warm disk index; building it is excluded from timing. Measured on the 0.5.0
build (commit `580fe6f`; the run's own record names `b445927e`, the same code
with documentation edits), against the same tasks and clients as the 0.4.0 run
below it. Pass rates: Codex 27/27, 27/27, 26/27; OpenCode 27/27 in all three;
Claude Code 22/27 in all three, the same five task-and-format failures in each
setup (the astro-forwarded-empty edit touching a duplicate definition, and
answers the grader rejected for their JSON format).

Mean seconds, agent tokens, and tool calls per session (27 sessions per cell):

| Client | Without Oko | Oko | Oko + guidance |
| --- | --- | --- | --- |
| Codex — seconds | 33.7 | 32.8 | 29.8 |
| Codex — agent tokens | 63,828 | 53,809 | 51,025 |
| Codex — tool calls | 3.11 | 2.52 | 2.19 |
| OpenCode — seconds | 32.4 | 27.7 | 25.7 |
| OpenCode — agent tokens | 29,712 | 17,859 | 16,299 |
| OpenCode — tool calls | 5.96 | 3.30 | 2.33 |
| Claude Code — seconds | 12.4 | 8.9 | 7.8 |
| Claude Code — agent tokens | 60,320 | 43,213 | 36,948 |
| Claude Code — tool calls | 5.15 | 3.00 | 2.26 |

Absolute times and tokens are higher than in the 0.4.0 run below (the model
providers were slower and Claude Code sessions used more tokens without Oko on
this day); the relative gains are what to compare. The 0.4.0 run, on commit
`c8934a0`:

| Client | Without Oko | Oko | Oko + guidance |
| --- | --- | --- | --- |
| Codex — seconds | 25.7 | 23.2 | 21.0 |
| Codex — agent tokens | 71,923 | 54,356 | 48,823 |
| Codex — tool calls | 3.41 | 2.48 | 2.04 |
| OpenCode — seconds | 22.8 | 23.0 | 19.3 |
| OpenCode — agent tokens | 29,413 | 21,912 | 17,017 |
| OpenCode — tool calls | 5.81 | 3.89 | 2.56 |
| Claude Code — seconds | 10.6 | 8.1 | 7.3 |
| Claude Code — agent tokens | 23,960 | 19,710 | 19,133 |
| Claude Code — tool calls | 3.67 | 1.96 | 1.59 |

Without the guidance, Oko saves Codex and OpenCode tokens but little or no time;
the guidance, which tells the agent when it can stop searching, is what makes
those sessions faster.

Per task, Oko + guidance against without Oko (mean of three sessions; time, tokens):

| Task | Codex | OpenCode | Claude Code |
| --- | --- | --- | --- |
| astro-image-probe-authorization | −54%, −69% | −53%, −80% | −45%, −37% |
| astro-action-key-guards | −49%, −54% | −18%, −51% | −42%, −27% |
| astro-forwarded-empty (edit) | +11%, −32% | −18%, −43% | −16%, −18% |
| httpx-decoder-chain | +5%, +19% | +33%, +58% | −28%, −10% |
| httpx-async-auth-body | −23%, −34% | +13%, +0% | −39%, −30% |
| httpx-reason-fallback (edit) | −19%, +5% | −20%, −47% | −9%, −2% |
| ripgrep-capture-expansion | −9%, −36% | −37%, −49% | −52%, −46% |
| ripgrep-printed-bytes | −21%, −2% | −3%, −15% | −33%, +11% |
| ripgrep-capture-hyphen (edit) | +3%, −27% | −11%, −51% | −28%, −17% |

Mean tool calls per task, without Oko → Oko + guidance. An Oko search counts as
one call. They fell on every task for every client:

| Task | Codex | OpenCode | Claude Code |
| --- | --- | --- | --- |
| astro-image-probe-authorization | 4.7 → 1.0 | 7.7 → 1.0 | 4.3 → 1.0 |
| astro-action-key-guards | 3.7 → 1.3 | 6.0 → 2.7 | 3.3 → 1.0 |
| astro-forwarded-empty (edit) | 3.0 → 2.7 | 5.3 → 3.0 | 4.0 → 2.0 |
| httpx-decoder-chain | 3.7 → 2.7 | 5.7 → 4.3 | 4.0 → 2.0 |
| httpx-async-auth-body | 2.7 → 1.0 | 5.0 → 3.0 | 3.0 → 1.0 |
| httpx-reason-fallback (edit) | 3.0 → 2.0 | 4.0 → 2.0 | 2.3 → 2.0 |
| ripgrep-capture-expansion | 3.3 → 2.7 | 9.0 → 2.0 | 4.0 → 1.0 |
| ripgrep-printed-bytes | 3.7 → 3.0 | 5.7 → 3.0 | 4.0 → 2.3 |
| ripgrep-capture-hyphen (edit) | 3.0 → 2.0 | 4.0 → 2.0 | 4.0 → 2.0 |

Automated grading passed every Oko + guidance session (81/81), 79/81 with Oko
alone, and 75/81 without Oko. All attempts count toward the times and tokens.
Token counts include cached input and exclude Jev. The guidance reaches each
client as user-level instructions (Codex `AGENTS.md`, OpenCode `instructions`,
Claude Code `--append-system-prompt`) so the graded checkout stays unmodified;
`oko setup` writes the same text to the project's `AGENTS.md` or `CLAUDE.md`.
Clients: Codex CLI 0.155.0 and OpenCode 1.18.31 with `gpt-5.6-sol`, Claude Code
2.1.278 with `claude-sonnet-5`, all at low reasoning effort. Three repeats of
nine tasks is still small: read direction, not decimals.

## Earlier pilot: 108 sessions

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
