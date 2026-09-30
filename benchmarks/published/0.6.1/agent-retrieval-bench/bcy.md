# BCY Budget Curve

- Generated at: `2026-09-30T11:10:43+00:00`
- Corpus manifest: `baselines/corpus_manifest.jsonl`
- Tokenizer: `regex_code_tokenizer_v1`
- Protocol: file-deduplicate ranked results, render canonical corpus file text with a path header, greedily pack by rank, and prefix-truncate at the budget boundary.
- Canonical coverage: a gold file counts when at least one non-header content token is packed.
- Sensitivity coverage: at threshold tau, require at least min(tau, available file-content tokens).
- Caveat: this report uses released corpus `kind=file` text and stored top-20 file lists.

## Overall Curve

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.6 run 3 | oko | 345 | 0.3845 | 0.5051 | 0.6229 | 0.6940 |
| 2 | Oko 0.6 run 2 | oko | 345 | 0.3768 | 0.5022 | 0.6213 | 0.6971 |
| 3 | Oko 0.6.1 run 2 | oko | 345 | 0.3831 | 0.5022 | 0.6034 | 0.6768 |
| 4 | Oko 0.6.1 run 3 | oko | 345 | 0.3778 | 0.5022 | 0.5971 | 0.6836 |
| 5 | Oko 0.6.1 run 1 | oko | 345 | 0.3816 | 0.4983 | 0.6027 | 0.6773 |
| 6 | Oko 0.6 run 1 | oko | 345 | 0.3831 | 0.4913 | 0.6164 | 0.6930 |
| 7 | RepoMap (local) | repo map | 345 | 0.2019 | 0.3837 | 0.5251 | 0.6256 |
| 8 | Lexical (local) | lexical | 345 | 0.1488 | 0.2650 | 0.3886 | 0.4882 |
| 9 | BM25 (local) | lexical | 345 | 0.1220 | 0.2051 | 0.3184 | 0.4365 |

## code2test

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.6 run 2 | oko | 106 | 0.2940 | 0.4292 | 0.5692 | 0.6667 |
| 2 | Oko 0.6 run 3 | oko | 106 | 0.2987 | 0.4245 | 0.5833 | 0.6667 |
| 3 | Oko 0.6.1 run 3 | oko | 106 | 0.2862 | 0.4182 | 0.5550 | 0.6509 |
| 4 | Oko 0.6 run 1 | oko | 106 | 0.2940 | 0.4151 | 0.5550 | 0.6478 |
| 5 | Oko 0.6.1 run 1 | oko | 106 | 0.2925 | 0.3994 | 0.5597 | 0.6557 |
| 6 | Oko 0.6.1 run 2 | oko | 106 | 0.3035 | 0.3915 | 0.5314 | 0.6321 |
| 7 | RepoMap (local) | repo map | 106 | 0.2311 | 0.3541 | 0.4676 | 0.5431 |
| 8 | BM25 (local) | lexical | 106 | 0.0739 | 0.1541 | 0.2453 | 0.2972 |
| 9 | Lexical (local) | lexical | 106 | 0.0362 | 0.0928 | 0.1619 | 0.2469 |

## comment2context

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.6 run 1 | oko | 80 | 0.2062 | 0.3354 | 0.4479 | 0.5354 |
| 2 | Oko 0.6 run 2 | oko | 80 | 0.2062 | 0.3271 | 0.4458 | 0.5292 |
| 3 | Oko 0.6 run 3 | oko | 80 | 0.2250 | 0.3229 | 0.4479 | 0.5229 |
| 4 | Oko 0.6.1 run 3 | oko | 80 | 0.2167 | 0.3208 | 0.4271 | 0.5604 |
| 5 | Oko 0.6.1 run 1 | oko | 80 | 0.2167 | 0.3146 | 0.4354 | 0.5417 |
| 6 | Oko 0.6.1 run 2 | oko | 80 | 0.2125 | 0.3083 | 0.4625 | 0.5542 |
| 7 | RepoMap (local) | repo map | 80 | 0.1271 | 0.2979 | 0.3812 | 0.4958 |
| 8 | BM25 (local) | lexical | 80 | 0.1875 | 0.2604 | 0.4354 | 0.5292 |
| 9 | Lexical (local) | lexical | 80 | 0.1396 | 0.2167 | 0.3833 | 0.4854 |

## trace2code

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Oko 0.6 run 3 | oko | 101 | 0.7096 | 0.8119 | 0.8548 | 0.8894 |
| 2 | Oko 0.6.1 run 2 | oko | 101 | 0.7145 | 0.8069 | 0.8284 | 0.8614 |
| 3 | Oko 0.6.1 run 1 | oko | 101 | 0.7112 | 0.7970 | 0.8399 | 0.8581 |
| 4 | Oko 0.6 run 2 | oko | 101 | 0.7096 | 0.7937 | 0.8416 | 0.9010 |
| 5 | Oko 0.6.1 run 3 | oko | 101 | 0.7063 | 0.7921 | 0.8300 | 0.8614 |
| 6 | Oko 0.6 run 1 | oko | 101 | 0.7046 | 0.7723 | 0.8482 | 0.8960 |
| 7 | RepoMap (local) | repo map | 101 | 0.2195 | 0.5000 | 0.7096 | 0.8366 |
| 8 | Lexical (local) | lexical | 101 | 0.1749 | 0.3795 | 0.5561 | 0.6865 |
| 9 | BM25 (local) | lexical | 101 | 0.1205 | 0.1931 | 0.3069 | 0.4934 |

## edit2ripple

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Lexical (local) | lexical | 58 | 0.3218 | 0.4468 | 0.5187 | 0.5876 |
| 2 | Oko 0.6.1 run 2 | oko | 58 | 0.1868 | 0.4411 | 0.5374 | 0.6063 |
| 3 | Oko 0.6.1 run 1 | oko | 58 | 0.1983 | 0.4124 | 0.4986 | 0.5891 |
| 4 | Oko 0.6.1 run 3 | oko | 58 | 0.1954 | 0.4009 | 0.5029 | 0.6034 |
| 5 | Oko 0.6 run 2 | oko | 58 | 0.1839 | 0.3693 | 0.5747 | 0.6293 |
| 6 | Oko 0.6 run 3 | oko | 58 | 0.1954 | 0.3693 | 0.5330 | 0.6394 |
| 7 | Oko 0.6 run 1 | oko | 58 | 0.2299 | 0.3563 | 0.5575 | 0.6394 |
| 8 | RepoMap (local) | repo map | 58 | 0.2213 | 0.3534 | 0.5072 | 0.5876 |
| 9 | BM25 (local) | lexical | 58 | 0.1221 | 0.2428 | 0.3103 | 0.4641 |

## Packing Diagnostics

| Method | Used@8k | Packed files@8k | Partial files@8k | Missing ranked files@8k |
|---|---:|---:|---:|---:|
| Oko 0.6 run 3 | 7995.8725 | 6.6783 | 0.9797 | 0.0000 |
| Oko 0.6 run 2 | 7997.4957 | 6.7188 | 0.9884 | 0.0000 |
| Oko 0.6.1 run 2 | 7995.1739 | 6.6986 | 0.9768 | 0.0000 |
| Oko 0.6.1 run 3 | 7995.4464 | 6.6667 | 0.9826 | 0.0000 |
| Oko 0.6.1 run 1 | 7994.6464 | 6.6754 | 0.9797 | 0.0000 |
| Oko 0.6 run 1 | 7994.6406 | 6.6580 | 0.9826 | 0.0000 |
| RepoMap (local) | 7850.1101 | 7.4174 | 0.9478 | 0.0000 |
| Lexical (local) | 7997.7362 | 6.7913 | 0.9797 | 0.0000 |
| BM25 (local) | 7997.8928 | 6.0464 | 0.9913 | 0.0000 |

## Minimum-content Threshold Sensitivity at 8k

| Method | tau=1 | tau=16 | tau=32 | tau=64 | tau=128 |
|---|---:|---:|---:|---:|---:|
| Oko 0.6 run 3 | 0.5051 | 0.5051 | 0.5051 | 0.5022 | 0.5012 |
| Oko 0.6 run 2 | 0.5022 | 0.5022 | 0.5022 | 0.5022 | 0.5012 |
| Oko 0.6.1 run 2 | 0.5022 | 0.5022 | 0.5022 | 0.5022 | 0.5002 |
| Oko 0.6.1 run 3 | 0.5022 | 0.5022 | 0.5012 | 0.4983 | 0.4983 |
| Oko 0.6.1 run 1 | 0.4983 | 0.4983 | 0.4973 | 0.4973 | 0.4949 |
| Oko 0.6 run 1 | 0.4913 | 0.4913 | 0.4913 | 0.4884 | 0.4845 |
| RepoMap (local) | 0.3837 | 0.3837 | 0.3837 | 0.3837 | 0.3837 |
| Lexical (local) | 0.2650 | 0.2650 | 0.2650 | 0.2621 | 0.2592 |
| BM25 (local) | 0.2051 | 0.2051 | 0.2051 | 0.2051 | 0.2041 |
