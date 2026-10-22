# BCY Budget Curve

- Generated at: `2026-10-22T08:44:53+00:00`
- Corpus manifest: `/var/folders/_k/r9207j096m1bt66_qyrrh7d40000gn/T/tmpby02l0qs/corpus_manifest.jsonl`
- Tokenizer: `regex_code_tokenizer_v1`
- Protocol: file-deduplicate ranked results, render canonical corpus file text with a path header, greedily pack by rank, and prefix-truncate at the budget boundary.
- Canonical coverage: a gold file counts when at least one non-header content token is packed.
- Sensitivity coverage: at threshold tau, require at least min(tau, available file-content tokens).
- Caveat: this report uses released corpus `kind=file` text and stored top-20 file lists.

## Overall Curve

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.9.1 run 2 | oko | 345 | 0.3923 | 0.5365 | 0.6415 | 0.7019 |
| 2 | Oko 0.9.1 run 3 | oko | 345 | 0.4029 | 0.5338 | 0.6396 | 0.7029 |
| 3 | Oko 0.9.1 run 1 | oko | 345 | 0.4034 | 0.5275 | 0.6425 | 0.6976 |
| 4 | Oko 0.5.0 run 2 | oko | 345 | 0.3937 | 0.4887 | 0.5837 | 0.6434 |
| 5 | Oko 0.5.0 run 1 | oko | 345 | 0.3913 | 0.4786 | 0.5914 | 0.6369 |
| 6 | Oko 0.5.0 run 3 | oko | 345 | 0.3831 | 0.4755 | 0.5875 | 0.6416 |

## code2test

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.9.1 run 1 | oko | 106 | 0.3381 | 0.4811 | 0.6305 | 0.6619 |
| 2 | Oko 0.9.1 run 3 | oko | 106 | 0.3286 | 0.4780 | 0.6022 | 0.6572 |
| 3 | Oko 0.9.1 run 2 | oko | 106 | 0.3050 | 0.4764 | 0.6164 | 0.6478 |
| 4 | Oko 0.5.0 run 1 | oko | 106 | 0.3066 | 0.4264 | 0.5208 | 0.5651 |
| 5 | Oko 0.5.0 run 2 | oko | 106 | 0.3365 | 0.4217 | 0.5113 | 0.5651 |
| 6 | Oko 0.5.0 run 3 | oko | 106 | 0.2956 | 0.3950 | 0.5113 | 0.5774 |

## comment2context

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.5.0 run 2 | oko | 80 | 0.2625 | 0.3771 | 0.4688 | 0.5292 |
| 2 | Oko 0.5.0 run 3 | oko | 80 | 0.2938 | 0.3708 | 0.4646 | 0.5104 |
| 3 | Oko 0.5.0 run 1 | oko | 80 | 0.2854 | 0.3583 | 0.4750 | 0.5250 |
| 4 | Oko 0.9.1 run 3 | oko | 80 | 0.2271 | 0.3521 | 0.4521 | 0.5625 |
| 5 | Oko 0.9.1 run 2 | oko | 80 | 0.2333 | 0.3458 | 0.4458 | 0.5875 |
| 6 | Oko 0.9.1 run 1 | oko | 80 | 0.2375 | 0.3271 | 0.4458 | 0.5479 |

## trace2code

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.9.1 run 2 | oko | 101 | 0.7178 | 0.8300 | 0.8729 | 0.9026 |
| 2 | Oko 0.9.1 run 3 | oko | 101 | 0.7244 | 0.8300 | 0.8680 | 0.9026 |
| 3 | Oko 0.9.1 run 1 | oko | 101 | 0.7228 | 0.8152 | 0.8729 | 0.8927 |
| 4 | Oko 0.5.0 run 2 | oko | 101 | 0.6650 | 0.7294 | 0.7822 | 0.8284 |
| 5 | Oko 0.5.0 run 3 | oko | 101 | 0.6601 | 0.7195 | 0.7871 | 0.8234 |
| 6 | Oko 0.5.0 run 1 | oko | 101 | 0.6601 | 0.7096 | 0.7871 | 0.8185 |

## edit2ripple

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.9.1 run 2 | oko | 58 | 0.2040 | 0.3980 | 0.5546 | 0.6092 |
| 2 | Oko 0.9.1 run 1 | oko | 58 | 0.1954 | 0.3879 | 0.5345 | 0.6293 |
| 3 | Oko 0.9.1 run 3 | oko | 58 | 0.2213 | 0.3707 | 0.5690 | 0.6322 |
| 4 | Oko 0.5.0 run 2 | oko | 58 | 0.2069 | 0.3463 | 0.5287 | 0.6221 |
| 5 | Oko 0.5.0 run 3 | oko | 58 | 0.1839 | 0.3420 | 0.5489 | 0.6236 |
| 6 | Oko 0.5.0 run 1 | oko | 58 | 0.2241 | 0.3376 | 0.5402 | 0.6063 |

## Packing Diagnostics

| Method | Used@8k | Packed files@8k | Partial files@8k | Missing ranked files@8k |
|---|---:|---:|---:|---:|
| Oko 0.9.1 run 2 | 7993.9681 | 6.6493 | 0.9826 | 0.0000 |
| Oko 0.9.1 run 3 | 7994.9565 | 6.6609 | 0.9855 | 0.0000 |
| Oko 0.9.1 run 1 | 7994.6783 | 6.6116 | 0.9826 | 0.0000 |
| Oko 0.5.0 run 2 | 7997.3884 | 6.6522 | 0.9826 | 0.0000 |
| Oko 0.5.0 run 1 | 7997.0638 | 6.7101 | 0.9855 | 0.0000 |
| Oko 0.5.0 run 3 | 7998.3681 | 6.6174 | 0.9884 | 0.0000 |

## Minimum-content Threshold Sensitivity at 8k

| Method | tau=1 | tau=16 | tau=32 | tau=64 | tau=128 |
|---|---:|---:|---:|---:|---:|
| Oko 0.9.1 run 2 | 0.5365 | 0.5365 | 0.5365 | 0.5365 | 0.5348 |
| Oko 0.9.1 run 3 | 0.5338 | 0.5338 | 0.5338 | 0.5309 | 0.5300 |
| Oko 0.9.1 run 1 | 0.5275 | 0.5275 | 0.5275 | 0.5275 | 0.5266 |
| Oko 0.5.0 run 2 | 0.4887 | 0.4887 | 0.4887 | 0.4878 | 0.4849 |
| Oko 0.5.0 run 1 | 0.4786 | 0.4786 | 0.4786 | 0.4786 | 0.4776 |
| Oko 0.5.0 run 3 | 0.4755 | 0.4755 | 0.4755 | 0.4745 | 0.4745 |
