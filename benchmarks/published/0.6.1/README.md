# Published benchmark results for Oko 0.6.1

Raw per-task results behind the numbers in the README and
[docs/benchmark-results.md](../../../docs/benchmark-results.md). Each `.jsonl`
row is one task: its id, the regions or files Oko returned, and the scores the
benchmark's own code assigned. Nothing here is a benchmark's data; the data is
downloaded by the harnesses in `scripts/benchmark-public/replay/`.

Every run here used commit `5f3afad` with `jev-1.13.0`. The release commit
after it changes only version numbers and docs.

## agent-retrieval-bench/

- `oko-run1.jsonl`, `oko-run2.jsonl`, `oko-run3.jsonl`: three full runs on all
  345 positive tasks. The docs report their mean. Each row keeps every judged
  candidate with its relevance.
- `bcy.md`: the benchmark's `report-bcy-curve` output for the three runs, next
  to the 0.6.0 runs and the locally run RepoMap, lexical and BM25 baselines.
- `evaluator-commit.txt`: the evaluator commit that scored everything.

The baselines' own rows are unchanged since 0.5.0 and are in
[`../0.5.0/agent-retrieval-bench/`](../0.5.0/agent-retrieval-bench/).

## swe-explore/

- `oko-jev-by-score.jsonl`: all 848 issues, five regions by relevance. Every
  issue answered.
- `evaluator-commit.txt`: the scorer commit.

The BM25 and TF-IDF baselines are in [`../0.5.0/swe-explore/`](../0.5.0/swe-explore/).

## agent-sessions/

- `report.json`, `report.md`: the 243-session agent benchmark behind the
  README's agent table (nine tasks, three clients, without Oko / Oko / Oko with
  guidance, three repeats) on `5f3afad`, every session with its timing, token
  breakdown, tool calls, and grade. The home directory in local paths is
  written as `~`. Raw client logs and source snapshots are not included.

Sense's benchmark was not rerun for 0.6.1; its paired run is in
[`../0.6.0/sense-bench/`](../0.6.0/sense-bench/).
