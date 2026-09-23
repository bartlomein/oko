# Published benchmark results for Oko 0.5.1

Raw results behind the agent table in the README and
[docs/benchmark-results.md](../../../docs/benchmark-results.md#agent-sessions).
The retrieval benchmarks were not rerun for 0.5.1, which changes no ranking or
search code; their results are in [0.5.0/](../0.5.0/).

## agent-sessions/

- `report.json`, `report.md`: the 243-session agent benchmark behind the README's
  agent table (nine tasks, three clients, without Oko / Oko / Oko with guidance,
  three repeats) on the 0.5.1 build, every session with its timing, token
  breakdown, tool calls, and grade. Raw client logs and source snapshots are not
  included.
