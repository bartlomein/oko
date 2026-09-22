# BCY Budget Curve

- Generated at: `2026-09-22T09:27:25+00:00`
- Corpus manifest: `baselines/corpus_manifest.jsonl`
- Tokenizer: `regex_code_tokenizer_v1`
- Protocol: file-deduplicate ranked results, render canonical corpus file text with a path header, greedily pack by rank, and prefix-truncate at the budget boundary.
- Canonical coverage: a gold file counts when at least one non-header content token is packed.
- Sensitivity coverage: at threshold tau, require at least min(tau, available file-content tokens).
- Caveat: this report uses released corpus `kind=file` text and stored top-20 file lists.

## Overall Curve

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko (second run) | oko | 345 | 0.3913 | 0.4786 | 0.5914 | 0.6369 |
| 2 | Oko (first run) | oko | 345 | 0.3118 | 0.3973 | 0.5029 | 0.6112 |
| 3 | RepoMap (local) | repo map | 345 | 0.2019 | 0.3837 | 0.5251 | 0.6256 |
| 4 | Lexical (local) | lexical | 345 | 0.1488 | 0.2650 | 0.3886 | 0.4882 |
| 5 | BM25 (local) | lexical | 345 | 0.1220 | 0.2051 | 0.3184 | 0.4365 |

## code2test

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko (second run) | oko | 106 | 0.3066 | 0.4264 | 0.5208 | 0.5651 |
| 2 | RepoMap (local) | repo map | 106 | 0.2311 | 0.3541 | 0.4676 | 0.5431 |
| 3 | BM25 (local) | lexical | 106 | 0.0739 | 0.1541 | 0.2453 | 0.2972 |
| 4 | Oko (first run) | oko | 106 | 0.0865 | 0.1525 | 0.2972 | 0.4783 |
| 5 | Lexical (local) | lexical | 106 | 0.0362 | 0.0928 | 0.1619 | 0.2469 |

## comment2context

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko (second run) | oko | 80 | 0.2854 | 0.3583 | 0.4750 | 0.5250 |
| 2 | RepoMap (local) | repo map | 80 | 0.1271 | 0.2979 | 0.3812 | 0.4958 |
| 3 | Oko (first run) | oko | 80 | 0.2354 | 0.2854 | 0.3937 | 0.4688 |
| 4 | BM25 (local) | lexical | 80 | 0.1875 | 0.2604 | 0.4354 | 0.5292 |
| 5 | Lexical (local) | lexical | 80 | 0.1396 | 0.2167 | 0.3833 | 0.4854 |

## trace2code

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko (first run) | oko | 101 | 0.6650 | 0.7492 | 0.7871 | 0.8531 |
| 2 | Oko (second run) | oko | 101 | 0.6601 | 0.7096 | 0.7871 | 0.8185 |
| 3 | RepoMap (local) | repo map | 101 | 0.2195 | 0.5000 | 0.7096 | 0.8366 |
| 4 | Lexical (local) | lexical | 101 | 0.1749 | 0.3795 | 0.5561 | 0.6865 |
| 5 | BM25 (local) | lexical | 101 | 0.1205 | 0.1931 | 0.3069 | 0.4934 |

## edit2ripple

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Lexical (local) | lexical | 58 | 0.3218 | 0.4468 | 0.5187 | 0.5876 |
| 2 | Oko (first run) | oko | 58 | 0.2141 | 0.3865 | 0.5345 | 0.6293 |
| 3 | RepoMap (local) | repo map | 58 | 0.2213 | 0.3534 | 0.5072 | 0.5876 |
| 4 | Oko (second run) | oko | 58 | 0.2241 | 0.3376 | 0.5402 | 0.6063 |
| 5 | BM25 (local) | lexical | 58 | 0.1221 | 0.2428 | 0.3103 | 0.4641 |

## Packing Diagnostics

| Method | Used@8k | Packed files@8k | Partial files@8k | Missing ranked files@8k |
|---|---:|---:|---:|---:|
| Oko (second run) | 7997.0638 | 6.7101 | 0.9855 | 0.0000 |
| Oko (first run) | 7996.9797 | 6.7275 | 0.9884 | 0.0000 |
| RepoMap (local) | 7850.1101 | 7.4174 | 0.9478 | 0.0000 |
| Lexical (local) | 7997.7362 | 6.7913 | 0.9797 | 0.0000 |
| BM25 (local) | 7997.8928 | 6.0464 | 0.9913 | 0.0000 |

## Minimum-content Threshold Sensitivity at 8k

| Method | tau=1 | tau=16 | tau=32 | tau=64 | tau=128 |
|---|---:|---:|---:|---:|---:|
| Oko (second run) | 0.4786 | 0.4786 | 0.4786 | 0.4786 | 0.4776 |
| Oko (first run) | 0.3973 | 0.3973 | 0.3973 | 0.3973 | 0.3973 |
| RepoMap (local) | 0.3837 | 0.3837 | 0.3837 | 0.3837 | 0.3837 |
| Lexical (local) | 0.2650 | 0.2650 | 0.2650 | 0.2621 | 0.2592 |
| BM25 (local) | 0.2051 | 0.2051 | 0.2051 | 0.2051 | 0.2041 |
