# Public repository benchmark

Sessions: 164/216. Complete: False.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | native | 20/20 | 13.22 | 907899 |
| codex | prefetch | 20/20 | 10.62 | 520070 |
| opencode | native | 21/21 | 16.84 | 277125 |
| opencode | prefetch | 21/21 | 7.56 | 158096 |
| claude | native | 19/20 | 12.57 | 485420 |
| claude | prefetch | 20/20 | 5.40 | 272537 |
| pi | native | 20/21 | 17.20 | 204472 |
| pi | prefetch | 21/21 | 7.27 | 136385 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals; the Components table gives it in full (requests, input, output, total), including the prompt hook's. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Model input | Model output | Model total | Jev calls | Jev input | Jev output | Jev total | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| codex | native | 189341 | 712832 | 0 | 902173 | 5726 | 907899 | 0 | 0 | 0 | 0 | 3.0 | unavailable | 0.0 |
| codex | prefetch | 130270 | 386304 | 0 | 516574 | 3496 | 520070 | 51 | 676549 | 57885 | 734434 | 1.0 | unavailable | unavailable |
| opencode | native | 132236 | 138112 | 0 | 270348 | 6777 | 277125 | 0 | 0 | 0 | 0 | 3 | 4 | 0 |
| opencode | prefetch | 113091 | 42112 | 0 | 155203 | 2893 | 158096 | 52 | 702035 | 59020 | 761055 | 0 | 1 | unavailable |
| claude | native | 148 | 394121 | 76522 | 470791 | 14629 | 485420 | 0 | 0 | 0 | 0 | 3.0 | 4.0 | 0.0 |
| claude | prefetch | 56 | 192153 | 75104 | 267313 | 5224 | 272537 | 51 | 676549 | 57885 | 734434 | 0.0 | 1.0 | unavailable |
| pi | native | 129225 | 70656 | 0 | 199881 | 4591 | 204472 | 0 | 0 | 0 | 0 | 4 | 4 | 0 |
| pi | prefetch | 103688 | 30208 | 0 | 133896 | 2489 | 136385 | 55 | 717318 | 61290 | 778608 | 1 | 2 | unavailable |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | prefetch | Passed/attempted |
|---|---|---|---:|---:|---:|
| astro | astro-image-probe-authorization | claude | 13.08s | 4.53s | 5/6 |
| astro | astro-image-probe-authorization | codex | 12.95s | 13.86s | 6/6 |
| astro | astro-image-probe-authorization | opencode | 20.84s | 7.59s | 6/6 |
| astro | astro-image-probe-authorization | pi | 19.08s | 9.55s | 6/6 |
| httpx | httpx-decoder-chain | codex | 13.67s | 5.55s | 6/6 |
| httpx | httpx-decoder-chain | opencode | 12.33s | 6.22s | 6/6 |
| httpx | httpx-decoder-chain | pi | 13.53s | 4.17s | 6/6 |
| httpx | httpx-decoder-chain | claude | 8.26s | 8.83s | 6/6 |
| ripgrep | ripgrep-capture-expansion | opencode | 12.74s | 5.73s | 6/6 |
| ripgrep | ripgrep-capture-expansion | pi | 17.20s | 4.64s | 5/6 |
| ripgrep | ripgrep-capture-expansion | claude | 10.08s | 3.07s | 4/4 |
| ripgrep | ripgrep-capture-expansion | codex | 10.90s | 5.99s | 4/4 |
| astro | astro-action-key-guards | codex | 12.30s | 5.51s | 4/4 |
| astro | astro-action-key-guards | opencode | 19.83s | 6.97s | 4/4 |
| astro | astro-action-key-guards | pi | 14.46s | 5.62s | 4/4 |
| astro | astro-action-key-guards | claude | 7.17s | 4.73s | 4/4 |
| httpx | httpx-async-auth-body | opencode | 12.49s | 6.83s | 4/4 |
| httpx | httpx-async-auth-body | pi | 11.16s | 3.83s | 4/4 |
| httpx | httpx-async-auth-body | claude | 14.47s | 3.70s | 4/4 |
| httpx | httpx-async-auth-body | codex | 9.79s | 5.58s | 4/4 |
| ripgrep | ripgrep-printed-bytes | pi | 12.92s | 7.50s | 4/4 |
| ripgrep | ripgrep-printed-bytes | claude | 11.93s | 5.46s | 4/4 |
| ripgrep | ripgrep-printed-bytes | codex | 12.56s | 11.87s | 4/4 |
| ripgrep | ripgrep-printed-bytes | opencode | 15.47s | 9.50s | 4/4 |
| astro | astro-forwarded-empty | opencode | 23.38s | 18.55s | 4/4 |
| astro | astro-forwarded-empty | pi | 21.09s | 17.78s | 4/4 |
| astro | astro-forwarded-empty | claude | 11.25s | 10.23s | 4/4 |
| astro | astro-forwarded-empty | codex | 20.93s | 19.29s | 4/4 |
| httpx | httpx-reason-fallback | pi | 21.96s | 12.21s | 4/4 |
| httpx | httpx-reason-fallback | claude | 13.09s | 6.25s | 4/4 |
| httpx | httpx-reason-fallback | codex | 16.02s | 11.02s | 4/4 |
| httpx | httpx-reason-fallback | opencode | 17.08s | 10.72s | 4/4 |
| ripgrep | ripgrep-capture-hyphen | claude | 17.80s | 6.94s | 4/4 |
| ripgrep | ripgrep-capture-hyphen | codex | 17.56s | 14.48s | 4/4 |
| ripgrep | ripgrep-capture-hyphen | opencode | 27.99s | 14.88s | 4/4 |
| ripgrep | ripgrep-capture-hyphen | pi | 20.16s | 11.12s | 4/4 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
