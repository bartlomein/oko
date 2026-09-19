#!/usr/bin/env python3
"""MCP cache smoke test. Offline by default; --live makes three Jev requests.

Only a synthetic temporary fixture is searched and edited.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import tempfile
import time


spec = importlib.util.spec_from_file_location(
    "cache_profiler", Path(__file__).with_name("profile-cache.py"))
profiler = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profiler)


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def live_key(env_file):
    """Read only a single-line key; never evaluate shell code or print secrets."""
    if os.environ.get("TYPESAFE_API_KEY"):
        return os.environ["TYPESAFE_API_KEY"]
    key = None
    if env_file.exists():
        for line in env_file.read_text().splitlines():
            match = re.match(r"^\s*(?:export\s+)?TYPESAFE_API_KEY\s*=\s*(.*)$", line)
            if match:
                try:
                    values = shlex.split(match[1], comments=True)
                except ValueError:
                    raise RuntimeError("Unsupported key format; use TYPESAFE_API_KEY in the environment") from None
                check(len(values) <= 1, "Expected a single-line API key")
                key = values[0] if values else None
    return key  # Oko can also use its saved OS credential.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path,
                        default=Path(__file__).resolve().parents[1] / "target/release/oko")
    parser.add_argument("--live", action="store_true",
                        help="Send exactly three normal searches of synthetic code to Jev (no retries)")
    parser.add_argument("--env-file", type=Path,
                        default=Path(__file__).resolve().parents[1] / ".env")
    parser.add_argument("--jev-model", default="jev-1.13.0")
    parser.add_argument("--report", type=Path, help="Optional JSON timing report, without credentials or source")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    connection = dict(live=args.live, api_key=live_key(args.env_file) if args.live else None,
                      model=args.jev_model if args.live else None)
    if args.live:
        print("Live test: three Jev requests using synthetic source only; no retries.", flush=True)
    rows = []
    with tempfile.TemporaryDirectory(prefix="oko-cache-smoke-") as directory:
        root = Path(directory) / "project"
        root.mkdir()
        cache = Path(directory) / "cache"
        target = root / "document_search.rs"
        target.write_text("pub fn document_search_debounce() -> u64 { 500 }\n")
        for index in range(80):
            (root / f"unrelated_{index}.rs").write_text(
                f"pub fn unrelated_{index}() -> u64 {{ {index} }}\n")
        # The cache deliberately rereads files with very recent timestamps.
        # Age the fixture before capture; no waits are used after editing.
        time.sleep(2.1)

        def search(client, phase):
            started = time.perf_counter()
            response = client.request("tools/call", {
                "name": "search", "arguments": {
                    "question": "where is document search debounce configured?"}})
            check(not response.get("isError"), f"{phase}: MCP search failed")
            packet = response["structuredContent"]
            check(packet["ranking"] == ("jev" if args.live else "lexical"),
                  "Unexpected ranking mode")
            timings = packet["timings"]["cache"]
            rerank_ms = (packet.get("retrieval") or {}).get("rerankMs", 0)
            rows.append((phase, (time.perf_counter() - started) * 1000, timings, rerank_ms))
            return packet, timings

        def target_text(packet):
            return "\n".join(item["text"] for item in packet["results"]
                             if item["path"] == target.name)

        client = profiler.Client(binary, root, cache, 30, **connection)
        try:
            client.initialize()
            cold, cold_t = search(client, "cold")
            check("500" in target_text(cold), "Cold search missed the fixture")
            check(cold_t["rebuiltFiles"] == 81, "Cold search did not prepare all files")
            warm, warm_t = search(client, "warm")
            # Provider scores may vary. Compare returned target source in live
            # mode; offline mode retains strict packet parity.
            check(target_text(cold) == target_text(warm) if args.live else
                  profiler.semantic_packet(cold) == profiler.semantic_packet(warm),
                  "Unchanged search returned different evidence")
            check(warm_t["rebuiltFiles"] == 0, "Warm search rebuilt unchanged files")
            # Unsupported filesystems may correctly use full content checks.
            if warm_t["validation"] == "incremental" and warm_t["reusedContents"]:
                check(warm_t["readFiles"] == 0 and warm_t["reusedContents"] == 81,
                      "Warm search unexpectedly reread unchanged eligible sources")

            old_stat = target.stat()
            target.write_text("pub fn document_search_debounce() -> u64 { 300 }\n")
            # Same byte length and restored mtime must not conceal the edit.
            os.utime(target, ns=(old_stat.st_atime_ns, old_stat.st_mtime_ns))
            edited, edit_t = search(client, "after edit")
            check("300" in target_text(edited) and "500" not in target_text(edited),
                  "Search served stale content after an immediate edit")
            check(edit_t["rebuiltFiles"] == 1 and edit_t["reusedFiles"] == 80,
                  "Editing one file did not preserve unrelated preparation")

            if not args.live:
                target.unlink()
                deleted, _ = search(client, "after delete")
                check(not target_text(deleted), "Deleted file survived in search results")
        finally:
            client.close()

        if not args.live:
            client = profiler.Client(binary, root, cache, 30)
            try:
                client.initialize()
                restarted, restart_t = search(client, "restart")
                check(profiler.semantic_packet(deleted) == profiler.semantic_packet(restarted),
                      "Restart returned different evidence")
                check(restart_t["readFiles"] == 80 and restart_t["rebuiltFiles"] == 0,
                      "Restart did not validate current files and reuse disk preparation")
            finally:
                client.close()

    print("PASS: cold/warm results and immediate edit freshness." if args.live else
          "PASS: unchanged results, immediate edit, deletion, and restart freshness.")
    print(f"{'Phase':<14} {'Total ms':>9} {'Cache ms':>9} {'Ranking ms':>11} {'Read':>6} {'Reused text':>12} {'Rebuilt':>8}")
    for phase, elapsed, timing, rerank_ms in rows:
        print(f"{phase:<14} {elapsed:9.1f} {timing['totalMs']:9} {rerank_ms:11} {timing['readFiles']:6} "
              f"{timing['reusedContents']:12} {timing['rebuiltFiles']:8}")
    if rows[1][2]['reusedContents'] == 0:
        print("Content-reuse fast path was not exercised; this host used conservative reads.")
    print("Synthetic fixture only; these timings are not a real-project speed benchmark.")
    if args.report:
        args.report.write_text(json.dumps({
            "passed": True, "live": args.live, "jevRequests": 3 if args.live else 0,
            "jevModel": args.jev_model if args.live else None,
            "rows": [{"phase": phase, "totalMs": elapsed, "cache": timing,
                      "rerankMs": rerank_ms} for phase, elapsed, timing, rerank_ms in rows],
        }, indent=2) + "\n")


if __name__ == "__main__":
    main()
