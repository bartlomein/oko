#!/usr/bin/env python3
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchmark_observability as obs


ROOT = Path(__file__).resolve().parents[1]
FORMAT_ROOT = ROOT / "benchmarks/oko-benchmark/v1"


class ObservabilityTests(unittest.TestCase):
    def record(self, record_id, *, duration, status="succeeded", condition="oko-cold", usage=None, grade=None, task_kind="search", observed=None, tools=None):
        return obs.make_record(
            run_id="test-run",
            record_id=record_id,
            task_id="task-" + record_id,
            client="codex",
            client_version="test-client",
            model="test-model",
            effort="medium",
            enabled=condition != "native",
            task={"cacheCondition": "warm" if condition == "oko-warm" else "cold", "kind": task_kind},
            target_commit="a" * 40,
            oko_commit="b" * 40,
            oko_version="0.2.1",
            row={
                "durationNs": duration,
                "toolCalls": 2,
                "okoCalls": 1 if condition != "native" else 0,
                "usage": usage,
                "grade": grade or {"correctTopFive": status == "succeeded"},
                "error": None if status == "succeeded" else "failed model session",
                "errorType": None if status == "succeeded" else "answer",
                "tools": tools or [],
                "observedCacheState": observed,
            },
            total_wall_ns=duration,
        )

    def test_checked_in_schemas_and_examples(self):
        envelope = json.loads((FORMAT_ROOT / "examples/benchmark.json").read_text())
        benchmark_schema = json.loads((FORMAT_ROOT / "benchmark.schema.json").read_text())
        obs.validate_schema(benchmark_schema, envelope)
        obs.validate_benchmark(envelope)
        manifest = json.loads((FORMAT_ROOT / "examples/run-manifest.json").read_text())
        summary = json.loads((FORMAT_ROOT / "examples/summary.json").read_text())
        records = [json.loads(line) for line in (FORMAT_ROOT / "examples/records.jsonl").read_text().splitlines()]
        for schema_name, value in (("run-manifest.schema.json", manifest), ("summary.schema.json", summary)):
            obs.validate_schema(json.loads((FORMAT_ROOT / schema_name).read_text()), value)
        record_schema = json.loads((FORMAT_ROOT / "record.schema.json").read_text())
        for value in records:
            obs.validate_schema(record_schema, value)

    def test_benchmark_schema_rejects_malformed_nested_values_by_itself(self):
        envelope = json.loads((FORMAT_ROOT / "examples/benchmark.json").read_text())
        schema = json.loads((FORMAT_ROOT / "benchmark.schema.json").read_text())
        cases = (
            ("manifest", {**envelope["manifest"], "runner": {}}),
            ("records", [{**envelope["records"][0], "client": {}}]),
            ("summary", {**envelope["summary"], "records": {"attempted": "one"}}),
        )
        for field, malformed in cases:
            with self.subTest(field=field):
                candidate = dict(envelope, **{field: malformed})
                with self.assertRaises(ValueError):
                    obs.validate_schema(schema, candidate)

    def test_privacy_rejects_forbidden_fields_paths_and_secrets(self):
        for value in (
            {"prompt": "do not publish"},
            {"Prompt": "do not publish"},
            {"PROMPT_TEXT": "do not publish"},
            {"prompt-text": "do not publish"},
            {"sourceBody": "fn secret() {}"},
            {"rawEvents": []},
            {"stderr": "provider output"},
            {"headers": {"authorization": "Bearer secret"}},
            {"Headers": {"content-type": "text/plain"}},
            {"local_path": "/project/private/repo"},
            {"absolute-path": "/srv/benchmark/output"},
            {"note": "/etc/oko/config"},
            {"note": "/root/.config/oko"},
            {"note": "/app/workspace/result.json"},
            {"note": "Authorization: Bearer abcdefghijklmnop"},
            {"note": "ghp_12345678901234567890"},
        ):
            with self.assertRaises(obs.PrivacyError):
                obs.validate_shareable(value)
        obs.validate_shareable({
            "sourceBodiesRecorded": False,
            "inputTokens": 10,
            "outputTokens": None,
            "cacheReadTokens": None,
            "cacheWriteTokens": None,
            "totalTokens": None,
            "source": {
                "repository": "bartlomein/oko",
                "url": "https://github.com/bartlomein/oko",
            },
        })

    def test_privacy_does_not_treat_urls_or_repository_labels_as_paths(self):
        obs.validate_shareable({
            "repository": "bartlomein/oko",
            "sourceUrl": "https://github.com/bartlomein/oko/tree/main/src",
            "documentation": "https://docs.example.test/project",
        })

    def test_usage_is_null_when_missing_and_total_is_never_inferred(self):
        missing = self.record("missing", duration=100, usage=None)
        self.assertIsNone(missing["agentUsage"])
        explicit = self.record(
            "explicit",
            duration=200,
            usage={"input_tokens": 10, "output_tokens": 4, "total_tokens": 14},
        )
        self.assertEqual(explicit["agentUsage"]["inputTokens"], 10)
        self.assertEqual(explicit["agentUsage"]["outputTokens"], 4)
        self.assertEqual(explicit["agentUsage"]["totalTokens"], 14)
        no_total = self.record("no-total", duration=300, usage={"input_tokens": 10, "output_tokens": 4})
        self.assertIsNone(no_total["agentUsage"]["totalTokens"])

    def test_normalized_multi_step_usage_is_retained_and_aggregated(self):
        steps = [
            {
                "inputTokens": 10,
                "outputTokens": 3,
                "cacheReadTokens": 4,
                "cacheWriteTokens": 1,
                "reasoningTokens": 2,
                "totalTokens": None,
            },
            {
                "inputTokens": 7,
                "outputTokens": 5,
                "cacheReadTokens": 6,
                "cacheWriteTokens": 2,
                "reasoningTokens": 1,
                "totalTokens": None,
            },
        ]
        record = obs.make_record(
            run_id="test-run", record_id="multi-step", task_id="task-multi-step",
            client="opencode", client_version="v", model="m", effort="low", enabled=False,
            task={"kind": "search"}, row={"agentUsageSteps": steps}, total_wall_ns=100,
        )
        self.assertEqual(record["agentUsageSteps"], steps)
        self.assertEqual(record["agentUsage"]["inputTokens"], 17)
        self.assertEqual(record["agentUsage"]["outputTokens"], 8)
        self.assertEqual(record["agentUsage"]["cacheReadTokens"], 10)
        self.assertEqual(record["agentUsage"]["cacheWriteTokens"], 3)
        self.assertEqual(record["agentUsage"]["reasoningTokens"], 3)
        self.assertIsNone(record["agentUsage"]["totalTokens"])
        summary = obs.aggregate([record], run_id="test-run")
        self.assertEqual(summary["groups"][0]["agentUsage"]["inputTokens"], 17)
        self.assertEqual(summary["groups"][0]["agentUsage"]["outputTokens"], 8)
        self.assertEqual(summary["groups"][0]["agentUsage"]["cacheReadTokens"], 10)
        self.assertEqual(summary["groups"][0]["agentUsage"]["cacheWriteTokens"], 3)
        self.assertEqual(summary["groups"][0]["agentUsage"]["reasoningTokens"], 3)
        self.assertIsNone(summary["groups"][0]["agentUsage"]["totalTokens"])

    def test_observed_cache_state_is_copied_without_inference(self):
        missing = self.record("missing-state", duration=100, condition="oko-warm")
        self.assertIsNone(missing["condition"]["observedCacheState"])
        observed = self.record("disk-state", duration=100, condition="oko-warm", observed="disk")
        self.assertEqual(observed["condition"]["observedCacheState"], "disk")

    def test_oko_wall_timing_is_retained_when_reported(self):
        record = self.record(
            "timing",
            duration=100,
            tools=[{"result": {"timings": {"totalWallNs": 4567}}}],
        )
        self.assertEqual(record["timing"]["okoWallNs"], 4567)

    def test_metrics_file_supplies_oko_metadata_absent_from_text_results(self):
        """Oko's tool result is plain source text; serving metadata arrives by file."""
        searches = [
            {"timings": {"totalWallNs": 4567, "totalMs": 5, "cache": {"status": "disk"}},
             "retrieval": {"shortlistedCandidates": 30, "jevCalls": [{
                 "phase": "normal", "durationNs": 9, "requestBytes": 10, "responseBytes": 11,
                 "httpStatus": 200, "success": True, "errorClass": None,
                 "usage": {"input_tokens": 3, "output_tokens": 1}}]},
             "directory": "/private/workspace", "results": [{"text": "SOURCE BODY"}]},
            {"timings": {"totalWallNs": 1, "totalMs": 1, "cache": {"status": "memory"}}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "oko-metrics.jsonl"
            path.write_text("".join(json.dumps(line) + "\n" for line in searches) + "not json\n")
            tools = [
                {"name": "Grep"},
                {"name": "mcp__oko__search", "result": "auth.rs:1-1 (whole file)"},
            ]
            self.assertEqual(obs.attach_oko_metrics(tools, path), 2)
            self.assertEqual(obs.attach_oko_metrics([{"name": "Read"}], path), 2)
            self.assertEqual(obs.attach_oko_metrics(tools, Path(directory) / "absent"), 0)
        self.assertNotIn("okoMetrics", tools[0])
        self.assertEqual(len(tools[1]["okoMetrics"]), 2, "surplus searches are retained")
        record = self.record("metrics-file", duration=100, condition="oko-warm",
                             observed="disk", tools=tools)
        self.assertEqual(record["timing"]["okoWallNs"], 4567)
        self.assertEqual(record["okoUsage"]["providerCalls"][0]["usage"]["inputTokens"], 3)
        self.assertEqual(record["okoUsage"]["phaseMetrics"][0]["cache"]["status"], "disk")
        serialized = json.dumps(record)
        self.assertNotIn("SOURCE BODY", serialized)
        self.assertNotIn("/private/workspace", serialized)

    def test_agent_and_jev_usage_are_separate(self):
        row = {
            "durationNs": 100,
            "toolCalls": 1,
            "okoCalls": 1,
            "usage": {"input_tokens": 5, "output_tokens": 2},
            "tools": [{"result": {"timings": {"totalMs": 77, "cache": {
                "status": "cold", "scanMs": 4, "loadMs": 2, "prepareMs": 6,
                "saveMs": 3, "reusedFiles": 0, "rebuiltFiles": 8,
            }}, "retrieval": {"shortlistedCandidates": 12, "requestBytes": 100,
                "previewMs": 5, "rerankMs": 20, "jevCalls": [{
                "phase": "normal", "durationNs": 9, "requestBytes": 10,
                "responseBytes": 11, "httpStatus": 200, "success": True,
                "errorClass": None, "usage": {"input_tokens": 3, "output_tokens": 1},
            }]}, "stats": {"providerCalls": [{
                "phase": "normal", "durationNs": 9, "requestBytes": 10,
                "responseBytes": 11, "httpStatus": 200, "success": True,
                "errorClass": None, "usage": None,
            }]}}}],
            "grade": {"passed": True},
        }
        record = obs.make_record(
            run_id="test-run", record_id="separate", task_id="task-separate",
            client="codex", client_version="v", model="m", effort="low", enabled=True,
            task={"cacheCondition": "cold"}, row=row, total_wall_ns=100,
        )
        self.assertEqual(record["agentUsage"]["inputTokens"], 5)
        self.assertEqual(record["okoUsage"]["providerCalls"][0]["usage"]["inputTokens"], 3)
        self.assertEqual(record["okoUsage"]["phaseMetrics"][0]["timings"]["totalMs"], 77)
        self.assertEqual(record["okoUsage"]["phaseMetrics"][0]["cache"]["status"], "cold")
        self.assertEqual(record["okoUsage"]["phaseMetrics"][0]["retrieval"]["shortlistedCandidates"], 12)
        self.assertEqual(record["calls"]["jevCalls"], 1)
        summary = obs.aggregate([record], run_id="test-run")
        self.assertEqual(summary["groups"][0]["agentUsage"]["inputTokens"], 5)
        self.assertEqual(summary["groups"][0]["jevUsage"]["inputTokens"], 3)
        self.assertEqual(summary["groups"][0]["jevUsage"]["outputTokens"], 1)
        self.assertIsNone(summary["groups"][0]["jevUsage"]["totalTokens"])

    def test_aggregation_keeps_failures_and_uses_median_p95(self):
        records = [
            self.record("one", duration=100),
            self.record("two", duration=200),
            self.record("three", duration=300),
            self.record("failed", duration=400, status="failed"),
        ]
        summary = obs.aggregate(records, run_id="test-run")
        group = summary["groups"][0]
        self.assertEqual(group["attempted"], 4)
        self.assertEqual(group["succeeded"], 3)
        self.assertEqual(group["failed"], 1)
        self.assertEqual(group["denominator"], 4)
        self.assertEqual(group["latencyNs"]["median"], 200)
        self.assertEqual(group["latencyNs"]["p95"], 300)

    def test_aggregation_separates_model_effort_and_cache_observation(self):
        records = [self.record("one", duration=100)]
        model_variant = self.record("two", duration=200)
        model_variant["client"]["model"] = "other-model"
        effort_variant = self.record("three", duration=300)
        effort_variant["client"]["effort"] = "high"
        cache_variant = self.record("four", duration=400)
        cache_variant["condition"]["observedCacheState"] = "disk"
        edit_variant = self.record("five", duration=500, task_kind="edit")
        summary = obs.aggregate(records + [model_variant, effort_variant, cache_variant, edit_variant], run_id="test-run")
        self.assertEqual(len(summary["groups"]), 5)
        self.assertEqual(
            {
                (
                    group["client"]["model"],
                    group["client"]["effort"],
                    group["condition"]["requested"],
                    group["condition"]["observedCacheState"],
                    group["taskKind"],
                )
                for group in summary["groups"]
            },
            {
                ("test-model", "medium", "oko-cold", None, "search"),
                ("other-model", "medium", "oko-cold", None, "search"),
                ("test-model", "high", "oko-cold", None, "search"),
                ("test-model", "medium", "oko-cold", "disk", "search"),
                ("test-model", "medium", "oko-cold", None, "edit"),
            },
        )

    def test_writer_emits_jsonl_summary_and_report(self):
        manifest = obs.manifest(
            run_id="test-run",
            target={"repository": "fixture", "commit": "a" * 40, "version": None},
            oko={"repository": "bartlomein/oko", "commit": "b" * 40, "version": "0.2.1", "binarySha256": None},
            clients=[{"name": "codex", "version": "v", "model": "m", "effort": "low"}],
        )
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            obs.write_bundle(output, manifest, [self.record("one", duration=100)])
            self.assertEqual(len((output / "records.jsonl").read_text().splitlines()), 1)
            self.assertTrue((output / "run-manifest.json").is_file())
            self.assertTrue((output / "summary.json").is_file())
            self.assertTrue((output / "benchmark.json").is_file())
            self.assertIn("p95", (output / "report.md").read_text())
            summary = json.loads((output / "summary.json").read_text())
            obs.validate_schema(json.loads((FORMAT_ROOT / "summary.schema.json").read_text()), summary)
            envelope = json.loads((output / "benchmark.json").read_text())
            obs.validate_benchmark(envelope)
            self.assertEqual(envelope["formatVersion"], 1)
            self.assertEqual(len(envelope["records"]), 1)
            self.assertIn("report", envelope)

    def test_writer_rejects_schema_invalid_manifest_and_record_before_writing(self):
        manifest = obs.manifest(
            run_id="test-run",
            target={"repository": "fixture", "commit": "a" * 40, "version": None},
            oko={"repository": "bartlomein/oko", "commit": "b" * 40, "version": "0.2.1", "binarySha256": None},
            clients=[{"name": "codex", "version": "v", "model": "m", "effort": "low"}],
        )
        record = self.record("one", duration=100)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "bundle"
            invalid_manifest = dict(manifest, unexpected="not in schema")
            with self.assertRaises(ValueError):
                obs.write_bundle(output, invalid_manifest, [record])
            self.assertFalse(output.exists())

            invalid_record = dict(record, unexpected="not in schema")
            with self.assertRaises(ValueError):
                obs.write_bundle(output, manifest, [invalid_record])
            self.assertFalse(output.exists())

            with patch.object(obs, "aggregate", return_value={"format": obs.FORMAT, "runId": "test-run", "unexpected": True}):
                with self.assertRaises(ValueError):
                    obs.write_bundle(output, manifest, [record])
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
