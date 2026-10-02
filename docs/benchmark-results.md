# Benchmark results

[← Back to Oko](../README.md#benchmarks)

Four benchmarks: two public retrieval benchmarks that score what Oko returns,
our own agent benchmark that times whole coding sessions, and Sense's agent
benchmark, run against Sense on the same evening. Raw per-task results are in
[`benchmarks/published/0.7.0/`](../benchmarks/published/0.7.0/) (and
[`0.6.1/`](../benchmarks/published/0.6.1/), [`0.6.0/`](../benchmarks/published/0.6.0/)
and [`0.5.0/`](../benchmarks/published/0.5.0/) for earlier releases; Sense's
earlier paired run is in `0.6.0/`); the
harnesses and commands are in [the runner README](../scripts/benchmark-public/README.md#retrieval-benchmarks).

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
`search` tool once (trimmed to Oko's 4,096-byte question limit; 60 of 345 are
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

**Since 0.5.0.** Nothing in 0.6.0 was tuned on this benchmark: its changes
were chosen on our own replay corpus and agent sessions, then measured here
once. The held-out half did slightly worse than the dev half this time (see
the split rows below), which is the direction a benchmark-tuned change would
not produce.

**Since 0.6.0.** 0.6.1 changes what Jev sees of each candidate: the first 15
get more of their code, import lines no longer count as evidence, and a section
of several definitions is shown on the one that matches. These were chosen on
our replay corpus and agent sessions, not on this benchmark. Top-20 recall
fell from 0.70 to 0.68: files the ranker already rated low (Jev scores of
about 0.1–0.4) fell just past rank 20, mostly on failing-test tasks. 0.6.0
rerun two days later scored 0.690, so about half of the gap is run-to-run and
day-to-day variation. Giving the later candidates 0.6.0-sized previews again
won back part of the failing-test loss (0.86 → 0.88 in one run) but left the
overall figure near 0.685 and made agents' first answers slightly worse, so
0.6.1 does not include it.

**Since 0.6.1.** Nothing in 0.7.0 changes how a search ranks: the prompt hook
is a separate path, and these tasks are single searches. The three 0.7.0 runs
match 0.6.1 within run-to-run variation. The largest difference is Recall@5 on
the 58 edit-ripple tasks (0.32 → 0.29); BCY@8k is 0.494 against 0.501, with the
0.7.0 runs between 0.489 and 0.502.

**Stability.** The 0.7.0 numbers are the mean of three runs of commit
`da83b98` on all 345 tasks with `jev-1.13.0` (later commits change only setup,
metrics, docs and version numbers). The runs agree within 0.007 on every
ranking metric and 0.013 on BCY@8k. The 0.6.1 rows are the mean of three runs
of commit `5f3afad`, the 0.6.0 rows of commit `3f7809b`, and the 0.5.0 rows of
that release's three runs.

### All 345 tasks

| Method | Recall@5 | Recall@20 | MRR | BCY@8k |
| --- | ---: | ---: | ---: | ---: |
| **Oko 0.7.0** | 0.44 | 0.68 | 0.37 | 0.49 |
| Oko 0.6.1 | 0.45 | 0.68 | 0.37 | **0.50** |
| Oko 0.6.0 | **0.46** | **0.70** | 0.37 | **0.50** |
| Oko 0.5.0 | 0.45 | 0.64 | **0.39** | 0.48 |
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
when ranked files are packed into 8,000 tokens, computed with the benchmark's
`report-bcy-curve`; Oko 0.7.0 leads it at every budget (4k: 0.39 against
RepoMap's 0.20; 16k: 0.60 against 0.53; 32k: 0.68 against 0.63; 0.6.1: 0.38,
0.60 and 0.68; 0.6.0: 0.62 and 0.69 at 16k and 32k).

What changed from 0.5.0 to 0.6.0: the shortlist Oko ranks grew from 30 to 60
candidates, so a needed file reaches ranking more often (0.72 → 0.82 of them)
and Recall@20 rose to 0.70, level with the best published embedding model.
MRR fell from 0.39 to 0.37, almost all of it on review comments (0.32 → 0.24),
where the right file is still near the top but less often first. 0.6.0 showed
an excerpt on 90% of tasks (0.5.0: about two thirds; 0.6.1: 89%).

Against RepoMap on the same 345 tasks (measured on the 0.6.1 runs, which 0.7.0
matches), Oko's MRR lead is +0.16 with a
95% bootstrap range of [+0.12, +0.20] by task and [+0.05, +0.26] with each
repository treated as one unit; Recall@5 +0.13, [+0.07, +0.19] by task;
Recall@20 +0.04, [−0.01, +0.09] by task and [−0.07, +0.15] by repository, so
the top-20 lead is no longer clear of zero (0.6.0: +0.06, [+0.01, +0.11] by
task).

### By task type, and by split

Oko 0.7.0, with 0.6.1 in brackets:

| Task | n | Oko R@5 | RepoMap R@5 | Oko R@20 | RepoMap R@20 | Oko MRR | RepoMap MRR |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Failing test → broken file | 101 | 0.78 (0.79) | 0.46 | 0.87 (0.86) | 0.85 | 0.70 (0.70) | 0.27 |
| Review comment → context | 80 | 0.24 (0.25) | 0.19 | 0.54 (0.55) | 0.50 | 0.24 (0.24) | 0.16 |
| Pull request → tests to update | 106 | 0.36 (0.35) | 0.26 | 0.64 (0.65) | 0.56 | 0.25 (0.25) | 0.20 |
| Edit → files it ripples into | 58 | 0.29 (0.32) | 0.36 | 0.61 (0.60) | 0.60 | 0.20 (0.21) | 0.23 |
| All 345 | 345 | 0.44 (0.45) | 0.32 | 0.68 (0.68) | 0.64 | 0.37 (0.37) | 0.22 |
| Held-out half | 181 | 0.42 (0.42) | 0.34 | 0.67 (0.66) | 0.64 | 0.33 (0.33) | 0.21 |
| Dev half | 164 | 0.47 (0.48) | 0.30 | 0.69 (0.70) | 0.63 | 0.42 (0.43) | 0.22 |
| Without gin-gonic/gin | 257 | 0.33 (0.34) | 0.27 | 0.59 (0.59) | 0.55 | 0.27 (0.27) | 0.19 |

Oko is strongest when the signal is an error message and weakest on the ripple
task, where RepoMap's import graph helps and Oko's one-hop links do not reach
far enough. One repository, gin-gonic/gin, contributes 88 of the 345 tasks (the
benchmark's own README notes this); without it Oko leads on all three
measures.

**Limits.** Scoring is per file: Oko returns functions, and gets no credit here
for the exact function. The queries are raw logs, review comments, and pull
request text, not the questions an agent would ask. Oko showed no excerpt on
about one task in ten (nothing reached its relevance cutoff); the ranked list
is still scored, but an agent would have to open the listed candidates.
No GPU or index is involved; each Oko search made three Jev requests.

## SWE-Explore

[SWE-Explore](https://arxiv.org/abs/2606.07297) (code:
[Qiushao-E/SWE-Explore-Bench](https://github.com/Qiushao-E/SWE-Explore-Bench))
has 848 real issues from SWE-bench Verified (451), SWE-bench Pro (215), and
SWE-bench Multilingual (182), across 64 repositories in ten languages. The
answer for each issue is the code that successful agents read while fixing it,
as line regions (4.3 files and 4.7 regions per issue on average), and an
explorer returns five ranked regions. We ran the benchmark's scorer at commit
`5602f031` once per release (0.7.0: commit `da83b98` with `jev-1.13.0`, all 848
answered; 0.6.1: commit `5f3afad`, all 848 answered; 0.6.0: commit `3f7809b`, 847 of 848 answered, the one failure hit the
response-size cap, fixed in the next commit), and its own BM25 and TF-IDF explorers
on the same inputs; ours match the paper's Table 6 within 0.01. Agent rows are
the paper's. Nothing was tuned on this benchmark.

| Method | Right file in top 5 | Right region in top 5 | Line precision | Line recall | nDCG@500 | First useful hit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Claude Code (agent, published) | 0.67 | | 0.60 | | 0.94 | |
| Mini-SWE-Agent (agent, published) | 0.64 | | 0.53 | | 0.89 | |
| LocAgent (agent, published) | 0.54 | | 0.64 | | 0.95 | |
| CoSIL (agent, published) | 0.54 | | 0.58 | | 0.82 | |
| **Oko 0.7.0, one call** | 0.40 | 0.34 | 0.56 | 0.18 | 0.82 | 0.89 |
| Oko 0.6.1, one call | 0.40 | 0.34 | 0.56 | 0.18 | 0.81 | 0.89 |
| Oko 0.6.0, one call | 0.40 | 0.35 | 0.56 | 0.17 | 0.81 | 0.89 |
| Oko 0.5.0, one call | 0.41 | 0.34 | 0.52 | 0.15 | 0.81 | 0.84 |
| AutoCodeRover (agent, published) | 0.28 | | 0.68 | | 0.72 | |
| Oko, keyword ranking only | 0.21 | 0.16 | 0.19 | 0.05 | 0.36 | 0.41 |
| TF-IDF (benchmark's explorer) | 0.14 | 0.11 | 0.10 | 0.04 | 0.22 | 0.23 |
| BM25 (benchmark's explorer) | 0.07 | 0.06 | 0.05 | 0.02 | 0.12 | 0.13 |

The agents explore for many turns with a frontier model; Oko is one search of
about a second with no model. On ranking (nDCG, first useful hit) and line
precision Oko sits in the agents' tier; on covering all of an issue's files it
does not, because five regions from one call cannot reach 4.3 files. In 0.6.0
its first region was in a right file 87% of the time (0.5.0: 83%). 0.6.1
matches 0.6.0 within 0.01 on every measure (line recall 0.172 → 0.177, context
efficiency 0.70 → 0.71), and 0.7.0 matches 0.6.1 within 0.01 on every measure
(nDCG 0.809 → 0.816, first useful hit 0.887 → 0.895). Ordering the five
slots so that each names a different file raised "right file" to 0.50 on 0.5.0
but halved precision, since the second to fifth files are usually wrong; we
report the by-score ordering. From 0.5.0 to 0.6.0, precision, recall and the
first useful hit rose, and the right-file rate held.

By source (0.6.0): SWE-bench Verified 0.47 right file, up on every measure;
Multilingual 0.40, with precision up from 0.44 to 0.50 (the benchmark's BM25
and TF-IDF score near zero there, because their chunker stops at 3,000 chunks
per repository); Pro 0.26, level with 0.5.0 on precision (0.55) but lower on
right file (0.29) and context efficiency (0.78 → 0.72). 0.6.1 by source:
Verified 0.46 right file with precision 0.60, Pro 0.26 with 0.54, Multilingual
0.41 with 0.50; 0.7.0 by source: Verified 0.46 with 0.59, Pro 0.26 with 0.54,
Multilingual 0.41 with 0.52.
Raw rows are in
[`benchmarks/published/0.7.0/swe-explore/`](../benchmarks/published/0.7.0/swe-explore/),
[`benchmarks/published/0.6.1/swe-explore/`](../benchmarks/published/0.6.1/swe-explore/),
[`benchmarks/published/0.6.0/swe-explore/`](../benchmarks/published/0.6.0/swe-explore/)
and, for 0.5.0 and the baselines,
[`benchmarks/published/0.5.0/swe-explore/`](../benchmarks/published/0.5.0/swe-explore/).

## Sense's agent benchmark, against Sense

[Sense](https://github.com/luuuc/sense) is a code-search MCP server with its own
public agent benchmark: six multi-step tasks on Axum, Discourse, Flask, Gin,
Javalin and Next.js, each answered by Claude Opus 4.7 with a budget and a time
limit ($1.00–2.25, 320–720 s). Scoring and the Opus 4.7 judge are the harness's.

### Latest run: Oko 0.7.0, Oko 0.6.1 and Sense (September 2026)

The README reports this run. On 30 September we ran the harness at Sense's
commit `a678631` with three setups in turn, task by task, five runs per task
each (90 sessions): Oko with the prompt hook (commit `ecb006a`; later commits
change only setup, metrics, docs and benchmark scripts), the Oko 0.6.1 release binary,
both installed in the image with `oko setup --client claude`, and Sense's own
image. All three ran on the harness's base image updated to Claude Code
2.1.286, which waits for MCP servers before running the first prompt's hook.

| | Oko 0.7.0 | Oko 0.6.1 | Sense |
| --- | ---: | ---: | ---: |
| **Cited recall** (the harness's headline) | **0.883** | 0.683 | 0.625 |
| **B-score** (0.55 cited + 0.25 related + 0.20 grounded) | **0.922** | 0.764 | 0.727 |
| Related (relation stated correctly) | **0.943** | 0.752 | 0.733 |
| Grounded precision (no false claims) | 1.000 | 1.000 | 1.000 |
| Sessions finished within budget and time | **30** | 28 | 21 |
| Model cost, 30 sessions | **$25.19** | $27.67 | $50.55 |
| Mean time per session | **161 s** | 170 s | 212 s |
| Tool calls per session | **6.7** | 9.0 | 32.1 |
| Whole-file reads per session | **0.7** | 1.6 | 12.5 |
| Model input tokens, 30 sessions (cache reads included) | **8.99M** | 11.96M | 28.84M |
| Model output tokens, 30 sessions | 382k | 340k | **314k** |
| Billed tokens per session (the harness's: uncached input and output) | 12,760 | 11,348 | **10,512** |

Cited recall and related come from the Discourse task alone (24 locations),
five runs each. Per run, Oko scored 0.88, 0.88, 0.83, 0.96 and 0.88; 0.6.1
scored 0.88, 0.88, 0.83, 0.83 and one session that ran to the time limit and
scored 0; Sense scored 0.79, 0.83, 0.62 and 0.88 (three of them over budget)
and one session that ran to the time limit. Most of the new build's lead over
0.6.1 is that one session: counting finished sessions only, it is 0.883 against
0.854. Across all six tasks Sense went over its budget in 8 of 30 sessions and
ran to the time limit in one; the harness scores what those sessions wrote.

Oko's agent wrote longer answers (more output tokens, so more billed tokens by
the harness's count) and read far less: 7.8M cache-read tokens against Sense's
27.1M. Jev usage was not recorded in this run; from the searches made it was
about $0.31 for Oko 0.7.0 and $0.34 for 0.6.1 at TypeSafe's listed $42 per
billion input tokens. Later runs record it. The transcripts do not show a
prompt hook's context, so we checked separately, with a stand-in model API in
the same image, that the first model request carries Oko's answer.

### Previous paired run: Oko 0.6.0 and Sense (September 2026)

We ran it at Sense's commit `a678631` with both
tools on the same day, five runs per task each (60 sessions, the harness's own
protocol), with Oko 0.6.0 (commit `3f7809b` with the Claude Code hooks that
0.6.0 ships, installed with `oko setup`'s guidance; the response-size fix came
one commit later) and Sense's own image.

| | Oko 0.6.0 | Sense |
| --- | ---: | ---: |
| **Cited recall** (the harness's headline) | **0.908** | 0.875 |
| **B-score** (0.55 cited + 0.25 related + 0.20 grounded) | **0.933** | 0.931 |
| Related (relation stated correctly) | 0.933 | **1.000** |
| Grounded precision (no false claims) | 1.000 | 1.000 |
| Total cost, 30 sessions | **$31.23** | $49.94 |
| Mean time per session | **210 s** | 242 s |
| Mean billed tokens per session | **10,857** | 13,089 |
| Sessions over budget or time | 4 | 5 |

Cited recall is the share of the task's must-find locations the answer cites as
`path:line`; in this version of the harness only the Discourse task has a
must-find set (24 locations), so the headline rests on five runs of one task.
Oko's lead holds under every way of counting the failed sessions: as the
harness counts them (scored on their partial answer) 0.908 against 0.875;
finished sessions only, 0.889 against 0.875; failed sessions as zero, 0.533
against 0.525. It is a small lead. Sense states the relations between the
found locations more exactly (1.00 against 0.93 on Discourse). One Sense
session that failed cost $8.71 and lifts its total; without it Sense's cost is
about $41. Counting every session, Oko was faster on five of the six tasks;
on Next.js one Oko session ran to the 720 s limit, making Oko's mean 275 s
against Sense's 258 s. Sense was also slightly cheaper on Flask ($2.96
against $3.05).

In a paired run two days earlier, on an early development build of 0.6.0,
Sense led cited recall 0.844 to 0.617; what closed the gap was the "who uses X" listing
of every dependent with `path:line`, which Sense answered with its call graph.
Sense's authors have since frozen this benchmark in favour of an internal one
that is not public.

**0.6.1, two tasks.** Before the latest run above, 0.6.1 was compared only with
the 0.6.0 build. On two of the six
tasks (Axum and Discourse, five runs each, the harness's image with Claude Code
2.1.281), 0.6.1 (`5f3afad`) and the build of the paired run above ran
alternately the same evening. On Discourse, counting the sessions that finished
(four each; one each ran to the time limit), cited recall was 0.906 for 0.6.1
against 0.792 for the 0.6.0 build, and B-score 0.931 against 0.853. Whole-file
reads per session fell from 6.6 to 2.8 on Axum and from 13.4 to 2.6 on
Discourse, and cost per Discourse session from $1.81 to $1.52. The same 0.6.0
build scored 0.908 in the paired run: five runs of one task move this number a
lot, which is why the table above stays as it was measured.

## Agent sessions

### Pi: 54 sessions (October 2026)

Completed 54 timed sessions: nine tasks across Astro, HTTPX and ripgrep, three
repetitions, two conditions. Two additional memory-canary sessions passed. Pi
1.0.0; Node 24.21.0; OpenAI GPT-5.6 Sol with low thinking. All raw assistant
events confirmed the requested model and their usage totals matched the report.

| Metric | Pi native | Pi + Oko |
|---|---:|---:|
| Sessions | 27 | 27 |
| Passed | 25 | 25 |
| Median seconds | 29.72 | 13.97 |
| Mean seconds | 31.38 | 15.00 |
| Coding-agent tokens | 1,130,932 | 250,043 |
| Uncached input tokens | 378,984 | 146,003 |
| Cached input tokens | 736,512 | 97,152 |
| Output tokens (includes reasoning) | 15,436 | 6,888 |
| Agent tool calls | 175 | 30 |
| Additional Jev tokens | 0 | 724,939 |
| Jev calls | 0 | 49 |

Oko reduced median session time by 53.0% and coding-agent tokens by 77.9% in
this run. Agent tokens include cached input; cache-write tokens were zero.
Additional Jev usage is reported separately. Combined recorded agent + Jev
tokens were 974,982 with Oko versus 1,130,932 native, a 13.8% reduction. Token
counts across different models are not billing estimates.

#### Quality

Both conditions passed 25/27 sessions, including every edit task (9/9 each).
Native missed an anchor in ripgrep-printed-bytes (repeat 1) and returned an
out-of-bounds line range in httpx-decoder-chain (repeat 3). Oko missed the final
URL authorization anchor in astro-image-probe-authorization in repetitions 2 and
3. All attempts are retained; failed answers were not retried. Equal aggregate
pass counts do not establish general quality equivalence.

#### Method

Pi native uses built-in file tools. Pi + Oko adds setup guidance, native MCP and
prefetch. Prefetch injected in all 27 enabled sessions. Enabled sessions use a
prebuilt disk index; task-independent index warmup is excluded from session
time. Each session starts with a fresh conversation, isolated client
configuration and fresh Oko session state. Provider-side caches are not cleared.
Timing includes the enabled prefetch work. Tool counts cover agent-issued calls;
prefetch runs before the first model call.

Isolation uses tool path guards and disabled shell tools, not an OS sandbox.
Offline instruction/extension isolation probes passed. Memory canaries test the
configured cross-session channel; they do not prove absence of every possible
memory channel. Source and frozen build artifacts were unchanged.

#### Build and reproduction

Branch: codex/pi-harness-support (uncommitted working-tree snapshot). Base
commit: cf59b521982926c26559e28afe88e6cc9e7d3728. Oko source SHA-256:
5a0d0cb1c3beba64c333ae2de83ade440b33dc4cfe1b2b7825786a19e160aaee. Binary
SHA-256: cf766f0afebf3df0bcea0e4987b0eb3c7198c9d0c9e0ab4935adb967668eadc3.

Run with Pi installed and authenticated: `python3
scripts/benchmark-public/runner.py --suite pi --prepare`, then `--suite pi
--check`, then `--suite pi --execute`. Default model is openai/gpt-5.6-sol.
Offline checks: public selftests 38 passed; shared selftests 24 passed;
observability tests 14 passed.

Sanitized [per-session
results](../benchmarks/published/0.7.2/agent-sessions/report-pi.json) preserve
every attempt without transcripts or local paths. This was measured before the
version-only bump from 0.7.1 to 0.7.2.

API-equivalent cost for 27 sessions per condition: $2.1193 native, $0.7606 agent
+ $0.0283 Jev = $0.7889 with Oko (62.8% lower). Rates checked 2 October:
[GPT-5.6 Sol](https://developers.openai.com/api/docs/pricing) $4/M uncached
input, $0.40/M cached input, $20/M output;
[Jev](https://docs.typesafe.ai/models) $0.042/M input, free output. Pi used
ChatGPT OAuth; estimates are not verified charges and exclude canaries and setup
checks.

## Previous run: 324 sessions, Oko 0.7.0 (September 2026)

The README reports this run. Same tasks, clients, models and repeats as before,
with four setups: **without Oko**, **Oko**, **Oko with the guidance `oko setup`
installs**, and **Oko with that guidance and setup's prompt hook**, which is
what `oko setup` now installs (the other Claude Code hooks are not part of
these sessions). The README compares the first and the last. A warm disk index
is excluded from timing. Claude Code and Codex ran on 30 September on commit
`498dd4e` (216 sessions, Claude Code 2.1.285, Codex CLI 0.155.0); OpenCode ran
the same day on commit `ecb006a` (108 sessions, OpenCode 1.18.31), which adds
OpenCode's prompt hook. Later commits change only setup, metrics, docs and
benchmark scripts.

Pass rates (without Oko, Oko, guidance, prompt hook): Codex 26/27, 26/27, 27/27,
27/27; OpenCode 27/27 in all four; Claude Code 26/27, 24/27, 24/27, 25/27. The
eleven failures: eight astro-forwarded-empty edits that also changed the
duplicate definition in `validate-headers.ts` (Codex once without Oko; Claude
Code twice with Oko, three times with guidance and twice with the prompt hook),
one Claude Code edit with Oko that also changed `crates/matcher/src/lib.rs`
(ripgrep-capture-hyphen), one Codex answer with Oko that missed one of three
locations (ripgrep-printed-bytes), and one Claude Code answer without Oko citing
a line past the end of a file.

Mean seconds, agent tokens, and tool calls per session (27 sessions per cell):

| Client | Without Oko | Oko | Oko + guidance | Oko + guidance + prompt hook |
| --- | --- | --- | --- | --- |
| Codex — seconds | 48.2 | 40.8 | 34.4 | 25.8 |
| Codex — agent tokens | 72,247 | 49,558 | 48,019 | 28,751 |
| Codex — tool calls | 3.52 | 2.07 | 1.70 | 0.89 |
| OpenCode — seconds | 25.7 | 21.7 | 18.3 | 13.1 |
| OpenCode — agent tokens | 28,513 | 17,915 | 16,341 | 10,624 |
| OpenCode — tool calls | 5.48 | 2.74 | 1.85 | 0.74 |
| Claude Code — seconds | 12.0 | 8.1 | 7.1 | 5.0 |
| Claude Code — agent tokens | 56,513 | 35,751 | 36,406 | 22,008 |
| Claude Code — tool calls | 4.44 | 2.15 | 1.85 | 0.67 |

What the prompt hook changed:

- **The first model round already has the code.** The hook answered every
  prompt in these sessions (81 of 81), in about 0.4 s at the median, with one
  Jev request of about 13,800 input tokens. Against guidance alone, model
  rounds per session fell from 2.85 to 1.67 for Claude Code and from 2.85 to
  1.74 for OpenCode, and the agent's own Oko searches from 1.22 to 0.07 per
  session (Claude Code), 1.22 to 0.37 (Codex) and 1.30 to 0.30 (OpenCode).
  63 of the 81 sessions made no Oko call.
- **Tokens fell a further 35–40%** against guidance alone (Claude Code −40%,
  Codex −40%, OpenCode −35%), and time 25–30%. Each model round re-reads the
  conversation so far, so one round fewer saves a large share of a session's
  tokens.
- **Not every task gained.** Against guidance alone, tokens rose on four
  task–tool pairs: ripgrep-printed-bytes for Claude Code (+18%) and OpenCode
  (+9%), astro-image-probe-authorization for Codex (+12%) and
  ripgrep-capture-expansion for Codex (+7%, and 18% slower).
- **Codex sessions without Oko took longer** than in the 0.6.1 run (48.2 s,
  against 25.1 s), so compare setups within a run, not across runs.

## Previous run: 243 sessions, Oko 0.6.1 (September 2026)

The 0.6.1 release reported this run. Same tasks, clients, models, repeats and three
setups as before: **without Oko**, **Oko**, and **Oko with the guidance `oko
setup` installs** (the Claude Code hooks are not part of these sessions), with
a warm disk index excluded from timing. Measured on commit `5f3afad` (the
release commit after it changes only version numbers and docs), with Codex CLI
0.155.0, OpenCode 1.18.31 and Claude Code 2.1.285. Pass rates: Codex 27/27,
27/27, 26/27; OpenCode 26/27, 25/27, 26/27; Claude Code 23/27, 24/27, 22/27.
The seventeen failures: nine astro-forwarded-empty edits that changed the
duplicate definition in `validate-headers.ts` (OpenCode without Oko; OpenCode
and Claude Code twice each with Oko; Codex, OpenCode and Claude Code twice with
guidance), five ripgrep-capture-hyphen edits by Claude Code that also changed
documentation comments (three without Oko, one with Oko, one with guidance), two
Claude Code answers citing a line past the end of a file (without Oko and with
guidance), and one Claude Code answer with guidance that missed two of four
locations (httpx-async-auth-body).

Mean seconds, agent tokens, and tool calls per session (27 sessions per cell):

| Client | Without Oko | Oko | Oko + guidance |
| --- | --- | --- | --- |
| Codex — seconds | 25.1 | 22.5 | 19.7 |
| Codex — agent tokens | 70,512 | 52,203 | 48,814 |
| Codex — tool calls | 3.33 | 2.22 | 1.81 |
| OpenCode — seconds | 22.5 | 19.1 | 18.3 |
| OpenCode — agent tokens | 29,429 | 18,131 | 19,375 |
| OpenCode — tool calls | 5.67 | 2.78 | 2.26 |
| Claude Code — seconds | 14.0 | 10.0 | 7.9 |
| Claude Code — agent tokens | 61,384 | 41,280 | 36,800 |
| Claude Code — tool calls | 4.93 | 2.63 | 1.89 |

What changed since 0.6.0:

- **Fewer follow-up searches.** With guidance, Codex sent 31 Oko searches over
  27 sessions (0.6.0: 44) and OpenCode 36 (0.6.0: 54); a second search came in
  4 and 8 of 18 search sessions (0.6.0: 11 and 15). The first answer held every
  expected location in 15 of 18 search sessions for both (0.6.0: 13). OpenCode's
  token saving with guidance went from 24% to 34%, Codex's from 27% to 31%.
- **Claude Code reads fewer whole files.** The guidance now tells agents to ask
  Oko for unshown code by name in `symbols` instead of reading whole files; on
  Sense's longer tasks that cut whole-file reads by 60–80% (see
  [above](#senses-agent-benchmark-against-sense)). Claude Code sent 31 Oko
  searches with guidance (0.6.0: 28).
- **Claude Code updated itself** from 2.1.282 to 2.1.285 between runs and used
  61,384 tokens per session without Oko (33,698 in the 0.6.0 run), so compare
  its figures within this run only.
- **Providers were faster** for Codex and OpenCode without Oko (Codex 26.0 s →
  25.1 s, OpenCode 27.0 s → 22.5 s), so compare setups within a run, not across
  runs.

## Previous run: 243 sessions, Oko 0.6.0 (September 2026)

The 0.6.0 release reported this run. Same tasks, clients, models, repeats and three
setups as before: **without Oko**, **Oko**, and **Oko with the guidance `oko
setup` installs** (the written guidance; the Claude Code hooks are not part of
these sessions), with a warm disk index excluded from timing. Measured on the
release build (commit `802ab8a`; the release commit after it changes only
version numbers and docs), with Codex CLI 0.155.0, OpenCode 1.18.31 and Claude
Code 2.1.282. Pass rates: Codex 26/27, 27/27, 27/27; OpenCode 27/27 in all
three; Claude Code 23/27, 25/27, 27/27. The seven failures: three
astro-forwarded-empty edits that also changed a duplicate definition in
`validate-headers.ts` (Codex and Claude Code without Oko), one answer citing a
line past the end of a file (Claude Code without Oko), and three Claude Code
answers with text after the JSON (one without Oko, two with Oko).

Grading changed once since 0.5.1: in late September Claude Code began putting a
sentence before its JSON answer ("I have enough to answer."), with and without
Oko and on older Oko builds alike. The grader now scores the JSON of a search
answer even after such a sentence, for every client and setup, and marks it;
one answer in this run needed that. Text after the JSON still fails.

Mean seconds, agent tokens, and tool calls per session (27 sessions per cell):

| Client | Without Oko | Oko | Oko + guidance |
| --- | --- | --- | --- |
| Codex — seconds | 26.0 | 29.4 | 26.1 |
| Codex — agent tokens | 74,800 | 61,484 | 54,413 |
| Codex — tool calls | 3.26 | 2.89 | 2.22 |
| OpenCode — seconds | 27.0 | 28.9 | 23.5 |
| OpenCode — agent tokens | 29,732 | 24,254 | 22,623 |
| OpenCode — tool calls | 5.81 | 3.93 | 2.74 |
| Claude Code — seconds | 9.8 | 7.3 | 7.1 |
| Claude Code — agent tokens | 33,698 | 23,997 | 28,067 |
| Claude Code — tool calls | 3.96 | 2.07 | 2.26 |

What changed since 0.5.1:

- **More Oko calls per session.** Over 27 sessions with guidance, Codex sent 44
  Oko searches (0.5.1: 33) and OpenCode 54 (0.5.1: 29); Claude Code still sent
  one per session (28 against 27). The first answer held every expected
  location as often as before; the second search, usually a `questions` batch
  re-asking part of the trace, rarely added anything on these tasks. That is
  where OpenCode's token savings went (44% → 24% fewer with guidance). The same
  inputs are what lifted Oko on [Sense's benchmark](#senses-agent-benchmark-against-sense),
  whose tasks need every dependent of a class; the next release will steer
  agents away from re-checking an answer they already have.
- **Providers were faster** for Codex and OpenCode with and without Oko (Codex
  without Oko 34.4 s → 26.0 s), so compare setups within a run, not across runs.
- **Claude Code without Oko** used 33,698 tokens per session, against 22,946 in
  the 0.5.1 run; with guidance Claude Code passed 27/27 and was 28% faster.

## Previous run: 243 sessions, Oko 0.5.1 (September 2026)

The 0.5.1 release reported this run. Same tasks, clients, models, repeats, and three
setups as the 0.5.0 run below: **without Oko**, **Oko**, and **Oko with the
guidance `oko setup` installs**, with a warm disk index excluded from timing.
Measured on the 0.5.1 build (commit `48da7a9`; the release commit after it
changes only version numbers and installation docs), with Codex CLI 0.155.0,
OpenCode 1.18.31, and Claude Code 2.1.280. Pass rates: Codex 26/27, 27/27,
25/27; OpenCode 27/27 in all three; Claude Code 25/27, 27/27, 27/27. The five
failures: three astro-forwarded-empty edits that also changed a duplicate
definition in `validate-headers.ts` (Codex without Oko and with guidance, Claude
Code without Oko), one ripgrep-printed-bytes answer that missed an anchor (Codex
with guidance), and one ripgrep-printed-bytes answer the grader rejected
(Claude Code without Oko).

Mean seconds, agent tokens, and tool calls per session (27 sessions per cell):

| Client | Without Oko | Oko | Oko + guidance |
| --- | --- | --- | --- |
| Codex — seconds | 34.4 | 34.3 | 34.6 |
| Codex — agent tokens | 69,724 | 55,619 | 49,808 |
| Codex — tool calls | 3.11 | 2.56 | 2.07 |
| OpenCode — seconds | 33.9 | 27.9 | 26.8 |
| OpenCode — agent tokens | 29,675 | 17,482 | 16,576 |
| OpenCode — tool calls | 5.89 | 3.00 | 2.30 |
| Claude Code — seconds | 8.5 | 6.9 | 6.5 |
| Claude Code — agent tokens | 22,946 | 18,729 | 20,072 |
| Claude Code — tool calls | 3.37 | 1.78 | 1.70 |

What changed since 0.5.0:

- **Claude Code without Oko** used 22,946 tokens per session, against 60,320 in
  the 0.5.0 run and 23,960 in the 0.4.0 run. The 0.5.0 day was the unusual one,
  so Claude Code's gains here are smaller than that run reported (23% faster and
  13% fewer tokens, against 38% and 39%). With Oko, Claude Code passed 27/27.
- **Codex's average time** includes one Oko session (httpx-async-auth-body,
  repetition 3) that took 123 s with a single tool call: Oko's search took
  0.45 s and Jev 1.1 s, and the rest was the model provider. All attempts count;
  without that session Codex's Oko + guidance mean is 31.2 s, 9% faster than
  without Oko.
- The guidance gained one line in 0.5.1, about passing Oko on to subagents.
  These sessions do not use subagents, so it has nothing to act on here.
- 0.5.1 lets searches run in parallel. No session in this run or the 0.5.0 run
  sent searches in parallel, so it does not show up here.

Per task, Oko + guidance against without Oko (mean of three sessions; time, tokens):

| Task | Codex | OpenCode | Claude Code |
| --- | --- | --- | --- |
| astro-image-probe-authorization | −37%, −43% | −46%, −65% | −40%, −28% |
| astro-action-key-guards | −45%, −58% | −21%, −60% | −48%, −36% |
| astro-forwarded-empty (edit) | +11%, −27% | −11%, −50% | +18%, −6% |
| httpx-decoder-chain | +21%, +54% | −7%, +6% | −19%, +23% |
| httpx-async-auth-body | +94%, −26% | +6%, −9% | −22%, −11% |
| httpx-reason-fallback (edit) | −4%, +3% | −17%, −43% | −13%, −1% |
| ripgrep-capture-expansion | −17%, −44% | −35%, −35% | −44%, −28% |
| ripgrep-printed-bytes | −4%, −28% | −25%, −32% | −36%, −11% |
| ripgrep-capture-hyphen (edit) | +0%, −36% | −23%, −57% | −15%, −10% |

Mean tool calls per task, without Oko → Oko + guidance. An Oko search counts as
one call. They fell on every task for every client except Codex on
httpx-decoder-chain:

| Task | Codex | OpenCode | Claude Code |
| --- | --- | --- | --- |
| astro-image-probe-authorization | 2.7 → 1.0 | 6.3 → 1.3 | 3.3 → 1.0 |
| astro-action-key-guards | 3.7 → 1.0 | 7.0 → 2.0 | 3.7 → 1.0 |
| astro-forwarded-empty (edit) | 3.0 → 2.7 | 5.3 → 3.0 | 2.7 → 2.0 |
| httpx-decoder-chain | 2.7 → 3.3 | 6.3 → 2.3 | 4.3 → 2.7 |
| httpx-async-auth-body | 2.7 → 1.3 | 5.0 → 2.7 | 3.0 → 1.7 |
| httpx-reason-fallback (edit) | 3.0 → 2.0 | 3.7 → 2.0 | 2.3 → 2.0 |
| ripgrep-capture-expansion | 3.7 → 2.7 | 8.3 → 3.0 | 3.3 → 1.0 |
| ripgrep-printed-bytes | 3.3 → 2.7 | 6.3 → 2.3 | 4.7 → 2.0 |
| ripgrep-capture-hyphen (edit) | 3.3 → 2.0 | 4.7 → 2.0 | 3.0 → 2.0 |

## Previous run: 243 sessions, Oko 0.5.0 (September 2026)

The README reported this run until 0.5.1. Nine tasks (six code-location questions and three
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
