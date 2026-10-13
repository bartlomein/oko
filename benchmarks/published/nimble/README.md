# Published benchmark results: Nimble against Jev

Raw results behind [Nimble against Jev](../../../docs/benchmark-results.md#nimble-against-jev-october-2026):
Oko 0.7.2 with the Nimble provider, ranking with Jev (`jev-1.13.0`) or with
Nimble (`nimble:latest`, Ollama 0.35.1, one RTX 5090), measured on 12 October
2026. Nothing here is a benchmark's data, and the home directory in local paths
is written as `~`.

## agent-retrieval-bench/

- `jev.jsonl`, `nimble.jsonl`: one run each on all 345 positive tasks, the same
  build, scored with the evaluator at `evaluator-commit.txt`. Each row keeps
  every judged candidate with its relevance.
- `bcy.md`: the benchmark's BCY curve for both runs and the three published
  0.7.0 runs, from each row's ranked files.

## agent-sessions/

- `report-codex-claude-pi.json`: 243 sessions of the rankers suite
  (`runner.py --suite rankers`) at commit `7dc779b`: Claude Code 2.1.289 with
  Claude Sonnet 5, Codex CLI 0.162.1 and Pi 1.2.0 with GPT-5.6 Sol; nine tasks,
  three repetitions, without Oko, with Oko and Jev, with Oko and Nimble.
- `report-opencode-sonnet.json`, `report-pi-sonnet.json`: 81 sessions each, the
  same suite and build, OpenCode 1.18.31 and Pi 1.2.0 with Claude Sonnet 5.
- `summary.md`: the 405 sessions by setup and condition, paired changes,
  prompt-hook outcomes and every session that did not pass.
- `report-opencode-gpt-partial.json`: the first 20 of 81 sessions of OpenCode
  1.18.31 with GPT-5.6 Sol on the same build, stopped when the ChatGPT plan's
  usage ran low.
- `report-before-prefetch-fix-partial.json`: 73 sessions of an earlier run,
  stopped, on the build before Nimble's prompt hook judged only the first 20
  candidates (commit `ec759c7`): every Nimble prefetch timed out.

Every session has its timing, token breakdown, tool calls, prompt-hook
decision and grade. Raw client logs and source snapshots are not included.
