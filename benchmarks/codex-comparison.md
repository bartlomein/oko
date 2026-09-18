# Oko vs Codex CLI — 2026-09-18

Five source-verified Telemetry Studio questions, three repetitions each.
Both tools completed all 15 trials without errors. The repository, Oko binary,
output schema, and question fixture were unchanged during the run.

| Tool | Median total time | Correct first | Correct top five |
|---|---:|---:|---:|
| Oko with Jev | 1.22 s | 12/15 | 15/15 |
| Codex CLI | 19.20 s | 15/15 | 15/15 |

Oko's median was 15.7x faster on this small test.
Codex placed the correct implementation first in every trial. Oko's three
first-result misses were the same Strava token-exchange question on each repeat;
the correct implementation was still in its top five. Function-range metrics
matched exact implementation-range metrics for both tools.

## Method

- Codex CLI 0.153.4, `gpt-6-astra`, medium reasoning. Fresh ephemeral sessions,
  read-only sandbox, user config ignored. Repository and applicable global
  instructions still applied; Codex read the installed communication skill.
- Codex received the question and response format, never the expected answers.
  Its command logs show local `rg` searches and source inspection. No web search,
  Oko calls, or benchmark-answer reads were observed.
- Oko used the release executable from the Rust rewrite. Both tools returned up
  to five repository-relative locations, with ranges limited to 120 lines.
- Correctness means overlapping the verified implementation range in the correct
  file. Function-range overlap is recorded separately. Only file matches do not
  count. Both use the same scorer and source hashes.
- Sequential trials alternate tool order. Time runs from process start through
  exit, including initialization, instructions, model/tool execution, and network
  requests. No warmup; OS caches were not cleared and source snapshotting reads
  files before timing. Provider prompt caching was not disabled; Codex usage,
  including cached tokens, is recorded.

These are complete code-location workflows, not a benchmark of ripgrep versus
Oko's lexical engine. These five development questions were already used to tune
Oko and are not held out. Fifteen trials repeat five questions; they are not
fifteen independent test cases. This result does not establish general accuracy,
Linux-server performance, token-cost savings, or performance on coding tasks.

## Reproduce on this machine

```sh
cargo build --release --locked --bin oko
node scripts/compare-codex.mjs /Users/bart/dev/telemetry-studio \
  --repeats 3 --model gpt-6-astra --effort medium \
  --codex-bin /Applications/ChatGPT.app/Contents/Resources/codex
```

Other machines can omit `--codex-bin` to use `codex` on PATH. It must support the
selected model. The initial attempt with installed CLI 0.143.0 was rejected by
the provider before searching and was aborted; its partial report is excluded.
No global CLI installation or configuration was changed.

Reports are generated and ignored by Git:

- `benchmarks/results/codex-2026-09-18T11-46-41.377Z/report.md`
- `benchmarks/results/codex-2026-09-18T11-46-41.377Z/report.json`

The JSON contains version/settings, source and artifact hashes, all returned
locations, scores, timings, Codex usage, and shell commands. Raw tool output and
provider stderr are not stored. No secrets are written by the runner.

Validation: `node --test scripts/compare-codex.test.mjs` passes five tests covering
range grading, invalid outputs, failed-run accounting, event parsing, and timeout
behavior. No production Rust code changed for this benchmark.
