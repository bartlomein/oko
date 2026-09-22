# Published benchmark results for Oko 0.5.0

Raw per-task results behind the numbers in the README and
[docs/benchmark-results.md](../../../docs/benchmark-results.md). Each `.jsonl`
row is one task: its id, the regions or files Oko returned, and the scores the
benchmark's own code assigned. Nothing here is a benchmark's data; the data is
downloaded by the harnesses in `scripts/benchmark-public/replay/`.

## agent-retrieval-bench/

- `oko-run1.jsonl`, `oko-run2.jsonl`, `oko-run3.jsonl`: three full runs of the
  0.5.0 build on all 345 positive tasks with `jev-1.13.0`. The README reports
  their mean. Each row keeps every judged candidate with its relevance.
- `oko-first-look-experiment-build.jsonl`: the one-time run of the frozen
  experiment build before the last two changes (see the docs page).
- `baseline-repomap.jsonl`, `baseline-lexical.jsonl`, `baseline-bm25.jsonl`:
  the benchmark's own baselines, run locally with its CLI on the same tasks.
- `bcy-run{1,2,3}.md`: the benchmark's `report-bcy-curve` output for each run.
- `evaluator-commit.txt`: the evaluator commit that scored everything.

## swe-explore/

- `oko-jev-by-score.jsonl`: the 0.5.0 build, five regions by relevance (the
  reported run). `oko-jev-one-per-file.jsonl`: the same, one region per file
  first (an ablation; its rows also keep the top 60 ranked regions).
- `oko-keywords.jsonl`: keyword ranking only, no Jev.
- `baseline-bm25.jsonl`, `baseline-tfidf.jsonl`: the benchmark's own explorers.
- `evaluator-commit.txt`: the scorer commit.

## agent-sessions/

- `report.json`, `report.md`: the 243-session agent benchmark behind the README's
  agent table (nine tasks, three clients, without Oko / Oko / Oko with guidance,
  three repeats), every session with its timing, token breakdown, tool calls,
  and grade. Raw client logs and source snapshots are not included.
