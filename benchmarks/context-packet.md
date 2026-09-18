# Context packet validation — 2026-09-18

Normal Oko search now ranks compact previews once, restores the winning source,
and assembles a bounded MCP context packet locally. This validation measures
candidate coverage, source provenance, protocol behavior, and local overhead.
It does **not** establish improved Jev accuracy or faster complete Codex answers.

## Offline candidate coverage

A temporary Rust release harness used one 19,725-chunk Telemetry Studio
snapshot, comparing full-chunk request preparation with compact previews on the
same shortlist. All 15 source hashes in the expanded fixture were verified.
No provider requests were made. All 30 candidates fit for every one of 18 queries;
all 15 fixture targets retained their function name and at least one expected
source evidence line in their preview.

| Fixture | Target rank | Full chunks sent | Previews sent |
|---|---:|---:|---:|
| strava-token | 2 | 20 | 30 |
| project-save | 1 | 26 | 30 |
| racebox-import | 1 | 15 | 30 |
| update-check | 1 | 18 | 30 |
| gapless-eligibility | 1 | 9 | 30 |
| optional-values | 3 | 26 | 30 |
| road-slope | 1 | 17 | 30 |
| gpx-repair | 1 | 27 | 30 |
| export-size | 1 | 17 | 30 |
| disk-space | 1 | 15 | 30 |
| mach-speed | 2 | 20 | 30 |
| unicode-path | 3 | 20 | 30 |
| intel-mitigation | 1 | 20 | 30 |
| parser-dispatch | 1 | 18 | 30 |
| centisecond-time | 1 | 30 | 30 |

Three export probes:

| Probe | Target rank | Full chunks sent | Previews sent | Entry-point header reaches Jev |
|---|---:|---:|---:|---|
| Latest expanded query A | 22 | 9 | 30 | Yes, previously omitted |
| Latest expanded query B | Absent | 8 | 30 | No, still absent from shortlist |
| Short question | 12 | 17 | 30 | Yes |

A: “Where is video export implemented, including the app export trigger,
ExportService pipeline, frame rendering and FFmpeg encoding?”

B: “Where is video export implemented, from starting export in the app through
export service orchestration and video frame encoding/output?”

Short: “Where is video export implemented?”

The target header is `run_export_job` at `job.rs:1947`. A separate context check
used the previous middle-body winner at `job.rs:3441–3560`: the packet now carries
that function name and its header location, with source excerpt lines 3437–3496.
This is a deterministic expansion check, not a new model selection result.

## Local cost

Preview preparation across the 18 queries: **7.89 ms median,
19.34 ms maximum** in a release build. Context expansion for three real
winning locations (export job, app entry point, service start) took **44–46 ms**
on the same corpus. Reducing the packet to 6 KB added approximately 0.2 ms.
These component timings exclude file scanning, shortlist scoring, Jev and Codex.
They are development measurements, not cross-platform latency guarantees.

## Protocol and source guarantees

- One normal Jev request; the context builder reads only the captured snapshot.
- Up to three primary excerpts and two related lexical definition candidates.
- Original inclusive source ranges; no ranking-preview markers returned as code.
- UTF-8-safe truncation, overlap deduplication, and explicit ambiguity flags.
- 16,000-byte serialized MCP result cap includes both structured content and
  its compatibility text copy, plus JSON escaping; excludes the JSON-RPC wrapper.

The Rust tests use synthetic temporary projects and mock HTTP/MCP servers.
They check late-candidate selection among 30 previews, exact source restoration,
related definitions, one-call behavior, escaping/long-question budgets,
read-only root boundaries, fresh files, and existing deep/CLI behavior.
Function metadata is a lexical hint, not a resolved import/type/call graph.

## Related-definition precision

The first real authentication run exposed a false association: the source called
`urlencoding::encode`, but related lookup returned two graphics methods named
`encode`. Both were labelled ambiguous, yet still occupied the two related
slots. A same-name definition is insufficient evidence for a qualified call.

Related lookup now preserves qualification, excludes unresolved receiver calls,
and omits ambiguous candidates. Simple Rust module paths and direct imports can
provide supporting evidence; unknown syntax does not trigger a basename-only
retry. Primary search results remain available when related lookup abstains.
The matching stays local and adds no provider request.

This deliberately stops short of compiler-level resolution. The
[Rust Reference](https://doc.rust-lang.org/reference/names/name-resolution.html)
describes names as scoped and namespace-dependent, with some resolution
requiring type information. Its [path rules](https://doc.rust-lang.org/reference/paths.html)
give qualifiers such as `crate`, `self`, and `super` distinct meanings.
[Tree-sitter's navigation documentation](https://github.com/tree-sitter/tree-sitter/blob/master/docs/src/4-code-navigation.md)
also distinguishes definition, method, and reference tags; syntax extraction
alone does not establish which receiver a method belongs to. These support
omitting uncertain extras in this bounded lexical implementation.

An offline release harness replayed the source excerpts captured in the latest
auth and export packets against the current 19,725-chunk workspace. Each case
ran three times against the old and new builders. It made no provider requests;
it did not rerun candidate ranking or simulate Codex.

| Replay | Related context before | Related context after | Median builder time before / after |
|---|---|---|---:|
| Authentication | Two unrelated graphics `encode` methods | Both omitted | 69.7 / 33.7 ms |
| Video export | `run_overlay_only_export`, `probe_source_video` | Both preserved | 37.2 / 40.7 ms |

Primary result paths, start lines, and function-header metadata were unchanged.
The auth result now avoids misleading extra context; this does not establish
better recall for the missing core OAuth implementation or fewer Codex calls.
The portable MCP regression was confirmed failing before the fix and passing
after it, while also requiring a valid helper, unchanged primary source, one
Jev request, and the complete response budget. Additional unit cases cover
multiline qualification/imports, aliases, duplicate names, unrelated scopes,
receiver calls, compatible languages, and shadowing.

Final validation after the precision fix: 101 tests passed; one real-Keychain
test was intentionally ignored. Formatting, Clippy with warnings denied, and
the locked release build all passed.

## Next real-client measurement

Install the new release build and start a fresh task. Keep the same question,
model and reasoning settings. Compare total answer time and follow-up call count,
using returned preparation, scan, shortlist, preview, rerank, context and total
server timings to distinguish Oko work from agent overhead. Existing benchmark
accuracy figures predate this preview change and should not be reused as proof.
