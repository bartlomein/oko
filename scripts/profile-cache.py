#!/usr/bin/env python3
"""Measure offline MCP cold, warm and restarted-cache searches without editing sources.

Example: python3 scripts/profile-cache.py --root /path/to/project > /tmp/cache.json
The temporary cache is removed on exit. No Jev or other model requests are made.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import queue
import statistics
import subprocess
import tempfile
import threading
import time


class Client:
    def __init__(self, binary, root, cache, timeout, *, live=False, api_key=None, model=None,
                 command=None, metrics=None):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("TYPESAFE_", "OKO_"))}
        # Oko's tool result is agent-facing text. Timings, retrieval metadata and
        # the structured packet are appended here, one JSON line per search. A
        # launcher passed as `command` chooses its own file; name it in `metrics`.
        self.metrics = Path(metrics or Path(cache) / "oko-metrics.jsonl")
        env.update(OKO_CACHE_DIR=str(cache), OKO_NO_CACHE="0", TYPESAFE_API_KEY="",
                   OKO_METRICS_FILE=str(self.metrics))
        if live:
            env.pop("TYPESAFE_API_KEY")
            if api_key:
                env["TYPESAFE_API_KEY"] = api_key
            if model:
                env["TYPESAFE_DEFAULT_MODEL"] = model
        self.process = subprocess.Popen(
            command or [str(binary), "mcp", *([] if live else ["--no-jev"]), "--root", str(root)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8", env=env,
        )
        self.responses = queue.Queue()
        self.timeout = timeout
        self.request_id = 0
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def last_metrics(self):
        """Metadata and structured packet of the most recent completed search."""
        try:
            lines = self.metrics.read_text(encoding="utf-8").splitlines()
        except FileNotFoundError:
            lines = []
        if not lines:
            raise RuntimeError("Oko recorded no search metrics")
        return json.loads(lines[-1])

    def read(self):
        try:
            for line in self.process.stdout:
                self.responses.put(json.loads(line))
        except Exception as error:
            self.responses.put(error)
        finally:
            self.responses.put(EOFError("MCP server closed stdout"))

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def request(self, method, params):
        self.request_id += 1
        self.send({"jsonrpc": "2.0", "id": self.request_id,
                   "method": method, "params": params})
        deadline = time.monotonic() + self.timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"MCP {method} timed out")
            try:
                response = self.responses.get(timeout=remaining)
            except queue.Empty:
                raise TimeoutError(f"MCP {method} timed out") from None
            if isinstance(response, Exception):
                raise response
            if response.get("id") == self.request_id:
                if "error" in response:
                    raise RuntimeError(f"MCP {method} returned a protocol error")
                return response["result"]

    def initialize(self):
        self.request("initialize", {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "oko-cache-profiler", "version": "1"},
        })
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def close(self):
        try:
            self.process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.reader.join(timeout=5)
        self.process.stdout.close()


def semantic_packet(value):
    """Ignore timing observations while retaining every other packet field."""
    if isinstance(value, dict):
        return {key: semantic_packet(item) for key, item in value.items()
                if key != "timings" and not key.endswith("Ms")}
    if isinstance(value, list):
        return [semantic_packet(item) for item in value]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/oko"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--question", default="where is authentication handled?")
    parser.add_argument("--repeats", type=int, default=5,
                        help="warm requests per server process (default: 5)")
    parser.add_argument("--timeout", type=float, default=120,
                        help="timeout per MCP request in seconds")
    args = parser.parse_args()
    if not 1 <= args.repeats <= 100 or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("repeats must be 1–100 and timeout must be positive")
    binary, root = args.binary.resolve(strict=True), args.root.resolve(strict=True)
    if not root.is_dir():
        parser.error("root must be a directory")
    rows, startup, reference = [], [], None
    with tempfile.TemporaryDirectory(prefix="oko-cache-profile-") as directory:
        cache = Path(directory).resolve()
        if cache.is_relative_to(root):
            parser.error("temporary cache would be inside root; set TMPDIR outside root")
        for session, first_phase in enumerate(("cold", "disk")):
            started = time.perf_counter()
            client = Client(binary, root, cache, args.timeout)
            try:
                client.initialize()
                startup.append(round((time.perf_counter() - started) * 1000, 3))
                for index in range(args.repeats + 1):
                    started = time.perf_counter()
                    result = client.request("tools/call", {
                        "name": "search", "arguments": {"question": args.question},
                    })
                    wall_ms = round((time.perf_counter() - started) * 1000, 3)
                    if result.get("isError"):
                        raise RuntimeError("MCP search failed")
                    packet = client.last_metrics()
                    canonical = json.dumps(semantic_packet(packet), sort_keys=True,
                                           separators=(",", ":"))
                    if reference is None:
                        reference = canonical
                    if canonical != reference:
                        raise RuntimeError("Search results changed between cache states")
                    rows.append({"phase": first_phase if index == 0 else "warm",
                                 "session": session + 1, "request": index + 1,
                                 "wallMs": wall_ms, "timings": packet["timings"]})
            finally:
                client.close()
    report = {
        "binary": str(binary), "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "root": str(root), "question": args.question, "offline": True,
        "semanticParity": True, "semanticSha256": hashlib.sha256(reference.encode()).hexdigest(),
        "startupMs": startup, "rows": rows,
        "medianWallMs": {phase: statistics.median(row["wallMs"] for row in rows
                                                  if row["phase"] == phase)
                         for phase in ("cold", "disk", "warm")},
        "note": "Cold refers to Oko's cache, not the OS page cache. Searches exclude MCP startup. "
                "The workspace must remain unchanged during this measurement.",
    }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
