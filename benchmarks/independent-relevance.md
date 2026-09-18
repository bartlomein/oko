# Independent relevance ranking — 2026-09-18

Oko now asks a separate Noul relevance question for each candidate, with all
questions sharing one Jev request. Previously, one Choice question selected a
winner and its mutually competing option probabilities were used as ranking
scores. Several useful implementations can now receive high scores together.

This changes the shared ranker used by `rank`, normal code search, and the ranking
steps of deep investigation. Deep mode's action-selection Choice question is
unchanged. Candidate discovery and source-excerpt extraction are unchanged.

## Contract

- At most 30 independent questions in one HTTP request per ranking step.
- Every question explicitly names its indexed candidate and the query in state.
- Typed `answers.candidate_N.noul` scores must be finite numbers in `[0, 1]`.
  Missing or invalid answers fail the request, including candidates outside the
  final result limit. The old Choice response is not accepted as a fallback.
- Scores above `0.5` are retained, sorted descending with stable ties. This is
  a provisional decision boundary, not a calibrated accuracy guarantee.
- Original IDs and sources, the 32,000-byte application request cap, ten-second
  provider timeout, and no-retry behavior remain. Transport adds the model field
  after the application budget check.

[TypeSafe's Noul documentation](https://docs.typesafe.ai/primitives/noul)
defines the yes/no score. Its [primitives documentation](https://docs.typesafe.ai/primitives)
supports independent questions sharing state in one request. The official
[reranking cookbook](https://docs.typesafe.ai/cookbooks/rerank_typesafe) evaluates
query-candidate pairs in separate requests; batching those judgments is Oko's
adaptation, not a reproduction of that cookbook's benchmark.

## Controlled live comparison

Four previously approved frozen Telemetry Studio preview sets were used:
speed, power, temperature, and elevation. Each query was phrased as
"where [topic] overlays are implemented". Both methods received identical
candidate IDs, order, text, and sources, fitted once to the new question budget.
Every request retained all 30 candidates. This isolates ranking behavior on
those shared previews; it is not a comparison against the larger previews the
old request format could fit.

Two repetitions per method produced exactly 16 successful provider requests,
with alternating method order and reversed topic order in the second repetition.
Both requested and returned model IDs were `jev-1.13.0`. Raw responses, model
metadata, usage, and requests were saved locally outside the repository.

| Check | Choice | Independent Noul |
|---|---|---|
| Specific power-wattage implementation | Excluded in both runs | Rank 3 in both runs |
| Elevation first result | Test in both runs | Drawing implementation in both runs |
| Temperature drawing implementations | Canvas first, headless second | Both in the first two positions |
| Median client/provider round trip | 472 ms | 488 ms |

The missing power candidate scores `0.01` and `0.0` with Choice, below its `none`
probability of `0.05`. It now scores `0.85` and `0.86`, independently of other
power implementations. In the second run it ties two other candidates and
remains third after the existing stable tie handling. Normal MCP packets select
three primary results; packet delivery is checked separately below.

Offline packet replay uses the actual shortlist mapping, code-search tie
handling, source expansion, RMCP structured serialization, and 16,000-byte fitting
loop. With conservative timing metadata, both Noul power responses include the
complete `draw_power` body at lines 961–1011, including watts formatting, marked
`truncated: false`. Each complete MCP envelope is 11,259 bytes. Choice omits it
in both runs. Both final temperature packets contain the two renderers and
rendering dispatch, without the unrelated test; envelopes are 15,642 and 15,640
bytes. This is a packet replay, not a fresh Codex agent session.

Noul keeps an average-speed helper first; Choice puts a planning document first
in one run. The specifically checked
speedometer and elevation-graph declaration ranges were absent from the input
shortlists, so neither scoring method could recover them. Noul also gives a
temperature project test a high score: fourth in both runs. Although it falls
outside the three primary MCP results in this sample, the implementation intent
does not reliably eliminate every test from ranked results.

Median reported input tokens increased from 9,690 to 10,199 on the identical
preview sets. Output tokens were approximately 317–318 for Choice and 565 for
Noul. Similar observed latency does not mean identical token cost. Eight timings
per method do not establish a speed improvement, and these measurements exclude
local search, preview generation, packet construction, and the coding agent.

## Budget and portable validation

Concise instructions use 5,952 question bytes for 30 implementation candidates,
versus 2,493 for the previous Choice question. Normal search automatically
shortens previews to fit the same total request cap. Generic `rank` and deep
mode retain their existing tail-dropping behavior for oversized full items.

The offline replay covers five frozen corpora and 29 existing source targets,
plus the four broad overlay queries. All 33 requests retained all 30 candidates,
with a maximum of 31,992 application bytes. All 29 original target candidates
and names remain. Exact preview-evidence coverage stays at 28/29, with no new
regressions. The setup-rollback evidence miss predates this change.
For the initial deep power ranking, 13 full chunks fit instead of 14; the power
implementation at candidate 9 is still included.

An earlier 7,182-byte prompt draft lost a parser-dispatch statement from its
preview. Removing redundant question wording restored that evidence without
changing preview selection. That draft also recovered the power implementation
at ranks three and five, delivering it in only one of two MCP packet replays.
It was superseded by the final build and all live checks were repeated. The
draft and final validations together used 50 provider requests: two sets of
16 controlled requests and two sets of nine synthetic checks. Earlier draft
results remain separate from the final figures above.

All nine existing synthetic live smoke checks passed: five generic support-ticket
queries, including no match, and four implementation/explanation preference
checks across Rust and TypeScript snippets. These use synthetic fixtures and
are not a held-out accuracy benchmark.

131 Rust tests pass, with one real OS-keychain test ignored. Tests cover typed
response validation, multiple independently high scores, abstention, reserved
user IDs, stable ties, exact UTF-8 budgets, all 30 questions in a single normal
MCP request, and unchanged deep action selection. Formatting, Clippy with
warnings denied, and the release build pass. No Codex end-to-end timing claim
is made, and no installed MCP binary was replaced by this validation.
