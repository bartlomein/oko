# Public repository benchmark

Sessions: 108/108. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| opencode | native | 27/27 | 25.36 | 769854 |
| opencode | current | 27/27 | 22.02 | 483694 |
| opencode | guided | 27/27 | 17.48 | 441207 |
| opencode | prefetch | 27/27 | 12.93 | 286856 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|
| opencode | native | 330915 | 423168 | 0 | 15771 | 0 / 0 | 6 | 4 | 0 |
| opencode | current | 222308 | 250112 | 0 | 11274 | 1289482 / 88657 | 3 | 4 | 0.672 |
| opencode | guided | 209921 | 221568 | 0 | 9718 | 1227728 / 83661 | 2 | 3 | 0.577 |
| opencode | prefetch | 173171 | 107904 | 0 | 5781 | unavailable / unavailable | 1 | 2 | unavailable |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | current | guided | prefetch | Passed/attempted |
|---|---|---|---:|---:|---:|---:|---:|
| astro | astro-image-probe-authorization | opencode | 25.54s | 14.08s | 13.23s | 9.33s | 12/12 |
| httpx | httpx-decoder-chain | opencode | 18.09s | 22.08s | 10.48s | 6.56s | 12/12 |
| ripgrep | ripgrep-capture-expansion | opencode | 27.44s | 18.85s | 19.81s | 13.06s | 12/12 |
| astro | astro-action-key-guards | opencode | 23.72s | 17.04s | 15.80s | 9.78s | 12/12 |
| httpx | httpx-async-auth-body | opencode | 19.34s | 20.53s | 12.64s | 6.70s | 12/12 |
| ripgrep | ripgrep-printed-bytes | opencode | 25.17s | 22.77s | 16.55s | 12.93s | 12/12 |
| astro | astro-forwarded-empty | opencode | 30.60s | 26.34s | 22.78s | 21.71s | 12/12 |
| httpx | httpx-reason-fallback | opencode | 23.16s | 23.02s | 19.52s | 12.98s | 12/12 |
| ripgrep | ripgrep-capture-hyphen | opencode | 33.47s | 23.89s | 23.91s | 16.05s | 12/12 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
| opencode | native | 27 | -3.42 | -13.2% |
| opencode | current -> guided | 27 | -3.50 | -16.1% |
