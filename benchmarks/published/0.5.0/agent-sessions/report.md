# Public repository benchmark

Sessions: 243/243. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | native | 27/27 | 32.60 | 1723365 |
| codex | current | 27/27 | 32.21 | 1452844 |
| codex | guided | 26/27 | 27.22 | 1377686 |
| opencode | native | 27/27 | 31.95 | 802233 |
| opencode | current | 27/27 | 25.51 | 482199 |
| opencode | guided | 27/27 | 23.96 | 440082 |
| claude | native | 22/27 | 13.21 | 1628626 |
| claude | current | 22/27 | 8.53 | 1166756 |
| claude | guided | 22/27 | 8.16 | 997594 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|
| codex | native | 366874 | 1337472 | 0 | 19019 | 0 / 0 | 3 | unavailable | 0 |
| codex | current | 369325 | 1065984 | 0 | 17535 | 882366 / 47787 | 3 | unavailable | 0.659 |
| codex | guided | 306005 | 1055872 | 0 | 15809 | 1047737 / 56688 | 2 | unavailable | 0.794 |
| opencode | native | 333891 | 452352 | 0 | 15990 | 0 / 0 | 6 | 4 | 0 |
| opencode | current | 231058 | 237696 | 0 | 13445 | 833985 / 45176 | 3 | 3 | 0.653 |
| opencode | guided | 221094 | 206848 | 0 | 12140 | 902888 / 49236 | 2 | 3 | 0.635 |
| claude | native | 290 | 1442757 | 156224 | 29355 | 0 / 0 | 5 | 5 | 0 |
| claude | current | 202 | 1010734 | 138750 | 17070 | unavailable / unavailable | 3 | 3 | 0.654 |
| claude | guided | 172 | 854318 | 129205 | 13899 | 914091 / 50185 | 2 | 3 | 0.747 |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | current | guided | Passed/attempted |
|---|---|---|---:|---:|---:|---:|
| astro | astro-image-probe-authorization | codex | 27.77s | 21.75s | 17.36s | 9/9 |
| astro | astro-image-probe-authorization | opencode | 26.28s | 15.36s | 17.58s | 9/9 |
| astro | astro-image-probe-authorization | claude | 8.83s | 8.53s | 8.91s | 8/9 |
| httpx | httpx-decoder-chain | opencode | 24.12s | 22.93s | 27.78s | 9/9 |
| httpx | httpx-decoder-chain | claude | 13.82s | 5.36s | 6.47s | 9/9 |
| httpx | httpx-decoder-chain | codex | 31.38s | 30.91s | 32.98s | 9/9 |
| ripgrep | ripgrep-capture-expansion | claude | 13.66s | 10.90s | 4.40s | 7/9 |
| ripgrep | ripgrep-capture-expansion | codex | 33.63s | 35.04s | 36.95s | 9/9 |
| ripgrep | ripgrep-capture-expansion | opencode | 40.02s | 27.23s | 19.21s | 9/9 |
| astro | astro-action-key-guards | opencode | 39.25s | 26.71s | 20.49s | 9/9 |
| astro | astro-action-key-guards | claude | 14.01s | 5.14s | 5.61s | 6/9 |
| astro | astro-action-key-guards | codex | 33.99s | 27.88s | 18.42s | 9/9 |
| httpx | httpx-async-auth-body | claude | 12.10s | 10.10s | 9.44s | 7/9 |
| httpx | httpx-async-auth-body | codex | 26.80s | 26.64s | 25.79s | 9/9 |
| httpx | httpx-async-auth-body | opencode | 23.19s | 19.57s | 19.76s | 9/9 |
| ripgrep | ripgrep-printed-bytes | codex | 31.83s | 31.59s | 26.37s | 9/9 |
| ripgrep | ripgrep-printed-bytes | opencode | 34.56s | 24.54s | 25.46s | 9/9 |
| ripgrep | ripgrep-printed-bytes | claude | 14.71s | 9.15s | 10.20s | 9/9 |
| astro | astro-forwarded-empty | claude | 9.93s | 12.45s | 9.12s | 3/9 |
| astro | astro-forwarded-empty | codex | 43.54s | 44.13s | 50.16s | 8/9 |
| astro | astro-forwarded-empty | opencode | 35.78s | 43.76s | 45.53s | 9/9 |
| httpx | httpx-reason-fallback | codex | 30.04s | 32.21s | 27.22s | 9/9 |
| httpx | httpx-reason-fallback | opencode | 27.04s | 25.51s | 23.96s | 9/9 |
| httpx | httpx-reason-fallback | claude | 9.20s | 5.90s | 8.12s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | opencode | 37.54s | 35.33s | 27.30s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | claude | 16.69s | 11.28s | 8.67s | 8/9 |
| ripgrep | ripgrep-capture-hyphen | codex | 41.96s | 38.12s | 34.47s | 9/9 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
| codex | native | 27 | -0.40 | -1.2% |
| codex | current -> guided | 27 | -4.99 | -15.5% |
| opencode | native | 27 | -4.07 | -12.4% |
| opencode | current -> guided | 27 | -0.66 | -3.9% |
| claude | native | 27 | -2.57 | -20.0% |
| claude | current -> guided | 27 | -0.40 | -4.8% |
