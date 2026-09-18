# Source evidence repair — 2026-09-18

This change repairs returned function excerpts and improves the source evidence
sent in the existing normal-search Jev request. It does not change the shortlist,
provider, ranking instructions, or number of provider calls.

## Changes

Declaration extent detection now allows bounded multiline signatures instead of
stopping after twelve lines. Incomplete or ambiguous extents use bounded source
context marked `truncated`, rather than returning a header as if it were a
complete implementation. Detection remains lexical, with a 128-line signature
limit; it is not a full language parser.

Ranking previews prioritize declaration names, attached attributes/decorators,
and useful implementation statements. Prose documentation follows query/body
evidence so a long comment cannot consume the entire preview. The normal CLI
and MCP paths can recover adjacent decorators from the already loaded corpus.
Missing, conflicting, and malformed context is not used. Candidate IDs still
map to the original source chunks, and the 32,000-byte request budget is unchanged.

## Returned snippet checks

An offline replay used the frozen source snapshot from the rank-fusion audit
and fixed source candidates corresponding to three previously observed
header-only results. Each now returns 60 source lines including body code:

| Declaration | Earlier observed excerpt | Replayed excerpt |
|---|---|---|
| `draw_average_speed` | line 52 only | lines 52–111 |
| `draw_power_graph` | line 21 only | lines 21–80 |
| `draw_elevation_graph` | line 31 only | lines 31–90 |

All three replayed excerpts are marked `truncated: true`, because more source
remains. These are fixed-winner packet checks, not evidence that Jev selects the
same winners on a new request.

## Frozen preview replay

The final release library was replayed against the same five corpus snapshots
and 29 source targets used by the rank-fusion audit. All 29 target candidates
and names remained in the requests. Exact preview-evidence checks stayed at
28/29; the previously documented setup-rollback evidence miss remains. Four
additional broad overlay questions were also checked. Every one of the 33
requests retained all 30 candidates, with a maximum of 31,998 application
request bytes. Target candidate IDs were unchanged in every comparison.

The first draft regressed one parser-dispatch preview by prioritizing too much
documentation. Moving prose behind body/query evidence restored the prior
result, and a portable regression test now covers that failure. This audit
establishes no loss on this small sample, not improved live ranking accuracy.

## Portable checks and limits

125 tests passed, with one real OS-keychain test ignored. Formatting, Clippy
with warnings denied, and the release build were checked separately. New tests
cover long Rust/Go/TypeScript signatures, Python multiline signatures, incomplete
boundaries, prototypes, annotation preservation, conflicting source context,
and crowded requests with long documentation. A real stdio MCP test uses a
local fake provider to verify annotation delivery, full body extraction, one
normal provider call, and both request and response budgets.

The frozen elevation test candidate already contained `#[test]` in its old
preview. Preserving annotations more reliably does not by itself explain or fix
that observed Jev ranking error. No end-to-end Codex timing measurement was
performed for this change. Deep mode keeps its
existing ranking inputs; returned-packet extraction improvements apply to both
normal and deep results. Raw private source snapshots and previews remain
outside the repository.

## Approved live comparison

After explicit approval, four frozen preview sets were sent to the configured
TypeSafe/Jev endpoint: speed, power, temperature, and elevation, each phrased
as "where [topic] overlays are implemented". The same binary, intent, candidate
IDs/order, and configuration were used for both preview versions. Two repetitions
per version produced exactly 16 successful requests, without retries. The first
repetition ran baseline before new previews; the second reversed that order.
Every request retained all 30 candidates.

| Question | First result, both versions and both repetitions |
|---|---|
| Speed | Section-average-speed overlay helper |
| Power | Power-graph overlay helper |
| Temperature | Current-temperature drawing implementation |
| Elevation | `elevation_gain_contract` test |

No top-result improvement was observed. The power wattage implementation was
in the input shortlist but absent from the returned results in all four power
runs. The specifically checked speedometer and elevation-graph declaration
ranges were absent from their input shortlists in both versions; changes to
preview wording cannot recover missing candidates. Other relevant rendering
code was present, so this does not imply no useful result existed.

Median elapsed CLI request time was 0.552 seconds before and 0.506 seconds
after. This includes process startup, key loading, HTTP/provider time, and
output serialization; it excludes repository search, preview generation, MCP
packet building, and the coding agent. With only eight requests per version,
the difference does not establish a speed improvement.

These results support the snippet repair and evidence-preservation regression
tests separately, but do not establish improved live ranking accuracy or reduced
Codex follow-up work. In particular, the elevation test remains first despite
its test annotation being visible in both versions.
