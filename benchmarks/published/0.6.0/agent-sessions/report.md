# Public repository benchmark

Sessions: 243/243. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | native | 26/27 | 24.15 | 2019604 |
| codex | current | 27/27 | 29.78 | 1660075 |
| codex | guided | 27/27 | 21.99 | 1469144 |
| opencode | native | 27/27 | 23.50 | 802759 |
| opencode | current | 27/27 | 27.02 | 654852 |
| opencode | guided | 27/27 | 21.81 | 610812 |
| claude | native | 23/27 | 7.39 | 909846 |
| claude | current | 25/27 | 6.82 | 647917 |
| claude | guided | 27/27 | 7.09 | 757810 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|
| codex | native | 444847 | 1555584 | 0 | 19173 | 0 / 0 | 3 | unavailable | 0 |
| codex | current | 326433 | 1315328 | 0 | 18314 | unavailable / unavailable | 3 | unavailable | 1.031 |
| codex | guided | 281321 | 1172736 | 0 | 15087 | 1817415 / 156652 | 2 | unavailable | 0.743 |
| opencode | native | 330935 | 455808 | 0 | 16016 | 0 / 0 | 6 | 4 | 0 |
| opencode | current | 257330 | 382976 | 0 | 14546 | 1679825 / 145284 | 4 | 4 | 1.023 |
| opencode | guided | 280813 | 317568 | 0 | 12431 | 1764889 / 153557 | 3 | 4 | 0.766 |
| claude | native | 228 | 761323 | 125409 | 22886 | 0 / 0 | 4 | 4 | 0 |
| claude | current | 160 | 511148 | 123719 | 12890 | unavailable / unavailable | 2 | 3 | 0.609 |
| claude | guided | 168 | 616578 | 127252 | 13812 | 929523 / 78937 | 2 | 3 | 0.436 |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | current | guided | Passed/attempted |
|---|---|---|---:|---:|---:|---:|
| astro | astro-image-probe-authorization | codex | 17.36s | 24.33s | 22.26s | 9/9 |
| astro | astro-image-probe-authorization | opencode | 19.60s | 20.05s | 27.44s | 9/9 |
| astro | astro-image-probe-authorization | claude | 8.07s | 8.21s | 10.08s | 9/9 |
| httpx | httpx-decoder-chain | opencode | 17.97s | 33.03s | 23.88s | 9/9 |
| httpx | httpx-decoder-chain | claude | 6.12s | 6.67s | 5.86s | 9/9 |
| httpx | httpx-decoder-chain | codex | 21.01s | 21.23s | 18.56s | 9/9 |
| ripgrep | ripgrep-capture-expansion | claude | 6.28s | 6.28s | 5.68s | 8/9 |
| ripgrep | ripgrep-capture-expansion | codex | 30.15s | 17.81s | 13.45s | 9/9 |
| ripgrep | ripgrep-capture-expansion | opencode | 28.35s | 28.93s | 19.38s | 9/9 |
| astro | astro-action-key-guards | opencode | 24.86s | 29.19s | 19.20s | 9/9 |
| astro | astro-action-key-guards | claude | 7.39s | 4.61s | 7.72s | 8/9 |
| astro | astro-action-key-guards | codex | 24.25s | 31.10s | 15.75s | 9/9 |
| httpx | httpx-async-auth-body | claude | 8.34s | 4.90s | 7.09s | 9/9 |
| httpx | httpx-async-auth-body | codex | 16.83s | 29.78s | 20.63s | 9/9 |
| httpx | httpx-async-auth-body | opencode | 19.99s | 14.56s | 16.89s | 9/9 |
| ripgrep | ripgrep-printed-bytes | codex | 21.95s | 24.73s | 21.99s | 9/9 |
| ripgrep | ripgrep-printed-bytes | opencode | 19.56s | 25.92s | 20.26s | 9/9 |
| ripgrep | ripgrep-printed-bytes | claude | 6.94s | 5.97s | 5.69s | 7/9 |
| astro | astro-forwarded-empty | claude | 8.63s | 9.02s | 8.59s | 7/9 |
| astro | astro-forwarded-empty | codex | 32.58s | 33.68s | 29.99s | 8/9 |
| astro | astro-forwarded-empty | opencode | 32.79s | 38.14s | 24.17s | 9/9 |
| httpx | httpx-reason-fallback | codex | 23.86s | 31.59s | 49.05s | 9/9 |
| httpx | httpx-reason-fallback | opencode | 25.48s | 34.62s | 22.85s | 9/9 |
| httpx | httpx-reason-fallback | claude | 5.31s | 7.20s | 7.38s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | opencode | 27.76s | 26.64s | 21.81s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | claude | 9.43s | 7.71s | 8.18s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | codex | 32.27s | 31.14s | 29.69s | 9/9 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
| codex | native | 27 | +2.40 | +10.4% |
| codex | current -> guided | 27 | -3.01 | -9.7% |
| opencode | native | 27 | +0.83 | +4.3% |
| opencode | current -> guided | 27 | -4.48 | -15.7% |
| claude | native | 27 | -0.96 | -13.9% |
| claude | current -> guided | 27 | +0.44 | +5.8% |

Search answers with text before their JSON, graded on the JSON: 1.
