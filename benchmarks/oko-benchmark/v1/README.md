# `oko-benchmark/v1`

This directory defines the sanitized interchange format for benchmark evidence.
It is deliberately small enough for a Python runner in another repository to
write with the standard library.

## Bundle

`benchmark.json` is the one self-contained, Slack-shareable artifact. It contains
the format/version envelope, pinned manifest, sanitized runner settings, canary
result, records, grouped summary, human-readable report, and explicit privacy
assertions. Upload this file only. Runners may keep `run-manifest.json`,
`records.jsonl`, `summary.json`, and `report.md` beside it as local compatibility
artifacts, but those files are not needed to interpret the shared result.

Each record is one attempt. Task identity is an opaque SHA-256 hash and
`taskKind` is explicitly `search`, `edit`, or `unknown`, so retrieval and edit
results cannot be merged accidentally. Conditions are `native`, `oko-cold`, and
`oko-warm`; `condition.observedCacheState` is copied from Oko output and remains
`null` when no trustworthy observation exists. It is never inferred from the
requested condition. Summary groups include client, model, effort, requested and
observed condition, and task kind.

Use the JSON Schema files in this directory with JSON Schema draft 2020-12.
The repository's offline Python test also validates the checked-in examples
without adding a runtime dependency on a schema package.

## Measurement boundary

Rust measures Oko internals: cache/retrieval phases and each Jev transport call
(phase, monotonic duration, serialized request/response byte counts, HTTP status,
success/error class, and provider-reported usage when present). Rust never emits
request/response bodies, prompts, source content, headers, or credentials in the
safe summaries.

Python measures the outer client attempt with
[`time.perf_counter_ns()`](https://docs.python.org/3/library/time.html#time.perf_counter_ns),
writes JSONL, validates privacy, and aggregates. Total wall time is authoritative
and is never reconstructed by adding overlapping cache/retrieval/provider phases.
Missing usage is `null`; no token estimator is used. Input, output, and cache
fields may be present without a total. A total is copied only from a provider's
explicit aggregate total field; it is never reconstructed by adding partial
fields or summing per-step values.

The OpenCode adapter must correlate each `tool_use` event with its own
`part.state.output` payload. Decode only that existing event field; do not invent
aliases. Preserve provider-reported per-step usage and Oko/Jev metadata from the
correlated result. If the same provider call appears in multiple metadata paths,
deduplicate it by its safe serialized call fields before aggregation.

For `oko-cold`, a timed client run is valid only when Oko reports cache status
`cold`. For `oko-warm`, the runner must prebuild the exact disposable workspace
cache before starting the timed client process and the timed result must report
status `disk`, with reused files and no rebuilt files. A `memory` result is not a
warm disk result. Missing or mismatched observations invalidate the run.

The standard-library writer validates the manifest, every record, and the
generated summary against the checked-in schemas before creating any bundle
files. Unknown fields are rejected wherever the schemas disallow additional
properties.

The design is informed by the
[OpenTelemetry GenAI span](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md),
[metric](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-metrics.md),
and [attribute](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md)
conventions, but does not add an OpenTelemetry SDK or collector. HTTP status and
request/response byte semantics follow [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html).

## Future runner example

```python
from pathlib import Path
import benchmark_observability as obs

run_id = "results-2026-09-20"
start = obs.perf_counter_ns()
# Run the paid/live client session here. Keep its raw logs outside `shareable/`.
duration = obs.elapsed_ns(start)
record = obs.make_record(
    run_id=run_id,
    record_id="001",
    task_id="opaque-task-name",
    client="codex",
    client_version="example-version",
    model="example-model",
    effort="medium",
    enabled=True,
    task={"kind": "search", "cacheCondition": "cold"},
    target_commit="0123456789abcdef0123456789abcdef01234567",
    oko_commit="89abcdef0123456789abcdef0123456789abcdef",
    oko_version="0.2.1",
    row={
        "durationNs": duration,
        "toolCalls": 2,
        "okoCalls": 1,
        "usage": None,
        "grade": {"passed": True},
    },
    total_wall_ns=duration,
)
manifest = obs.manifest(
    run_id=run_id,
    target={"repository": "example", "commit": "0123456789abcdef0123456789abcdef01234567", "version": None},
    oko={"repository": "bartlomein/oko", "commit": "89abcdef0123456789abcdef0123456789abcdef", "version": "0.2.1", "binarySha256": None},
    clients=[{"name": "codex", "version": "example-version", "model": "example-model", "effort": "medium"}],
)
obs.write_bundle(
    Path("shareable"),
    manifest,
    [record],
    settings={
        "preset": "pilot",
        "clients": ["codex"],
        "conditions": ["oko-cold"],
        "repeats": 1,
        "taskIds": ["opaque-task-name"],
        "models": {"codex": "example-model"},
        "effort": "medium",
        "timeoutSeconds": 300,
        "isolation": "example",
        "cachePolicy": "cold-vs-prebuilt-disk-v1",
    },
    canary=None,
)
```

Import `scripts/benchmark_observability.py` directly; runners in a nested
directory should add the parent `scripts/` directory to `sys.path` or package
it locally. Keep provider/client raw event artifacts separate from the generated
bundle. The current Twenty runner is the reference integration.
