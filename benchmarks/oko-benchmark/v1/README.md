# `oko-benchmark/v1`

This directory defines the sanitized interchange format for benchmark evidence.
It is deliberately small enough for a Python runner in another repository to
write with the standard library.

## Bundle

Each shareable bundle contains:

- `run-manifest.json`: pinned target and Oko commits/versions, client/model/
  effort metadata, standards references, and explicit privacy assertions.
- `records.jsonl`: one attempt per line. Task identity is an opaque SHA-256
  hash; prompts, source bodies, snippets, tool arguments, raw events, stderr,
  headers, keys, and absolute paths are not allowed.
- `summary.json`: grouped denominators, failures, correctness counts, median
  and p95 successful wall time, and separate agent/Jev token sections.
- `report.md`: a human-readable rendering of `summary.json`.

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
Missing usage is `null`; no token estimator is used.

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
    task={"cacheCondition": "cold"},
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
obs.write_bundle(Path("shareable"), manifest, [record])
```

Import `scripts/benchmark_observability.py` directly; runners in a nested
directory should add the parent `scripts/` directory to `sys.path` or package
it locally. Keep provider/client raw event artifacts separate from the generated
bundle. The current Twenty runner is the reference integration.
