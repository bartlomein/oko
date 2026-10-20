# Public repository benchmark

Sessions: 216/216. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | prefetch-previous | 27/27 | 10.80 | 885096 |
| codex | prefetch | 27/27 | 9.82 | 684950 |
| opencode | prefetch-previous | 27/27 | 9.35 | 223688 |
| opencode | prefetch | 27/27 | 10.10 | 226673 |
| claude | prefetch-previous | 27/27 | 5.85 | 471888 |
| claude | prefetch | 26/27 | 5.17 | 400276 |
| pi | prefetch-previous | 27/27 | 8.27 | 206968 |
| pi | prefetch | 27/27 | 7.65 | 185906 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals; the Components table gives it in full (requests, input, output, total), including the prompt hook's. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Model input | Model output | Model total | Jev calls | Jev input | Jev output | Jev total | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| codex | prefetch-previous | 169939 | 709504 | 0 | 879443 | 5653 | 885096 | 42 | 583455 | 44592 | 628047 | 1 | unavailable | unavailable |
| codex | prefetch | 173554 | 506496 | 0 | 680050 | 4900 | 684950 | 66 | 865493 | 73775 | 939268 | 1 | unavailable | unavailable |
| opencode | prefetch-previous | 155058 | 64640 | 0 | 219698 | 3990 | 223688 | 40 | 526779 | 41285 | 568064 | 1 | 2 | unavailable |
| opencode | prefetch | 151835 | 70784 | 0 | 222619 | 4054 | 226673 | 66 | 892338 | 74340 | 966678 | 1 | 2 | unavailable |
| claude | prefetch-previous | 98 | 350507 | 112185 | 462790 | 9098 | 471888 | 27 | 371586 | 30645 | 402231 | 1 | 2 | unavailable |
| claude | prefetch | 82 | 290341 | 102200 | 392623 | 7653 | 400276 | 69 | 891229 | 75475 | 966704 | 0 | 1 | unavailable |
| pi | prefetch-previous | 136434 | 66816 | 0 | 203250 | 3718 | 206968 | 43 | 568444 | 44027 | 612471 | 1 | 2 | unavailable |
| pi | prefetch | 124151 | 58368 | 0 | 182519 | 3387 | 185906 | 69 | 907076 | 76496 | 983572 | 1 | 2 | unavailable |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | prefetch-previous | prefetch | Passed/attempted |
|---|---|---|---:|---:|---:|
| astro | astro-image-probe-authorization | claude | 3.06s | 5.17s | 5/6 |
| astro | astro-image-probe-authorization | codex | 11.88s | 11.19s | 6/6 |
| astro | astro-image-probe-authorization | opencode | 7.29s | 11.10s | 6/6 |
| astro | astro-image-probe-authorization | pi | 8.84s | 9.36s | 6/6 |
| httpx | httpx-decoder-chain | codex | 10.69s | 6.09s | 6/6 |
| httpx | httpx-decoder-chain | opencode | 6.85s | 6.29s | 6/6 |
| httpx | httpx-decoder-chain | pi | 3.34s | 3.66s | 6/6 |
| httpx | httpx-decoder-chain | claude | 5.85s | 2.94s | 6/6 |
| ripgrep | ripgrep-capture-expansion | opencode | 8.70s | 6.12s | 6/6 |
| ripgrep | ripgrep-capture-expansion | pi | 8.02s | 6.64s | 6/6 |
| ripgrep | ripgrep-capture-expansion | claude | 4.90s | 4.41s | 6/6 |
| ripgrep | ripgrep-capture-expansion | codex | 10.86s | 5.54s | 6/6 |
| astro | astro-action-key-guards | codex | 6.99s | 7.01s | 6/6 |
| astro | astro-action-key-guards | opencode | 12.52s | 7.81s | 6/6 |
| astro | astro-action-key-guards | pi | 5.24s | 5.49s | 6/6 |
| astro | astro-action-key-guards | claude | 6.05s | 3.22s | 6/6 |
| httpx | httpx-async-auth-body | opencode | 5.54s | 7.61s | 6/6 |
| httpx | httpx-async-auth-body | pi | 3.89s | 4.37s | 6/6 |
| httpx | httpx-async-auth-body | claude | 4.00s | 4.37s | 6/6 |
| httpx | httpx-async-auth-body | codex | 6.02s | 5.42s | 6/6 |
| ripgrep | ripgrep-printed-bytes | pi | 8.27s | 8.65s | 6/6 |
| ripgrep | ripgrep-printed-bytes | claude | 9.04s | 8.84s | 6/6 |
| ripgrep | ripgrep-printed-bytes | codex | 10.09s | 9.82s | 6/6 |
| ripgrep | ripgrep-printed-bytes | opencode | 9.44s | 10.10s | 6/6 |
| astro | astro-forwarded-empty | opencode | 15.06s | 21.64s | 6/6 |
| astro | astro-forwarded-empty | pi | 13.09s | 14.26s | 6/6 |
| astro | astro-forwarded-empty | claude | 10.89s | 9.97s | 6/6 |
| astro | astro-forwarded-empty | codex | 17.33s | 13.59s | 6/6 |
| httpx | httpx-reason-fallback | pi | 8.41s | 8.68s | 6/6 |
| httpx | httpx-reason-fallback | claude | 4.84s | 5.91s | 6/6 |
| httpx | httpx-reason-fallback | codex | 10.36s | 10.50s | 6/6 |
| httpx | httpx-reason-fallback | opencode | 10.51s | 10.14s | 6/6 |
| ripgrep | ripgrep-capture-hyphen | claude | 7.49s | 9.74s | 6/6 |
| ripgrep | ripgrep-capture-hyphen | codex | 12.72s | 11.85s | 6/6 |
| ripgrep | ripgrep-capture-hyphen | opencode | 12.60s | 10.94s | 6/6 |
| ripgrep | ripgrep-capture-hyphen | pi | 10.13s | 10.56s | 6/6 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
