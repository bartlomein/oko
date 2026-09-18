# Bounded rank fusion audit — 2026-09-18

The final release build retained all 29 checked source targets in its 30-item
shortlists, compared with 26 before the change. All 29 targets also survived
request packing. This is an offline candidate-recall check, not a Jev answer
accuracy or end-to-end agent-speed benchmark.

## Implementation under test

The local selector merges the top 100 ordinary body/path BM25 candidates with
the top 100 body/path-plus-symbol candidates using reciprocal rank fusion
(`k = 60`). It suppresses substantially overlapping source ranges and returns
at most 30 candidates. Separate useful functions in the same file are no longer
penalized just because their file was already selected. The reusable
`lexical_score` remains the original body/path-plus-symbol score.

Preview generation preserves a recognized declaration before other text. The
provider item limit and 32,000-byte application request budget are unchanged.
No file-expansion lane, additional provider call, new model, or user setting was
added in this change.

## Method

A temporary Rust harness compared the actual pre-change and final release
libraries against the same frozen source-chunk snapshots. The 15 existing
Telemetry Studio fixture hashes were verified. Three export questions and the
two observed power questions complete the original 20-case audit. Nine further
source-grounded questions were selected before inspecting new-ranker results:
three in Oko, three in a TypeScript monorepo, and three in a Python backend.
All 29 target source-file hashes were verified again after the final replay.

A target must overlap its expected evidence lines in the correct file; another
chunk from the same file does not count. Export probes require the actual
`run_export_job` declaration, and power probes require `draw_power`.

The harness also ran the real preview and request-packing functions. It checked
retained candidate IDs, target symbol names, and source evidence in the emitted
preview. The fixture checks require a nontrivial source line from the expected
evidence range; the added cases use an explicitly selected source fragment.
This text-presence check does not establish that a preview is sufficient to
answer the entire question.

No provider requests were made. External source, raw previews, absolute local
paths, and temporary corpus snapshots are not included in the repository. These
local checks are separate from the portable regression tests used by CI.

## Candidate results

| Corpus | Chunks | Cases | Before: target in final 30 | After: target in final 30 |
|---|---:|---:|---:|---:|
| Telemetry Studio, whole repository | 19,725 | 19 | 17 | 19 |
| Telemetry Studio, compositor subtree | 3,672 | 1 | 0 | 1 |
| Oko, Rust | 476 | 3 | 3 | 3 |
| TypeScript monorepo | 58,893 | 3 | 3 | 3 |
| Python backend | 1,675 | 3 | 3 | 3 |
| **Total** | — | **29** | **26** | **29** |

All 15 existing fixture targets stayed inside the shortlist. The difficult
probes changed as follows; positions are local candidate ranks, not model scores.

| Probe | Before | After |
|---|---:|---:|
| Long export question, including trigger, pipeline, rendering and FFmpeg | 22 | 13 |
| Long export question, from starting export through orchestration and output | absent | 10 |
| Short export question | 12 | 5 |
| Broad power question | absent | 9 |
| Scoped power question | absent | 8 |

Exact probe wording:

- “Where is video export implemented, including the app export trigger, ExportService pipeline, frame rendering and FFmpeg encoding?”
- “Where is video export implemented, from starting export in the app through export service orchestration and video frame encoding/output?”
- “Where is video export implemented?”
- “where power overlays are implemented”
- “where the Power wattage overlay is drawn” (compositor subtree)

The additional cases cover API-key validation, setup rollback, ranking
abstention, webhook signature normalization, OAuth digest comparison, an issue
deduplication cache, Eastern-time weekend detection, audio duration estimation,
and topic-based article retrieval. All nine targets were already retrievable
before this change and remained retrievable afterward; they are regression
smoke checks rather than evidence of a new cross-language accuracy gain.

## Preview and request results

Every request retained all 30 candidates. Serialized application requests ranged
from 25,722 to 31,999 bytes, before the HTTP transport adds its model field.
All 29 expected target candidates and all 29 target symbol names were retained.

The explicit source-evidence checks passed for 28 of 29 cases. All 20 Telemetry
cases passed, including the previously omitted long-export function header.
The remaining Oko setup-rollback preview omitted the specifically checked
reverse-iteration loop line. It included the `apply` declaration and nearby
restoration/error context, but this does not satisfy that exact evidence check.
The full candidate contains the loop. Thus complete candidate recall must not
be reported as complete preview sufficiency or final-answer accuracy.

## Bounded-pool and hierarchy experiments

Before implementation, disposable experiments compared candidate windows of
50, 100, and 200 with RRF constants of 20 and 60 on the original 20 questions.
The chosen pair of ranking lists retained all 20 targets at window 100 for both
constants. Window 50 missed the broad power target; window 200 recovered no
additional targets beyond 100. These results suggest a reasonable initial
configuration, not a universally optimal one.

Replacing the second list with symbol-only ranking was weaker: window 100 and
`k = 60` retained 18 of 20 targets and missed both long export definitions.
The implemented second list therefore retains body/path context as well as the
symbol contribution.

A separate prototype reserved up to five slots for one additional declaration
from each of five leading files, ranked within the file by ordinary body/path
relevance. It recovered broad power with the smaller 50-candidate windows, but
added no target recall at the chosen 100-candidate configuration. In a weaker
fusion configuration it displaced a target previously at rank 30. File
expansion was therefore not included in this version.

## Local component timings

The timing harness used temporary copies of the before/after search modules on
the same frozen snapshots. Corpus preparation was measured separately once per
corpus. Each query's selection time is the median of three calls on an already
prepared corpus; the table reports the median across that corpus's queries.
The final timing pass ran after the build/test checks completed.

| Corpus | Preparation before / after | Selection before / after |
|---|---:|---:|
| Telemetry Studio | 844 / 881 ms | 11.00 / 9.81 ms |
| Compositor subtree | 157 / 162 ms | 1.08 / 0.86 ms |
| Oko | 19 / 19 ms | 0.19 / 0.19 ms |
| TypeScript monorepo | 3,936 / 4,187 ms | 45.75 / 43.47 ms |
| Python backend | 217 / 221 ms | 0.61 / 0.74 ms |

Preparation still dominates local work. This change does not optimize that
phase. These small, sequential measurements include normal machine and cache
variation; they do not demonstrate a significant speed improvement. They
exclude filesystem scanning, preview preparation, network/provider time,
context-packet construction, and the coding agent's reasoning and follow-ups.

## Validation and limits

The final implementation passed 112 tests, with one real OS-keychain test
ignored, plus formatting, Clippy with warnings denied, and the release build.
Portable tests cover fusion, source-range overlap, same-file definitions,
filtering, stable scores/order, and declaration-preserving previews.

The observed gain is three recovered candidates on a small development sample.
The nine new questions broaden the language and repository coverage but do not
replace a larger independently chosen benchmark. No new Jev ranking accuracy,
final-answer quality, or Codex wall-clock improvement was measured.
