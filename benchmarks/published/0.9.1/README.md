# Published benchmark results for Oko 0.9.1

Agent Retrieval Bench only: three full runs on all 345 positive tasks with
`jev-1.14.0`, measured on 22 October 2026 with the 0.9.1 release build
(`7209f00`) and evaluator commit `07014c98`. Other benchmarks were not rerun;
their latest results stay under earlier versions.

## agent-retrieval-bench/

- `oko-run1.jsonl`, `oko-run2.jsonl`, `oko-run3.jsonl`: one row per task, in
  the same format as [0.5.0](../0.5.0/agent-retrieval-bench/), keeping every
  judged candidate with its relevance.
- `oko-run{1,2,3}.summary.json`: per-task means for each run.
- `jev-calls-run{1,2,3}.jsonl`: every Jev request: phase, status, latency, and
  input and output tokens. About 1,090 requests and 14.7M input tokens per run
  ($0.62 at $0.042 per million input tokens); 9 of 3,266 requests across the
  three runs failed in transport, and no task errored.
- `bcy.md`: the benchmark's BCY report for these three runs and the three 0.5.0
  runs, computed offline from the stored rankings.
- `evaluator-commit.txt`: the evaluator commit that scored everything.
- `SHA256SUMS`: checksums of the files above.
