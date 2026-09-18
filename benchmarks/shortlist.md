# Local shortlist audit — 2026-09-18

The new shortlist retains all 15 existing fixture targets inside the actual
Jev request budget. It also recovers the export entry point for the short export
question. The two longer export questions are still unresolved.

## Method

This is an offline candidate audit, not a model accuracy or agent speed benchmark.
A temporary Rust harness compared the pre-change search module with the new
release library on the same 19,725-chunk Telemetry Studio snapshot. All 15
expected source-file hashes in `telemetry-studio-expanded.json` were verified.
The target must overlap the fixture's evidence lines. Export probes require the
definition of `run_export_job` at `job.rs:1947`, not merely another chunk in that file.

The harness called `rank_lexically`, converted the results to `RankItem`, then
called `prepare_request_with_intent` with implementation intent and the unchanged
32,000-byte budget. No provider requests were made.

One pass across 18 queries measured a median of 738 ms before and 872 ms after
for corpus preparation plus local ranking, excluding filesystem reads and Jev.
These are illustrative timings; build/test work overlapped part of this audit.
This does not establish an end-to-end speed improvement.

## Results

| Question | Old target rank | New target rank | Sent to Jev before / after |
|---|---:|---:|---|
| strava-token | 2 | 2 | true / true |
| project-save | 1 | 1 | true / true |
| racebox-import | 1 | 1 | true / true |
| update-check | 1 | 1 | true / true |
| gapless-eligibility | 1 | 1 | true / true |
| optional-values | 2 | 3 | true / true |
| road-slope | 1 | 1 | true / true |
| gpx-repair | 1 | 1 | true / true |
| export-size | 1 | 1 | true / true |
| disk-space | 1 | 1 | true / true |
| mach-speed | 2 | 2 | true / true |
| unicode-path | 2 | 3 | true / true |
| intel-mitigation | 1 | 1 | true / true |
| parser-dispatch | 1 | 1 | true / true |
| centisecond-time | 1 | 1 | true / true |
| export-0 | absent | absent | false / false |
| export-1 | absent | 20 | false / false |
| export-2 | absent | 12 | false / true |

Export probe wording:

- **export-0:** Where is video export implemented, from the app export action through ExportService to frame rendering and video encoding?
- **export-1:** Where is video export implemented, from UI export start handler through export service frame rendering and FFmpeg encoding?
- **export-2:** Where is video export implemented?

The first long probe still misses the definition. The second finds it at rank 20,
but only nine full chunks fit in the request. Compact previews remain a separate
follow-up; this change does not modify request packing or chunk text.

## Implementation and portable coverage

The shortlist alternates ordinary content/path BM25 with symbol-aware BM25.
The symbol contribution uses the best matching declaration, with a bounded
cross-file identifier occurrence weight. It does not resolve language scopes,
imports, overloads, or calls. A soft per-file penalty encourages diversity.
The first ordinary content/path hit is preserved to protect against noisy hints.

Declaration extraction is heuristic. Unknown syntax and document formats retain
content/path search. Tests cover ten language examples, long definitions hidden
behind prose matches, file diversity and single-file backfill, deterministic
ordering, reference deduplication/bounds, and non-code fallback. These tests use
synthetic Rust fixtures and require neither Telemetry Studio nor an API key.

Validation: 70 tests passed, one real-keychain test ignored; formatting, Clippy
and the release build passed. Normal MCP/CLI search retains one ranking request.
The installed MCP executable has not been replaced by this change.
