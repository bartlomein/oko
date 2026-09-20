"""Privacy-first writers for the ``oko-benchmark/v1`` shareable format.

The module intentionally uses only the Python standard library.  Benchmark
runners may keep richer local/debug artifacts, but anything written through
this module is validated before it enters a shareable bundle.
"""

from __future__ import annotations

import json
import hashlib
import math
import re
import statistics
import time
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


FORMAT = "oko-benchmark/v1"
FORMAT_VERSION = 1
SCHEMA_DRAFT = "https://json-schema.org/draft/2020-12/schema"

PRIVACY_ASSERTIONS = {
    "promptRecorded": False,
    "sourceBodiesRecorded": False,
    "rawEventLogsRecorded": False,
    "headersRecorded": False,
    "keysRecorded": False,
    "toolArgumentsRecorded": False,
    "evidenceBodiesRecorded": False,
    "stderrRecorded": False,
    "absolutePathsRecorded": False,
}

_RECORDED_ASSERTION_KEYS = set(PRIVACY_ASSERTIONS)
_FORBIDDEN_KEYS = {
    "prompt",
    "prompt_text",
    "promptText",
    "question",
    "question_text",
    "questionText",
    "sourceText",
    "sourceBody",
    "sourceBodies",
    "snippet",
    "snippets",
    "evidence",
    "evidenceBody",
    "evidenceText",
    "rawEvents",
    "events",
    "event",
    "eventLog",
    "raw",
    "logs",
    "log",
    "stderr",
    "headers",
    "requestHeaders",
    "responseHeaders",
    "auth",
    "authorization",
    "apiKey",
    "accessToken",
    "refreshToken",
    "secret",
    "secrets",
    "toolArguments",
    "tool_arguments",
    "toolInput",
    "toolOutput",
    "arguments",
    "final",
    "response",
    "responseBody",
    "localPath",
    "absolutePath",
    "workingDirectory",
    "cwd",
    "artifact",
}
_FORBIDDEN_NORMALIZED_KEYS = {
    re.sub(r"[^a-z0-9]", "", key.casefold()) for key in _FORBIDDEN_KEYS
}
_SECRET_PATTERN = re.compile(
    r"(?:api[_-]?key|authorization|bearer|secret|password)\s*[:=]\s*\S+"
    r"|(?:sk-|ghp_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{10,}"
    r"|-----BEGIN [A-Z ]+ PRIVATE KEY-----",
    re.IGNORECASE,
)
_ABSOLUTE_PATH_PATTERN = re.compile(
    r"(?<![A-Za-z0-9_./-])/(?!/)(?=[A-Za-z0-9._~%+-]|$)[^\s\"'<>]*"
)
_WINDOWS_PATH_PATTERN = re.compile(r"(?<![A-Za-z0-9_])[A-Za-z]:\\")

_CACHE_FIELDS = (
    "status", "scanMs", "loadMs", "scanLoadOverlapped", "prepareMs",
    "aggregateMs", "aggregateReused", "navigationMs", "saveMs", "totalMs",
    "reusedFiles", "rebuiltFiles", "readFiles", "reusedContents", "validation",
    "validationReason", "fallbackReason",
)
_TIMING_FIELDS = (
    "preparationMs", "scanMs", "shortlistMs", "investigateMs", "contextMs",
    "totalWallNs", "totalMs",
)
_RETRIEVAL_FIELDS = (
    "shortlistedCandidates", "rankedCandidates", "omittedCandidates", "requestBytes",
    "previewMs", "rerankMs", "attempts", "recoveryCandidates", "recovered",
    "candidateCount", "omittedCount",
)
_INVESTIGATION_FIELDS = ("steps", "jevCalls", "stopReason", "complete")
_USAGE_FIELDS = (
    "inputTokens", "outputTokens", "cacheReadTokens", "cacheWriteTokens",
    "reasoningTokens",
)


class PrivacyError(ValueError):
    """Raised when a value cannot be included in a shareable bundle."""


def perf_counter_ns() -> int:
    """Return a monotonic nanosecond reading for benchmark wall timing."""

    return time.perf_counter_ns()


def elapsed_ns(start_ns: int, end_ns: int | None = None) -> int:
    end_ns = perf_counter_ns() if end_ns is None else end_ns
    if end_ns < start_ns:
        raise ValueError("monotonic end time precedes start time")
    return end_ns - start_ns


def _number(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if isinstance(value, float) and (not math.isfinite(value) or value < 0):
        return None
    if value < 0 or int(value) != value:
        return None
    return int(value)


def normalize_usage(value: Any, *, aliases: Mapping[str, Sequence[str]] | None = None) -> dict[str, int | None] | None:
    """Normalize provider-reported usage without estimating missing totals.

    ``totalTokens`` is copied only from an explicit provider field.  It is
    never derived from input/output fields.
    """

    if value is None:
        return None
    aliases = aliases or {
        "inputTokens": ("inputTokens", "input_tokens", "prompt_tokens", "input"),
        "outputTokens": ("outputTokens", "output_tokens", "completion_tokens", "output"),
        "cacheReadTokens": (
            "cacheReadTokens",
            "cache_read_input_tokens",
            "cached_input_tokens",
            "cache_read_tokens",
        ),
        "cacheWriteTokens": (
            "cacheWriteTokens",
            "cache_creation_input_tokens",
            "cache_write_tokens",
        ),
        "totalTokens": ("totalTokens", "total_tokens", "total"),
        "reasoningTokens": ("reasoningTokens", "reasoning_tokens", "reasoning"),
    }
    if not isinstance(value, Mapping):
        return None
    result: dict[str, int | None] = {}
    for target, names in aliases.items():
        result[target] = next((_number(value[name]) for name in names if name in value), None)
    if isinstance(value.get("cache"), Mapping):
        cache = value["cache"]
        if result["cacheReadTokens"] is None:
            result["cacheReadTokens"] = _number(cache.get("read"))
        if result["cacheWriteTokens"] is None:
            result["cacheWriteTokens"] = _number(cache.get("write"))
    return result if any(item is not None for item in result.values()) else None


def _safe_failure_class(error: Any) -> str | None:
    if error is None:
        return None
    if isinstance(error, str):
        value = error
    elif isinstance(error, Mapping):
        value = str(error.get("class") or error.get("type") or "unknown")
    else:
        value = type(error).__name__
    value = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("_")
    return value[:80] or "unknown"


def _condition(task: Mapping[str, Any], enabled: bool) -> tuple[str, str | None]:
    if not enabled:
        return "native", "native"
    requested = task.get("cacheCondition")
    if requested in {"cold", "oko-cold"}:
        requested = "oko-cold"
    elif requested in {"warm", "oko-warm"}:
        requested = "oko-warm"
    else:
        requested = "oko-cold"
    observed = task.get("observedCacheState")
    if observed not in {"cold", "disk", "memory", "refresh", "disabled", "native", None}:
        observed = None
    return requested, observed


def _normalized_key(key: str) -> str:
    return re.sub(r"[^a-z0-9]", "", key.casefold())


def _contains_absolute_path(value: str) -> bool:
    for match in _ABSOLUTE_PATH_PATTERN.finditer(value):
        prefix = value[: match.start()]
        if re.search(r"[A-Za-z][A-Za-z0-9+.-]*:$", prefix):
            continue
        return True
    return bool(_WINDOWS_PATH_PATTERN.search(value))


def _agent_usage(row: Mapping[str, Any]) -> dict[str, int | None] | None:
    usage = row.get("agentUsage") or row.get("usage")
    if isinstance(usage, Mapping):
        return normalize_usage(usage)
    tokens = row.get("tokens")
    if isinstance(tokens, Mapping):
        return normalize_usage(tokens)
    return None


def _normalize_usage_step(step: Any) -> dict[str, int | None] | None:
    if isinstance(step, Mapping) and isinstance(step.get("tokens"), Mapping):
        normalized = normalize_usage(step["tokens"])
        declared = normalize_usage(step.get("usage"))
        if normalized is None:
            normalized = declared
        elif declared is not None:
            for key, value in declared.items():
                if value is not None:
                    normalized[key] = value
        return normalized
    return normalize_usage(step)


def _agent_usage_steps(row: Mapping[str, Any]) -> list[dict[str, int | None]] | None:
    steps = row.get("agentUsageSteps")
    if not isinstance(steps, list):
        usage = row.get("usage")
        steps = usage if isinstance(usage, list) else []
    normalized = [_normalize_usage_step(step) for step in steps]
    normalized = [step for step in normalized if step is not None]
    return normalized or None


def _aggregate_usage_steps(steps: Sequence[Mapping[str, Any]] | None) -> dict[str, int | None] | None:
    if not steps:
        return None
    result = {}
    for field in _USAGE_FIELDS:
        values = [
            int(step[field])
            for step in steps
            if isinstance(step.get(field), int) and not isinstance(step[field], bool)
        ]
        result[field] = sum(values) if values else None
    result["totalTokens"] = None
    return result if any(value is not None for value in result.values()) else None


def _safe_fields(value: Any, fields: Sequence[str]) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    result = {}
    for field in fields:
        if field not in value:
            continue
        item = value[field]
        if item is None or isinstance(item, (bool, str)):
            result[field] = item
        else:
            number = _number(item)
            if number is not None:
                result[field] = number
    return result or None


def is_oko_tool(tool: Mapping[str, Any]) -> bool:
    name = tool.get("name") or tool.get("tool")
    return tool.get("server") == "oko" or name in ("oko_search", "mcp__oko__search")


def read_oko_metrics(path: Any) -> list[dict[str, Any]]:
    """Read the JSON lines Oko appends to ``OKO_METRICS_FILE``, one per completed search.

    Oko keeps timings and retrieval metadata out of the agent-visible tool
    result, so the launcher points this file into the trial directory.
    """

    try:
        with open(path, encoding="utf-8") as handle:
            lines = handle.read().splitlines()
    except FileNotFoundError:
        return []
    metrics = []
    for line in lines:
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict):
            metrics.append(value)
    return metrics


def attach_oko_metrics(tools: Sequence[dict[str, Any]], path: Any) -> int:
    """Attach each recorded search to the matching Oko tool call, in order.

    A failed search records nothing, so surplus lines stay with the last Oko
    call rather than being dropped. Returns the number of recorded searches.
    """

    metrics = read_oko_metrics(path)
    calls = [tool for tool in tools if is_oko_tool(tool)]
    for tool, metric in zip(calls, metrics):
        tool["okoMetrics"] = [metric]
    if calls and len(metrics) > len(calls):
        calls[-1]["okoMetrics"].extend(metrics[len(calls):])
    return len(metrics)


def _safe_phase_metrics(value: Any) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    if isinstance(value, Mapping):
        timings = _safe_fields(value.get("timings"), _TIMING_FIELDS)
        cache = _safe_fields(
            value.get("timings", {}).get("cache")
            if isinstance(value.get("timings"), Mapping)
            else value.get("cache"),
            _CACHE_FIELDS,
        )
        retrieval = _safe_fields(value.get("retrieval"), _RETRIEVAL_FIELDS)
        investigation = _safe_fields(value.get("investigation"), _INVESTIGATION_FIELDS)
        metric = {}
        if timings:
            metric["timings"] = timings
        if cache:
            metric["cache"] = cache
        if retrieval:
            metric["retrieval"] = retrieval
        if investigation:
            metric["investigation"] = investigation
        if metric and metric not in found:
            found.append(metric)
        for child in value.values():
            found.extend(_safe_phase_metrics(child))
    elif isinstance(value, list):
        for child in value:
            found.extend(_safe_phase_metrics(child))
    return found


def _find_safe_provider_calls(value: Any) -> list[dict[str, Any]]:
    """Extract already-serialized safe Rust call summaries from Oko output.

    The function copies only the allowlisted fields and never forwards the
    surrounding tool result, which may contain source evidence or arguments.
    """

    calls: list[dict[str, Any]] = []
    if isinstance(value, Mapping):
        for field in ("providerCalls", "jevCalls"):
            candidates = value.get(field)
            if not isinstance(candidates, list):
                continue
            for call in candidates:
                if not isinstance(call, Mapping):
                    continue
                calls.append(
                    {
                        "phase": str(call.get("phase") or "unknown"),
                        "durationNs": _number(call.get("durationNs")),
                        "requestBytes": _number(call.get("requestBytes")),
                        "responseBytes": _number(call.get("responseBytes")),
                        "httpStatus": _number(call.get("httpStatus")),
                        "success": call.get("success") if isinstance(call.get("success"), bool) else False,
                        "errorClass": _safe_failure_class(call.get("errorClass")),
                        "usage": normalize_usage(call.get("usage")),
                    }
                )
        for child in value.values():
            calls.extend(_find_safe_provider_calls(child))
    elif isinstance(value, list):
        for child in value:
            calls.extend(_find_safe_provider_calls(child))
    return calls


def _provider_calls(row: Mapping[str, Any]) -> list[dict[str, Any]]:
    calls = _find_safe_provider_calls(row.get("tools", []))
    unique = []
    seen: dict[str, dict[str, Any]] = {}
    for call in calls:
        identity = {key: value for key, value in call.items() if key != "usage"}
        marker = json.dumps(identity, sort_keys=True, separators=(",", ":"))
        existing = seen.get(marker)
        if existing is None:
            seen[marker] = call
            unique.append(call)
        elif existing.get("usage") is None and call.get("usage") is not None:
            existing["usage"] = call["usage"]
    return unique


def _correctness(row: Mapping[str, Any]) -> dict[str, Any]:
    grade = row.get("grade") or {}
    booleans = {
        key: grade.get(key) if isinstance(grade.get(key), bool) else None
        for key in ("correctFirst", "correctTopFive", "expectedPatchMatch", "passed")
    }
    known = any(value is not None for value in booleans.values())
    return {
        **booleans,
        "known": known,
        "quality": "exact" if grade.get("expectedPatchMatch") is True else "correct" if grade.get("correctTopFive") is True else "unknown",
    }


def make_record(
    *,
    run_id: str,
    record_id: str,
    task_id: str,
    client: str,
    client_version: str | None,
    model: str | None,
    effort: str | None,
    enabled: bool,
    task: Mapping[str, Any] | None = None,
    target_commit: str | None = None,
    oko_commit: str | None = None,
    oko_version: str | None = None,
    row: Mapping[str, Any] | None = None,
    correctness: Mapping[str, Any] | None = None,
    total_wall_ns: int | None = None,
) -> dict[str, Any]:
    task = task or {}
    row = row or {}
    condition_task = dict(task)
    if row.get("observedCacheState") is not None:
        condition_task["observedCacheState"] = row["observedCacheState"]
    requested_condition, observed_condition = _condition(condition_task, enabled)
    provider_calls = _provider_calls(row)
    usage = _agent_usage(row)
    usage_steps = _agent_usage_steps(row)
    if usage is None:
        usage = _aggregate_usage_steps(usage_steps)
    phase_metrics = _safe_phase_metrics(row.get("tools", []))
    oko_wall_ns = next(
        (
            metric.get("timings", {}).get("totalWallNs")
            for metric in phase_metrics
            if isinstance(metric.get("timings"), Mapping)
            and metric["timings"].get("totalWallNs") is not None
        ),
        None,
    )
    task_kind = str(task.get("kind") or task.get("taskKind") or row.get("kind") or "unknown")
    requested_condition = row.get("condition") or requested_condition
    if requested_condition not in {"native", "oko-cold", "oko-warm"}:
        requested_condition, observed_condition = _condition(condition_task, enabled)
    else:
        observed_condition = row.get("observedCacheState")
    status = "succeeded" if not row.get("error") else "timeout" if "Timeout" in str(row.get("error")) else "failed"
    record = {
        "format": FORMAT,
        "runId": run_id,
        "recordId": record_id,
        "task": {"idHash": hashlib.sha256(task_id.encode()).hexdigest()},
        "taskKind": task_kind,
        "target": {"commit": target_commit},
        "oko": {"commit": oko_commit, "version": oko_version},
        "client": {"name": client, "version": client_version, "model": model, "effort": effort},
        "condition": {"requested": requested_condition, "observedCacheState": observed_condition},
        "status": status,
        "failure": {"class": _safe_failure_class(row.get("errorType") or row.get("error")) if row.get("error") else None},
        "timing": {
            "totalWallNs": total_wall_ns if total_wall_ns is not None else _number(row.get("durationNs")),
            "agentWallNs": _number(row.get("durationNs")),
            "okoWallNs": oko_wall_ns,
            "monotonic": True,
        },
        "correctness": dict(correctness or _correctness(row)),
        "calls": {
            "agentToolCalls": _number(row.get("toolCalls")),
            "okoCalls": _number(row.get("okoCalls")),
            "jevCalls": 0 if not enabled else len(provider_calls) if provider_calls else None,
        },
        "agentUsage": usage,
        "agentUsageSteps": usage_steps,
        "okoUsage": {
            "phaseMetrics": phase_metrics,
            "providerCalls": provider_calls,
            "tokenUsage": [call["usage"] for call in provider_calls if call.get("usage") is not None] or None,
        },
        "privacy": dict(PRIVACY_ASSERTIONS),
    }
    validate_shareable(record)
    return record


def _values(records: Iterable[Mapping[str, Any]], *keys: str) -> list[Any]:
    values = []
    for record in records:
        value: Any = record
        for key in keys:
            if not isinstance(value, Mapping):
                value = None
                break
            value = value.get(key)
        values.append(value)
    return values


def _sum_if_complete(values: Iterable[Any]) -> int | None:
    values = list(values)
    if not values or any(value is None for value in values):
        return None
    return sum(int(value) for value in values)


def _jev_usage(group: Iterable[Mapping[str, Any]], key: str) -> int | None:
    calls = [call for record in group for call in record.get("okoUsage", {}).get("providerCalls", [])]
    if not calls:
        return None
    return _sum_if_complete(
        call.get("usage", {}).get(key) if isinstance(call.get("usage"), Mapping) else None
        for call in calls
    )


def _percentile(values: Sequence[int], percentile: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * percentile) - 1)
    return ordered[index]


def aggregate(records: Sequence[Mapping[str, Any]], *, run_id: str, generated_at: str | None = None) -> dict[str, Any]:
    groups: list[dict[str, Any]] = []
    keys = {
        (
            r["client"]["name"],
            r["client"].get("model"),
            r["client"].get("effort"),
            r["condition"]["requested"],
            r["condition"].get("observedCacheState"),
            r["taskKind"],
        )
        for r in records
    }
    keys = sorted(keys, key=lambda key: tuple("" if value is None else str(value) for value in key))
    for client, model, effort, requested_condition, observed_cache_state, task_kind in keys:
        group = [
            r
            for r in records
            if (
                r["client"]["name"],
                r["client"].get("model"),
                r["client"].get("effort"),
                r["condition"]["requested"],
                r["condition"].get("observedCacheState"),
                r["taskKind"],
            )
            == (client, model, effort, requested_condition, observed_cache_state, task_kind)
        ]
        successful = [r for r in group if r["status"] == "succeeded"]
        durations = [v for v in _values(successful, "timing", "totalWallNs") if isinstance(v, int)]
        correct = [r for r in group if r.get("correctness", {}).get("known")]
        groups.append(
            {
                "client": {"name": client, "model": model, "effort": effort},
                "condition": {"requested": requested_condition, "observedCacheState": observed_cache_state},
                "taskKind": task_kind,
                "attempted": len(group),
                "succeeded": len(successful),
                "failed": len(group) - len(successful),
                "denominator": len(group),
                "latencyNs": {
                    "observed": len(durations),
                    "median": int(statistics.median(durations)) if durations else None,
                    "p95": _percentile(durations, 0.95),
                },
                "correctness": {
                    "known": len(correct),
                    "correct": sum(r["correctness"].get("correctTopFive") is True or r["correctness"].get("expectedPatchMatch") is True for r in correct),
                },
                "agentUsage": {
                    key: _sum_if_complete(_values(group, "agentUsage", key))
                    for key in ("inputTokens", "cacheReadTokens", "cacheWriteTokens", "outputTokens", "totalTokens", "reasoningTokens")
                },
                "jevUsage": {
                    "calls": sum(len(r.get("okoUsage", {}).get("providerCalls", [])) for r in group),
                    "inputTokens": _jev_usage(group, "inputTokens"),
                    "outputTokens": _jev_usage(group, "outputTokens"),
                    "cacheReadTokens": _jev_usage(group, "cacheReadTokens"),
                    "cacheWriteTokens": _jev_usage(group, "cacheWriteTokens"),
                    "totalTokens": _jev_usage(group, "totalTokens"),
                    "reasoningTokens": _jev_usage(group, "reasoningTokens"),
                },
            }
        )
    return {
        "format": FORMAT,
        "runId": run_id,
        "generatedAt": generated_at,
        "records": {"attempted": len(records), "succeeded": sum(r["status"] == "succeeded" for r in records), "failed": sum(r["status"] != "succeeded" for r in records)},
        "groups": groups,
        "privacy": dict(PRIVACY_ASSERTIONS),
    }


def report_markdown(summary: Mapping[str, Any]) -> str:
    lines = [
        "# Oko benchmark report",
        "",
        f"Format: `{summary['format']}`",
        f"Records: {summary['records']['succeeded']}/{summary['records']['attempted']} succeeded; {summary['records']['failed']} failed.",
        "",
        "| Client | Model | Effort | Task kind | Requested | Observed cache | Succeeded / attempted | Median ms | p95 ms | Correct / known | Agent tokens | Jev calls |",
        "|---|---|---|---|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for group in summary["groups"]:
        latency = group["latencyNs"]
        median = "—" if latency["median"] is None else f"{latency['median'] / 1_000_000:.2f}"
        p95 = "—" if latency["p95"] is None else f"{latency['p95'] / 1_000_000:.2f}"
        usage = group["agentUsage"]["totalTokens"]
        usage = "—" if usage is None else str(usage)
        lines.append(
            f"| {group['client']['name']} | {group['client']['model'] or '—'} | {group['client']['effort'] or '—'} | {group['taskKind']} | {group['condition']['requested']} | {group['condition']['observedCacheState'] or '—'} | {group['succeeded']}/{group['attempted']} | {median} | {p95} | {group['correctness']['correct']}/{group['correctness']['known']} | {usage} | {group['jevUsage']['calls']} |"
        )
    lines += [
        "",
        "Failed attempts remain in denominators. Latency percentiles use successful observations only; missing usage remains unavailable.",
        "",
        "Rust records safe Oko/Jev transport summaries. Python records client wall time with `time.perf_counter_ns()` and aggregates without adding overlapping phases.",
    ]
    return "\n".join(lines) + "\n"


def validate_shareable(value: Any, *, _path: str = "$") -> None:
    if isinstance(value, Mapping):
        for key, child in value.items():
            if not isinstance(key, str):
                raise PrivacyError(f"{_path} contains a non-string key")
            if key in _RECORDED_ASSERTION_KEYS:
                if not isinstance(child, bool):
                    raise PrivacyError(f"{_path}.{key} must be boolean")
            elif (
                _normalized_key(key) in _FORBIDDEN_NORMALIZED_KEYS
                or _normalized_key(key).endswith("body")
                or _normalized_key(key).endswith("snippet")
            ):
                raise PrivacyError(f"forbidden shareable field: {_path}.{key}")
            elif _normalized_key(key) in {"token", "tokens", "credential", "credentials", "key"}:
                raise PrivacyError(f"forbidden shareable field: {_path}.{key}")
            validate_shareable(child, _path=f"{_path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            validate_shareable(child, _path=f"{_path}[{index}]")
    elif isinstance(value, str):
        if _SECRET_PATTERN.search(value) or _contains_absolute_path(value):
            raise PrivacyError(f"forbidden secret or absolute path at {_path}")


def _schema_error(
    schema: Mapping[str, Any],
    value: Any,
    path: str,
    root: Mapping[str, Any] | None = None,
) -> str | None:
    root = schema if root is None else root
    reference = schema.get("$ref")
    if isinstance(reference, str) and reference.startswith("#/"):
        target: Any = root
        for part in reference[2:].split("/"):
            part = part.replace("~1", "/").replace("~0", "~")
            target = target.get(part) if isinstance(target, Mapping) else None
        if not isinstance(target, Mapping):
            return f"{path} has unresolved reference {reference}"
        return _schema_error(target, value, path, root)
    if "const" in schema and value != schema["const"]:
        return f"{path} must equal const"
    expected = schema.get("type")
    if isinstance(expected, list):
        if not any(_schema_error({"type": item}, value, path, root) is None for item in expected):
            return f"{path} has the wrong type"
    elif expected == "object" and not isinstance(value, Mapping):
        return f"{path} must be object"
    elif expected == "array" and not isinstance(value, list):
        return f"{path} must be array"
    elif expected == "string" and not isinstance(value, str):
        return f"{path} must be string"
    elif expected == "integer" and (isinstance(value, bool) or not isinstance(value, int)):
        return f"{path} must be integer"
    elif expected == "boolean" and not isinstance(value, bool):
        return f"{path} must be boolean"
    elif expected == "null" and value is not None:
        return f"{path} must be null"
    if "minLength" in schema and isinstance(value, str) and len(value) < schema["minLength"]:
        return f"{path} is shorter than minLength"
    if "enum" in schema and value not in schema["enum"]:
        return f"{path} is not in enum"
    if "anyOf" in schema and not any(_schema_error(option, value, path, root) is None for option in schema["anyOf"]):
        return f"{path} does not match anyOf"
    if isinstance(value, Mapping):
        missing = [key for key in schema.get("required", []) if key not in value]
        if missing:
            return f"{path} missing required {missing}"
        properties = schema.get("properties", {})
        unknown = set(value) - set(properties)
        additional = schema.get("additionalProperties", True)
        if unknown and additional is False:
            return f"{path} has unknown properties {sorted(unknown)}"
        for key, child in value.items():
            if key in properties:
                child_schema = properties[key]
            elif isinstance(additional, Mapping):
                child_schema = additional
            else:
                continue
            error = _schema_error(child_schema, child, f"{path}.{key}", root)
            if error:
                return error
    if isinstance(value, list) and schema.get("items"):
        for index, child in enumerate(value):
            error = _schema_error(schema["items"], child, f"{path}[{index}]", root)
            if error:
                return error
    return None


def validate_schema(schema: Mapping[str, Any], value: Any) -> None:
    error = _schema_error(schema, value, "$")
    if error:
        raise ValueError(error)
    validate_shareable(value)


def build_envelope(
    manifest: Mapping[str, Any],
    records: Sequence[Mapping[str, Any]],
    *,
    settings: Mapping[str, Any] | None = None,
    canary: Mapping[str, Any] | None = None,
    report: str | None = None,
) -> dict[str, Any]:
    summary = aggregate(records, run_id=str(manifest["runId"]))
    envelope_settings = settings or {
        "preset": "pilot",
        "clients": [],
        "conditions": [],
        "repeats": 1,
        "taskIds": [],
        "models": {},
        "effort": "unknown",
        "timeoutSeconds": 0,
        "isolation": "unspecified",
        "cachePolicy": "unspecified",
    }
    return {
        "format": FORMAT,
        "formatVersion": FORMAT_VERSION,
        "manifest": dict(manifest),
        "settings": dict(envelope_settings),
        "canary": dict(canary) if canary is not None else None,
        "records": [dict(record) for record in records],
        "summary": summary,
        "report": report if report is not None else report_markdown(summary),
        "privacy": dict(PRIVACY_ASSERTIONS),
    }


def validate_benchmark(envelope: Mapping[str, Any]) -> None:
    schema_root = Path(__file__).resolve().parent.parent / "benchmarks/oko-benchmark/v1"
    schemas = {
        name: json.loads((schema_root / name).read_text())
        for name in (
            "benchmark.schema.json",
            "run-manifest.schema.json",
            "record.schema.json",
            "summary.schema.json",
        )
    }
    validate_schema(schemas["benchmark.schema.json"], envelope)
    validate_schema(schemas["run-manifest.schema.json"], envelope["manifest"])
    for record in envelope["records"]:
        validate_schema(schemas["record.schema.json"], record)
    validate_schema(schemas["summary.schema.json"], envelope["summary"])


def write_bundle(
    directory: Path,
    manifest: Mapping[str, Any],
    records: Sequence[Mapping[str, Any]],
    *,
    settings: Mapping[str, Any] | None = None,
    canary: Mapping[str, Any] | None = None,
    report: str | None = None,
) -> dict[str, Any]:
    schema_root = Path(__file__).resolve().parent.parent / "benchmarks/oko-benchmark/v1"
    schemas = {
        name: json.loads((schema_root / name).read_text())
        for name in ("run-manifest.schema.json", "record.schema.json", "summary.schema.json")
    }
    validate_schema(schemas["run-manifest.schema.json"], manifest)
    for record in records:
        validate_schema(schemas["record.schema.json"], record)
    summary = aggregate(records, run_id=str(manifest["runId"]))
    validate_schema(schemas["summary.schema.json"], summary)
    envelope = build_envelope(
        manifest,
        records,
        settings=settings,
        canary=canary,
        report=report or report_markdown(summary),
    )
    validate_benchmark(envelope)

    directory.mkdir(parents=True, exist_ok=True)
    (directory / "benchmark.json").write_text(json.dumps(envelope, indent=2) + "\n")
    (directory / "run-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    with (directory / "records.jsonl").open("w") as stream:
        for record in records:
            stream.write(json.dumps(record, sort_keys=True) + "\n")
    (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (directory / "report.md").write_text(envelope["report"])
    return summary


def manifest(*, run_id: str, target: Mapping[str, Any], oko: Mapping[str, Any], clients: Sequence[Mapping[str, Any]], runner_version: str = "benchmark-twenty") -> dict[str, Any]:
    value = {
        "format": FORMAT,
        "formatVersion": FORMAT_VERSION,
        "runId": run_id,
        "runner": {"name": runner_version, "version": "1"},
        "target": dict(target),
        "oko": dict(oko),
        "clients": [dict(client) for client in clients],
        "privacy": dict(PRIVACY_ASSERTIONS),
        "standards": {
            "pythonPerfCounterNs": "https://docs.python.org/3/library/time.html#time.perf_counter_ns",
            "otelGenAiSpans": "https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md",
            "otelGenAiMetrics": "https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-metrics.md",
            "otelGenAiAttributes": "https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md",
            "httpSemantics": "https://www.rfc-editor.org/rfc/rfc9110.html",
        },
    }
    validate_shareable(value)
    return value
