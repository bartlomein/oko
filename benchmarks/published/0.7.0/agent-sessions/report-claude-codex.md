# Public repository benchmark

Sessions: 216/216. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | native | 26/27 | 45.80 | 1950675 |
| codex | current | 26/27 | 35.64 | 1338076 |
| codex | guided | 27/27 | 27.94 | 1296500 |
| codex | prefetch | 27/27 | 25.49 | 776281 |
| claude | native | 26/27 | 10.64 | 1525853 |
| claude | current | 24/27 | 7.08 | 965281 |
| claude | guided | 24/27 | 5.50 | 982958 |
| claude | prefetch | 25/27 | 4.52 | 594215 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|
| codex | native | 432696 | 1498368 | 0 | 19611 | 0 / 0 | 3 | unavailable | 0 |
| codex | current | 240153 | 1082624 | 0 | 15299 | 1532715 / 107583 | 2 | unavailable | 0.534 |
| codex | guided | 237987 | 1045632 | 0 | 12881 | 1605962 / 115382 | 1 | unavailable | 0.568 |
| codex | prefetch | 215354 | 551296 | 0 | 9631 | unavailable / unavailable | 1 | unavailable | unavailable |
| claude | native | 278 | 1344087 | 154951 | 26537 | 0 / 0 | 4 | 5 | 0 |
| claude | current | 164 | 822294 | 128210 | 14613 | 1217022 / 83701 | 2 | 3 | 0.468 |
| claude | guided | 154 | 849798 | 120286 | 12720 | 1364576 / 96576 | 2 | 3 | 0.539 |
| claude | prefetch | 90 | 487119 | 97943 | 9063 | unavailable / unavailable | 1 | 2 | unavailable |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | current | guided | prefetch | Passed/attempted |
|---|---|---|---:|---:|---:|---:|---:|
| astro | astro-image-probe-authorization | claude | 15.17s | 5.93s | 5.88s | 3.22s | 11/12 |
| astro | astro-image-probe-authorization | codex | 46.33s | 24.93s | 24.65s | 23.83s | 12/12 |
| httpx | httpx-decoder-chain | codex | 39.13s | 35.64s | 20.17s | 17.25s | 12/12 |
| httpx | httpx-decoder-chain | claude | 8.75s | 5.12s | 4.10s | 3.72s | 12/12 |
| ripgrep | ripgrep-capture-expansion | claude | 12.86s | 5.31s | 4.37s | 5.93s | 12/12 |
| ripgrep | ripgrep-capture-expansion | codex | 43.05s | 57.22s | 24.94s | 27.41s | 12/12 |
| astro | astro-action-key-guards | codex | 53.74s | 31.44s | 22.76s | 9.66s | 12/12 |
| astro | astro-action-key-guards | claude | 16.72s | 5.61s | 5.25s | 3.38s | 12/12 |
| httpx | httpx-async-auth-body | claude | 10.34s | 8.12s | 4.66s | 1.98s | 12/12 |
| httpx | httpx-async-auth-body | codex | 32.31s | 24.44s | 23.80s | 7.90s | 12/12 |
| ripgrep | ripgrep-printed-bytes | codex | 41.39s | 38.21s | 38.21s | 26.73s | 11/12 |
| ripgrep | ripgrep-printed-bytes | claude | 13.62s | 6.06s | 5.35s | 5.27s | 12/12 |
| astro | astro-forwarded-empty | claude | 8.10s | 13.85s | 17.77s | 11.26s | 5/12 |
| astro | astro-forwarded-empty | codex | 55.99s | 61.01s | 52.77s | 48.04s | 11/12 |
| httpx | httpx-reason-fallback | codex | 44.18s | 39.17s | 37.23s | 28.21s | 12/12 |
| httpx | httpx-reason-fallback | claude | 8.49s | 8.31s | 10.96s | 4.52s | 12/12 |
| ripgrep | ripgrep-capture-hyphen | claude | 10.74s | 10.00s | 9.21s | 5.83s | 11/12 |
| ripgrep | ripgrep-capture-hyphen | codex | 56.58s | 50.09s | 47.09s | 32.75s | 12/12 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
| codex | native | 27 | -5.82 | -14.2% |
| codex | current -> guided | 27 | -4.09 | -11.0% |
| claude | native | 27 | -4.05 | -41.1% |
| claude | current -> guided | 27 | -0.87 | -14.8% |

Search answers with text before their JSON, graded on the JSON: 6.
