# Jev-only investigation validation — 2026-09-18

The original deep mode did not improve aggregate location accuracy on this
Telemetry Studio fixture and is about 15.8 times slower at the median.
Keep ordinary search as the default; deep mode remains experimental. These are
the pre-optimization results; see the token-cache follow-up below.

| Metric | Ordinary search | Deep, maximum five steps |
|---|---:|---:|
| First-result implementation hit | 34/45 (75.6%) | 34/45 (75.6%) |
| Top-five implementation hit | 42/45 (93.3%) | 42/45 (93.3%) |
| Median elapsed seconds | 1.219 | 19.232 |
| Total Jev calls | 45 | 118 |
| Execution errors | 0 | 0 |

All 90 trials completed: 15 questions, three repeats per mode, sequential runs
with rotating mode order. Each trial starts a fresh release process. Timing
includes local processing and network calls. There is no warmup, cache clearing,
or automatic retry. Both modes use only Jev and the same executable. Codex and
OpenCode were not invoked. Function-range scores equal implementation-range
scores in this run.

Before/after checks confirm the source snapshot, executable and fixture stayed
unchanged. Source snapshots cover Git state and ripgrep-visible files up to
256 KiB, not every ignored or larger file. Expected source hashes matched before
the run. Raw source and credentials are not stored in the report.

## What the traces show

- The optional-values case missed in all three repeats in both modes. Deep
  search used four steps and eight calls each time, following derived-metrics
  and elevation-profile code before Jev chose to finish. It never returned the
  expected `lerp_option` implementation. A model-finished flag is not evidence
  of correctness.
- Deep mode stopped after the initial search in 39/45 trials. The other six
  trials investigated RaceBox parsing or optional-value interpolation. No trial
  hit the five-step cap, so increasing that cap alone would not change these
  observed stopping decisions.
- First-result gains on update-check and gapless-eligibility were offset by
  losses on disk-space and mach-speed. Aggregate top-five coverage did not
  change. These independent Jev responses can vary; the small differences do
  not establish a reliable benefit or regression for an individual question.
- Code inspection identifies avoidable local work: `search_actions` eagerly
  runs every generated query, and each `rank_lexically` call tokenizes the
  corpus again. This is a likely contributor to the slowdown; this run records
  total latency, not a CPU/network timing breakdown.

## Limits and next work

This is one repository and an already-used development fixture, not a held-out
general search benchmark. Location labels are conservative: plausible alternative
implementations can count as misses. Three repeats are not 45 independent
questions. Results do not establish accuracy for documentation or databases.

The synthetic smoke case recovered a target behind distracting documentation,
but that success did not generalize to the real optional-values case. Next work
should improve candidate actions and stopping decisions, reuse tokenized source
across queries, and repeat this benchmark plus fresh held-out questions.
No application behavior was changed during this validation.

## Reproduce and artifacts

```sh
cargo build --release --locked
node scripts/compare-investigation.mjs /path/to/telemetry-studio benchmarks/telemetry-studio-expanded.json
```

This sends repository snippets to Jev using `TYPESAFE_API_KEY` from the shell
or Oko's `.env`. No other model connection is required.

Local raw report (ignored by Git):
`benchmarks/results/deep-comparison-2026-09-18T14-06-54.861Z/report.json`.

Executable SHA-256:
`5291ec7e919a1d064c2556ae1a50fe5c1f77c56dafd689b425d27da18b955d30`.

Fixture SHA-256:
`8e3ac70f7e0215dc7a75dea7c078e6b20a3c033c3580902063cc396eb98e3232`.

The benchmark runner passed Node syntax validation and the nine shared harness
tests passed. The release build and whitespace checks passed.

## Token-cache optimization follow-up

Deep search now prepares normalized content/path word sets once per snapshot,
then reuses them for all generated queries and source-only filters. It borrows
chunk text rather than duplicating it in the cache. The temporary sets use extra
memory and are discarded after action generation. Ordinary search is unchanged.
No scoring, query-generation, candidate-limit, or controller rules changed.

The same 90-trial benchmark completed with no errors and unchanged repository,
binary and fixture during the run:

| Metric | Ordinary | Optimized deep | Original deep |
|---|---:|---:|---:|
| First-result hits | 35/45 | 34/45 | 34/45 |
| Top-five hits | 42/45 | 41/45 | 42/45 |
| Median seconds | 1.188 | 1.817 | 19.232 |
| Total Jev calls | 45 | 127 | 118 |

Deep median latency improved approximately 10.6 times. A separate local-only
probe of the optional-values question on 19,725 chunks measured time to the
first provider callback dropping from 17.502 s to 0.786 s, including snapshot
loading. These are individual probe measurements, not repeated-run medians;
neither probe sent a provider request.

The new parity test compares full candidate objects, including scores and order,
against the unchanged single-pass scorer for every generated action. It covers
all intents, filtered and unfiltered corpora, shortlist limits, stemming, code
identifiers, UTF-16 ties, repeated terms, empty corpora, and no-match queries.
All 41 Rust tests, Clippy with warnings denied, formatting, and whitespace checks
passed. HTTP mock tests required localhost access outside the sandbox.

Live responses varied: the Intel mitigation case had one additional top-five
miss after further investigation, and one optional-values run reached the step
limit. The deterministic parity checks demonstrate equivalent local candidates
for their tested inputs; the live run does not establish unchanged accuracy on
every query. Accuracy work remains deferred.

Follow-up report:
`benchmarks/results/deep-comparison-2026-09-18T14-27-27.307Z/report.json`.

Optimized executable SHA-256:
`927dbc6370a03b6a77e62d3e29221f2358527ff9b0244c3a56f3c4062605ea5b`.
