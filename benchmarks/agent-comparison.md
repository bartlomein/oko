# Oko, Codex CLI, and OpenCode comparison

## Results — 2026-09-18

All 135 trials completed, with zero failed trials. The target repository snapshot,
Oko executable, question fixture and output schema were unchanged.

| Tool | Expected location first | In top five | Median total time |
|---|---:|---:|---:|
| oko | 35/45 | 42/45 | 1.28 s |
| codex | 42/45 | 45/45 | 33.70 s |
| opencode | 42/45 | 45/45 | 30.61 s |

Oko was about 24–26 times faster by median on this configuration and question set.
Its three top-five misses were the same optional-value interpolation question in
every repeat. Codex and OpenCode retrieved the expected location on every trial.
First-place misses include useful alternative functions, as detailed below; the
table measures agreement with fixed location labels rather than all valid answers.

The ten new questions, repeated three times, scored as follows:

| Tool | Expected location first | In top five | Median total time |
|---|---:|---:|---:|
| oko | 23/30 | 27/30 | 1.28 s |
| codex | 30/30 | 30/30 | 37.55 s |
| opencode | 29/30 | 30/30 | 32.61 s |

Oko first-place counts varied between 10/15, 14/15 and 11/15 across repeats.
Its first-result misses included file headers, a test, and a useful Mach-formatting
caller. Codex put the RaceBox row parser before the expected file parser three
times. OpenCode did so twice, and put the Mach formatter before its numerical
helper once. These alternatives were inspected but did not change frozen scores.

OpenCode recovered from three failed tool calls; no trial failed. Its recorded
tool types were read, glob and grep. Codex recorded shell commands and final
messages; no commands referencing benchmark answer files or saved memories were
observed. The runner does not retain raw tool output.

Full per-trial reports (ignored by Git):

- `benchmarks/results/comparison-2026-09-18T12-42-17.437Z/report.md`
- `benchmarks/results/comparison-2026-09-18T12-42-17.437Z/report.json`

Validation: nine offline benchmark tests pass, including a synthetic complete
run of both adapters with shared model settings. No production Rust code was
changed for the OpenCode/expanded-fixture work.

## Scope

Fifteen source-verified implementation-location questions from Telemetry Studio:
five development questions previously used for Oko, and ten additional questions
frozen before this run. Three repetitions per question, 45 trials per tool.
The additional questions cover interpolation, slope calculation, GPX repair,
export disk handling, Mach conversion, Unicode display paths, hardware-specific
encoding, format dispatch, and time formatting.

This is a small, single-repository evaluation, mostly of localized functions.
Repeating a question does not create another independent test case. The ten
new questions were not used to adjust Oko during this comparison, but they
are not an independently curated benchmark.

## Configuration

Codex and OpenCode both use **GPT-5.5 with medium reasoning**. Codex CLI is
0.153.4; OpenCode is 1.18.21. Both use saved OpenAI authentication. Oko uses
its Rust release binary and Jev with the implementation ranking intent.

Codex runs fresh ephemeral sessions in its read-only sandbox with user config
ignored. OpenCode runs fresh sessions with external plugins disabled and a
custom read-only agent: native read/glob/grep allowed; shell, edits, web and
delegation denied. OpenCode permissions are not an OS sandbox. Local OpenCode
sessions are retained but not shared. Neither global tool configuration nor
the target repository is intentionally modified.

The agents receive the same question and output instructions, without expected
answers. Codex additionally enforces its output JSON schema; OpenCode's final
JSON is validated by the runner. Same model and effort do not equalize system
instructions, context, available tools, output enforcement, or provider caching.
This measures these specific tool configurations, not every possible setup.

## Scoring and timing

A hit overlaps the verified implementation's inclusive line range in the right
file. First-result and top-five hits are measured; overlap with the containing
function is recorded separately. “Exact” in the generated table means the
verified implementation range, not identical start/end boundaries. A plausible
alternative function can score as a miss if it is not in the fixture. In
particular, the RaceBox question expects the file parser; its row parser is
also useful and is discussed separately when returned first. The Mach question
expects the numerical conversion helper, but the speed-unit formatter also
validates input and calls that helper; that is another useful alternative.
Scores are not changed after seeing model output. These conservative location
labels should not be read as a complete judgment of semantic answer quality.

Timing includes fresh process startup, model calls, local search/read tools,
and exit. Runs are sequential; tool order rotates through all positions across
three repeats. There is no warmup or cache clearing, and the source snapshot
reads files before timing. Errors count as misses and stop the run; successful
latency medians exclude errors. Each invocation has a 180-second timeout with
no harness-level automatic retry. Agent tools and provider libraries may
retry or recover internally; recovered tool errors are not failed trials. Token counters are recorded in each CLI's own format;
no cost comparison is inferred.

Source file hashes are checked before requests. Before/after snapshots cover
Git state and ripgrep-visible files up to 256 KiB, matching Oko's source-size
limit. This does not prove every ignored or larger file remained unchanged.
The Oko binary, fixture and output schema are also checked before/after.
Reports record returned locations and tool metadata, without raw source output
or provider stderr. Secrets are not stored in benchmark reports.

## Reproduce

```sh
cargo build --release --locked --bin oko
node scripts/compare-agents.mjs /path/to/telemetry-studio \
  --repeats 3 --model gpt-5.5 --effort medium
```

Both CLIs must be installed and authenticated. Use `--codex-bin PATH` and
`--opencode-bin PATH` to choose executables. Oko's Jev key is loaded from its
`.env` or the shell. `--tools codex,opencode` omits Oko. `--fixture PATH` accepts
another repository's source-verified question fixture. Reports are ignored
under `benchmarks/results/comparison-*/`.

Offline validation, with no API requests or Telemetry Studio checkout:

```sh
node --test scripts/compare-codex.test.mjs
```

The tests cover grading, invalid output, errors, timeouts, both event formats,
rotating tool order, fixture structure, and a complete synthetic two-agent run
that verifies both model arguments and prevents forwarding the Jev key.
