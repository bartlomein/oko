# Public repository benchmark

Sessions: 243/243. Complete: True.
Suite: branch; repeats: 3; cache policy: warm.

| Client | Condition | Passed/attempted | Median seconds (all attempts) | Agent tokens (derived) |
|---|---|---:|---:|---:|
| codex | native | 27/27 | 24.15 | 1903829 |
| codex | current | 27/27 | 20.57 | 1409472 |
| codex | guided | 26/27 | 18.89 | 1317970 |
| opencode | native | 26/27 | 23.39 | 794584 |
| opencode | current | 25/27 | 18.94 | 489541 |
| opencode | guided | 26/27 | 18.47 | 523121 |
| claude | native | 23/27 | 11.66 | 1657359 |
| claude | current | 24/27 | 7.63 | 1114561 |
| claude | guided | 22/27 | 7.17 | 993596 |

All attempts, including failed grades, remain in timings. Compare within each client: models differ across clients.
Agent token totals are derived from recorded components, including provider cache reads/writes. Raw provider totals remain separate. Jev usage is additional and excluded from agent totals. These are not cost estimates.
Fresh checkout and conversation per session; provider prompt caches are not cleared. Warm means a prebuilt disk index, not cached answers or a persistent MCP process. Offline warm-up is excluded from agent latency.
Focused module checks do not establish full application correctness. Repeated trials are paired by task and repetition; task diversity is still limited.

## Components

| Client | Condition | Uncached input | Cache reads | Cache writes | Output | Jev input / output | Median tools | Median model rounds | Median Oko seconds |
|---|---|---:|---:|---:|---:|---|---:|---:|---:|
| codex | native | 442390 | 1441408 | 0 | 20031 | 0 / 0 | 3 | unavailable | 0 |
| codex | current | 287994 | 1105152 | 0 | 16326 | 1356272 / 93930 | 2 | unavailable | 0.611 |
| codex | guided | 266936 | 1036800 | 0 | 14234 | 1594436 / 114741 | 2 | unavailable | 0.462 |
| opencode | native | 333505 | 445696 | 0 | 15383 | 0 / 0 | 5 | 4 | 0 |
| opencode | current | 238694 | 239488 | 0 | 11359 | 1334345 / 91019 | 3 | 4 | 0.502 |
| opencode | guided | 256221 | 256000 | 0 | 10900 | unavailable / unavailable | 2 | 3 | 0.527 |
| claude | native | 300 | 1473325 | 154162 | 29572 | 0 / 0 | 4 | 5 | 0 |
| claude | current | 186 | 956856 | 140202 | 17317 | 1306729 / 89409 | 2 | 3 | 0.433 |
| claude | guided | 154 | 836671 | 143333 | 13438 | 1415986 / 100816 | 2 | 3 | 0.453 |

Oko and Jev durations are nested work, not additive to agent wall time. Startup preparation, Jev durations, and offline warm-up are separate fields in JSON. Unknowns remain unavailable; Codex turn events do not expose model-round counts.

## Per-task medians

| Repository | Task | Client | native | current | guided | Passed/attempted |
|---|---|---|---:|---:|---:|---:|
| astro | astro-image-probe-authorization | codex | 17.61s | 14.22s | 15.34s | 9/9 |
| astro | astro-image-probe-authorization | opencode | 26.16s | 12.52s | 12.16s | 9/9 |
| astro | astro-image-probe-authorization | claude | 8.74s | 7.52s | 8.36s | 9/9 |
| httpx | httpx-decoder-chain | opencode | 18.18s | 20.92s | 23.02s | 9/9 |
| httpx | httpx-decoder-chain | claude | 11.08s | 6.57s | 4.40s | 9/9 |
| httpx | httpx-decoder-chain | codex | 22.25s | 19.71s | 22.06s | 9/9 |
| ripgrep | ripgrep-capture-expansion | claude | 11.77s | 11.06s | 4.65s | 9/9 |
| ripgrep | ripgrep-capture-expansion | codex | 27.53s | 26.36s | 15.50s | 9/9 |
| ripgrep | ripgrep-capture-expansion | opencode | 27.03s | 14.98s | 18.47s | 9/9 |
| astro | astro-action-key-guards | opencode | 24.86s | 17.45s | 18.63s | 9/9 |
| astro | astro-action-key-guards | claude | 10.42s | 5.56s | 6.50s | 7/9 |
| astro | astro-action-key-guards | codex | 24.15s | 17.24s | 15.83s | 9/9 |
| httpx | httpx-async-auth-body | claude | 14.09s | 5.40s | 4.99s | 8/9 |
| httpx | httpx-async-auth-body | codex | 18.65s | 15.45s | 13.23s | 9/9 |
| httpx | httpx-async-auth-body | opencode | 14.96s | 20.63s | 10.39s | 9/9 |
| ripgrep | ripgrep-printed-bytes | codex | 21.48s | 22.64s | 20.17s | 9/9 |
| ripgrep | ripgrep-printed-bytes | opencode | 20.07s | 19.36s | 15.40s | 9/9 |
| ripgrep | ripgrep-printed-bytes | claude | 11.66s | 12.14s | 8.18s | 9/9 |
| astro | astro-forwarded-empty | claude | 12.69s | 18.75s | 12.97s | 5/9 |
| astro | astro-forwarded-empty | codex | 29.35s | 32.14s | 27.94s | 8/9 |
| astro | astro-forwarded-empty | opencode | 28.67s | 27.32s | 27.06s | 5/9 |
| httpx | httpx-reason-fallback | codex | 21.74s | 20.57s | 19.22s | 9/9 |
| httpx | httpx-reason-fallback | opencode | 20.40s | 16.86s | 14.62s | 9/9 |
| httpx | httpx-reason-fallback | claude | 9.51s | 9.18s | 7.17s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | opencode | 24.58s | 21.43s | 20.48s | 9/9 |
| ripgrep | ripgrep-capture-hyphen | claude | 29.99s | 8.24s | 9.90s | 4/9 |
| ripgrep | ripgrep-capture-hyphen | codex | 25.30s | 35.05s | 23.70s | 9/9 |

## Paired current-build changes

Each pair has the same task, client, and repetition. Negative means current Oko was faster. Failed grades are retained; these are descriptive measurements, not significance tests.

| Client | Baseline | Pairs | Median seconds change | Median percent change |
|---|---|---:|---:|---:|
| codex | native | 27 | -1.35 | -6.6% |
| codex | current -> guided | 27 | -1.62 | -8.4% |
| opencode | native | 27 | -4.03 | -15.9% |
| opencode | current -> guided | 27 | -0.47 | -2.5% |
| claude | native | 27 | -3.82 | -34.2% |
| claude | current -> guided | 27 | -0.70 | -3.7% |

Search answers with text before their JSON, graded on the JSON: 5.
