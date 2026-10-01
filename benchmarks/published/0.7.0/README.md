# Published benchmark results for Oko 0.7.0

Raw per-task results behind the numbers in the README and
[docs/benchmark-results.md](../../../docs/benchmark-results.md). Each `.jsonl`
row is one task: its id, the regions or files Oko returned, and the scores the
benchmark's own code assigned. Nothing here is a benchmark's data; the data is
downloaded by the harnesses in `scripts/benchmark-public/replay/`. The home
directory in local paths is written as `~`.

## agent-sessions/

- `report-claude-codex.json`, `report-claude-codex.md`: 216 sessions of the
  agent benchmark with Claude Code 2.1.285 and Codex CLI 0.155.0 on commit
  `498dd4e` (nine tasks, four setups: without Oko, Oko, Oko with guidance, Oko
  with guidance and the prompt hook; three repeats).
- `report-opencode.json`, `report-opencode.md`: the same 108 sessions for
  OpenCode 1.18.31 on commit `ecb006a`, which adds OpenCode's prompt hook.

Every session has its timing, token breakdown, tool calls and grade. Later
commits change only setup, metrics, docs and benchmark scripts. Raw client logs
and source snapshots are not included.

## sense-bench/

- `report-oko-0.7.0.md`/`.json`, `report-oko-0.6.1.md`/`.json`,
  `report-sense.md`/`.json`: Sense's harness report for each of the three
  setups in the 90-session run of 30 September (Oko with the prompt hook at
  `ecb006a`, the Oko 0.6.1 release binary, Sense's own image), scored and
  judged by the harness.
- `costs.md`: model tokens (uncached input, cache writes, cache reads, output)
  and cost per setup, from the session transcripts. Jev usage was not recorded
  in this run; later runs record it.
- `harness-commit.txt`: the Sense commit whose harness ran.

Transcripts are not included.

## agent-retrieval-bench/

- `oko-run1.jsonl`, `oko-run2.jsonl`, `oko-run3.jsonl`: three full runs on all
  345 positive tasks, commit `da83b98` with `jev-1.13.0`. The docs report their
  mean. Each row keeps every judged candidate with its relevance.
- `bcy.md`: the benchmark's `report-bcy-curve` output for the three runs, next
  to the 0.6.1 runs and the locally run RepoMap, lexical and BM25 baselines.
- `evaluator-commit.txt`: the evaluator commit that scored everything.

The baselines' own rows are unchanged since 0.5.0 and are in
[`../0.5.0/agent-retrieval-bench/`](../0.5.0/agent-retrieval-bench/).

## swe-explore/

- `oko-jev-by-score.jsonl`: all 848 issues, five regions by relevance, commit
  `da83b98` with `jev-1.13.0`. Every issue answered.
- `evaluator-commit.txt`: the scorer commit.

The BM25 and TF-IDF baselines are in [`../0.5.0/swe-explore/`](../0.5.0/swe-explore/).
