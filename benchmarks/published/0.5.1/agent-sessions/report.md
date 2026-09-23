# Public repository benchmark

Sessions: 243/243. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | native | 26/27 | 32.90 | 1882546 |
| codex | current | 27/27 | 30.98 | 1501718 |
| codex | guided | 25/27 | 30.32 | 1344825 |
| opencode | native | 27/27 | 31.36 | 801218 |
| opencode | current | 27/27 | 26.10 | 472020 |
| opencode | guided | 27/27 | 26.51 | 447545 |
| claude | native | 25/27 | 8.48 | 619552 |
| claude | current | 27/27 | 6.89 | 505681 |
| claude | guided | 27/27 | 6.00 | 541945 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|
| codex | native | 456948 | 1406592 | 0 | 19006 | 0 / 0 | 3 | unavailable | 0 |
| codex | current | 307889 | 1176448 | 0 | 17381 | unavailable / unavailable | 3 | unavailable | 0.687 |
| codex | guided | 296671 | 1032832 | 0 | 15322 | 1010677 / 54709 | 2 | unavailable | 0.72 |
| opencode | native | 355122 | 429568 | 0 | 16528 | 0 / 0 | 6 | 4 | 0 |
| opencode | current | 239831 | 219520 | 0 | 12669 | 832913 / 45195 | 3 | 3 | 0.539 |
| opencode | guided | 216493 | 219136 | 0 | 11916 | 872762 / 47739 | 2 | 3 | 0.675 |
| claude | native | 192 | 486746 | 113187 | 19427 | 0 / 0 | 3 | 4 | 0 |
| claude | current | 144 | 387705 | 105632 | 12200 | 819019 / 45309 | 2 | 3 | 0.673 |
| claude | guided | 144 | 423691 | 107074 | 11036 | 819764 / 45328 | 2 | 3 | 0.615 |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | current | guided | Passed/attempted |
|---|---|---|---:|---:|---:|---:|
| astro | astro-image-probe-authorization | codex | 28.40s | 30.42s | 19.30s | 9/9 |
| astro | astro-image-probe-authorization | opencode | 31.36s | 18.18s | 15.35s | 9/9 |
| astro | astro-image-probe-authorization | claude | 7.68s | 4.65s | 4.32s | 9/9 |
| httpx | httpx-decoder-chain | opencode | 28.46s | 28.29s | 26.51s | 9/9 |
| httpx | httpx-decoder-chain | claude | 7.69s | 6.98s | 6.34s | 9/9 |
| httpx | httpx-decoder-chain | codex | 29.51s | 29.48s | 40.65s | 9/9 |
| ripgrep | ripgrep-capture-expansion | claude | 6.34s | 4.25s | 3.79s | 9/9 |
| ripgrep | ripgrep-capture-expansion | codex | 39.69s | 31.62s | 35.58s | 9/9 |
| ripgrep | ripgrep-capture-expansion | opencode | 37.75s | 34.71s | 23.32s | 9/9 |
| astro | astro-action-key-guards | opencode | 31.29s | 26.10s | 23.74s | 9/9 |
| astro | astro-action-key-guards | claude | 8.48s | 4.32s | 4.08s | 9/9 |
| astro | astro-action-key-guards | codex | 30.78s | 22.81s | 19.71s | 9/9 |
| httpx | httpx-async-auth-body | claude | 7.69s | 3.98s | 5.41s | 9/9 |
| httpx | httpx-async-auth-body | codex | 29.45s | 25.12s | 28.94s | 9/9 |
| httpx | httpx-async-auth-body | opencode | 25.62s | 19.00s | 27.09s | 9/9 |
| ripgrep | ripgrep-printed-bytes | codex | 29.37s | 30.30s | 30.23s | 8/9 |
| ripgrep | ripgrep-printed-bytes | opencode | 31.12s | 24.66s | 22.63s | 9/9 |
| ripgrep | ripgrep-printed-bytes | claude | 9.34s | 6.30s | 6.00s | 8/9 |
| astro | astro-forwarded-empty | claude | 9.89s | 12.84s | 10.99s | 8/9 |
| astro | astro-forwarded-empty | codex | 40.42s | 53.89s | 44.54s | 7/9 |
| astro | astro-forwarded-empty | opencode | 44.37s | 44.42s | 39.29s | 9/9 |
| httpx | httpx-reason-fallback | codex | 31.03s | 36.23s | 30.32s | 9/9 |
| httpx | httpx-reason-fallback | opencode | 27.06s | 27.73s | 21.05s | 9/9 |
| httpx | httpx-reason-fallback | claude | 6.01s | 7.17s | 6.72s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | opencode | 37.50s | 32.71s | 29.73s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | claude | 10.43s | 9.16s | 9.21s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | codex | 38.40s | 42.11s | 38.48s | 9/9 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
| codex | native | 27 | -0.53 | -1.8% |
| codex | current -> guided | 27 | -3.57 | -11.8% |
| opencode | native | 27 | -5.00 | -16.7% |
| opencode | current -> guided | 27 | -1.78 | -6.3% |
| claude | native | 27 | -1.92 | -24.4% |
| claude | current -> guided | 27 | -0.33 | -6.3% |
