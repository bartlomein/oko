# BCY Budget Curve

- Generated at: `2026-09-22T09:49:52+00:00`
- Corpus manifest: `baselines/corpus_manifest.jsonl`
- Tokenizer: `regex_code_tokenizer_v1`
- Protocol: file-deduplicate ranked results, render canonical corpus file text with a path header, greedily pack by rank, and prefix-truncate at the budget boundary.
- Canonical coverage: a gold file counts when at least one non-header content token is packed.
- Sensitivity coverage: at threshold tau, require at least min(tau, available file-content tokens).
- Caveat: this report uses released corpus `kind=file` text and stored top-20 file lists.

## Overall Curve

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 3 | oko | 345 | 0.3831 | 0.4755 | 0.5875 | 0.6416 |

## code2test

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 3 | oko | 106 | 0.2956 | 0.3950 | 0.5113 | 0.5774 |

## comment2context

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 3 | oko | 80 | 0.2938 | 0.3708 | 0.4646 | 0.5104 |

## trace2code

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 3 | oko | 101 | 0.6601 | 0.7195 | 0.7871 | 0.8234 |

## edit2ripple

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 3 | oko | 58 | 0.1839 | 0.3420 | 0.5489 | 0.6236 |

## Packing Diagnostics

| Method | Used@8k | Packed files@8k | Partial files@8k | Missing ranked files@8k |
|---|---:|---:|---:|---:|
| run 3 | 7998.3681 | 6.6174 | 0.9884 | 0.0000 |

## Minimum-content Threshold Sensitivity at 8k

| Method | tau=1 | tau=16 | tau=32 | tau=64 | tau=128 |
|---|---:|---:|---:|---:|---:|
| run 3 | 0.4755 | 0.4755 | 0.4755 | 0.4745 | 0.4745 |
