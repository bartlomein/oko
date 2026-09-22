# BCY Budget Curve

- Generated at: `2026-09-22T09:49:31+00:00`
- Corpus manifest: `baselines/corpus_manifest.jsonl`
- Tokenizer: `regex_code_tokenizer_v1`
- Protocol: file-deduplicate ranked results, render canonical corpus file text with a path header, greedily pack by rank, and prefix-truncate at the budget boundary.
- Canonical coverage: a gold file counts when at least one non-header content token is packed.
- Sensitivity coverage: at threshold tau, require at least min(tau, available file-content tokens).
- Caveat: this report uses released corpus `kind=file` text and stored top-20 file lists.

## Overall Curve

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 2 | oko | 345 | 0.3937 | 0.4887 | 0.5837 | 0.6434 |

## code2test

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 2 | oko | 106 | 0.3365 | 0.4217 | 0.5113 | 0.5651 |

## comment2context

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 2 | oko | 80 | 0.2625 | 0.3771 | 0.4688 | 0.5292 |

## trace2code

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 2 | oko | 101 | 0.6650 | 0.7294 | 0.7822 | 0.8284 |

## edit2ripple

| Rank | Method | Family | Samples | BCY@4k | BCY@8k | BCY@16k | BCY@32k |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | run 2 | oko | 58 | 0.2069 | 0.3463 | 0.5287 | 0.6221 |

## Packing Diagnostics

| Method | Used@8k | Packed files@8k | Partial files@8k | Missing ranked files@8k |
|---|---:|---:|---:|---:|
| run 2 | 7997.3884 | 6.6522 | 0.9826 | 0.0000 |

## Minimum-content Threshold Sensitivity at 8k

| Method | tau=1 | tau=16 | tau=32 | tau=64 | tau=128 |
|---|---:|---:|---:|---:|---:|
| run 2 | 0.4887 | 0.4887 | 0.4887 | 0.4878 | 0.4849 |
