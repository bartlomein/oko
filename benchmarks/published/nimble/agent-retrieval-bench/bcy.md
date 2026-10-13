# BCY Budget Curve

- Generated at: `2026-10-13T01:10:47+00:00`
- Corpus manifest: the four task types' released `corpus_manifest.jsonl`, merged
- Tokenizer: `regex_code_tokenizer_v1`
- Protocol: file-deduplicate ranked results, render canonical corpus file text with a path header, greedily pack by rank, and prefix-truncate at the budget boundary.
- Canonical coverage: a gold file counts when at least one non-header content token is packed.
- Sensitivity coverage: at threshold tau, require at least min(tau, available file-content tokens).
- Caveat: this report uses released corpus `kind=file` text and stored top-20 file lists.

## Overall Curve

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.7.0 + Jev, run 2 | oko | 345 | 0.3870 | 0.5017 | 0.5973 | 0.6729 |
| 2 | Oko + Jev, 12 Oct | oko | 345 | 0.3894 | 0.4944 | 0.6029 | 0.6860 |
| 3 | Oko 0.7.0 + Jev, run 3 | oko | 345 | 0.3903 | 0.4906 | 0.6143 | 0.6816 |
| 4 | Oko 0.7.0 + Jev, run 1 | oko | 345 | 0.3802 | 0.4891 | 0.5935 | 0.6802 |
| 5 | Oko + Nimble, 12 Oct | oko | 345 | 0.3386 | 0.4696 | 0.5903 | 0.6725 |

## code2test

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko + Nimble, 12 Oct | oko | 106 | 0.3491 | 0.4733 | 0.6006 | 0.6667 |
| 2 | Oko 0.7.0 + Jev, run 2 | oko | 106 | 0.3113 | 0.4151 | 0.5519 | 0.6289 |
| 3 | Oko 0.7.0 + Jev, run 1 | oko | 106 | 0.3035 | 0.4041 | 0.5362 | 0.6415 |
| 4 | Oko + Jev, 12 Oct | oko | 106 | 0.3050 | 0.3994 | 0.5314 | 0.6509 |
| 5 | Oko 0.7.0 + Jev, run 3 | oko | 106 | 0.2909 | 0.3899 | 0.5739 | 0.6431 |

## comment2context

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.7.0 + Jev, run 2 | oko | 80 | 0.2167 | 0.3271 | 0.4292 | 0.5354 |
| 2 | Oko 0.7.0 + Jev, run 3 | oko | 80 | 0.2292 | 0.3208 | 0.4562 | 0.5542 |
| 3 | Oko + Jev, 12 Oct | oko | 80 | 0.2167 | 0.3146 | 0.4604 | 0.5604 |
| 4 | Oko + Nimble, 12 Oct | oko | 80 | 0.2000 | 0.3083 | 0.4333 | 0.5687 |
| 5 | Oko 0.7.0 + Jev, run 1 | oko | 80 | 0.1917 | 0.2979 | 0.4313 | 0.5375 |

## trace2code

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.7.0 + Jev, run 3 | oko | 101 | 0.7360 | 0.8020 | 0.8432 | 0.8713 |
| 2 | Oko + Jev, 12 Oct | oko | 101 | 0.7261 | 0.7970 | 0.8432 | 0.8663 |
| 3 | Oko 0.7.0 + Jev, run 2 | oko | 101 | 0.7013 | 0.7970 | 0.8251 | 0.8663 |
| 4 | Oko 0.7.0 + Jev, run 1 | oko | 101 | 0.7162 | 0.7871 | 0.8399 | 0.8713 |
| 5 | Oko + Nimble, 12 Oct | oko | 101 | 0.5182 | 0.6683 | 0.7706 | 0.8317 |

## edit2ripple

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko + Jev, 12 Oct | oko | 58 | 0.1954 | 0.3894 | 0.5115 | 0.6092 |
| 2 | Oko 0.7.0 + Jev, run 1 | oko | 58 | 0.1954 | 0.3894 | 0.4928 | 0.6149 |
| 3 | Oko 0.7.0 + Jev, run 2 | oko | 58 | 0.2126 | 0.3865 | 0.5158 | 0.6063 |
| 4 | Oko 0.7.0 + Jev, run 3 | oko | 58 | 0.1925 | 0.3664 | 0.5072 | 0.5977 |
| 5 | Oko + Nimble, 12 Oct | oko | 58 | 0.1983 | 0.3391 | 0.4741 | 0.5489 |

## Packing Diagnostics

| Method | Used@8k | Packed files@8k | Partial files@8k | Missing ranked files@8k |
|---|---:|---:|---:|---:|
| Oko 0.7.0 + Jev, run 2 | 7977.8261 | 6.6638 | 0.9739 | 0.0000 |
| Oko + Jev, 12 Oct | 7993.2493 | 6.6899 | 0.9768 | 0.0000 |
| Oko 0.7.0 + Jev, run 3 | 7994.5623 | 6.6551 | 0.9797 | 0.0000 |
| Oko 0.7.0 + Jev, run 1 | 7995.7217 | 6.7014 | 0.9826 | 0.0000 |
| Oko + Nimble, 12 Oct | 7993.1652 | 6.5913 | 0.9884 | 0.0000 |

## Minimum-content Threshold Sensitivity at 8k

| Method | tau=1 | tau=16 | tau=32 | tau=64 | tau=128 |
|---|---:|---:|---:|---:|---:|
| Oko 0.7.0 + Jev, run 2 | 0.5017 | 0.5017 | 0.5007 | 0.5007 | 0.4993 |
| Oko + Jev, 12 Oct | 0.4944 | 0.4944 | 0.4935 | 0.4935 | 0.4935 |
| Oko 0.7.0 + Jev, run 3 | 0.4906 | 0.4906 | 0.4896 | 0.4896 | 0.4896 |
| Oko 0.7.0 + Jev, run 1 | 0.4891 | 0.4891 | 0.4882 | 0.4882 | 0.4857 |
| Oko + Nimble, 12 Oct | 0.4696 | 0.4696 | 0.4696 | 0.4686 | 0.4657 |
