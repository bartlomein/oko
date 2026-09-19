#!/usr/bin/env python3
"""Exercise a release archive without Rust, Node, rg on PATH, or real credentials."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading


def run(binary, args, cwd, env):
    result = subprocess.run([str(binary), *args], cwd=cwd, env=env, text=True,
                            capture_output=True, timeout=30)
    if result.returncode:
        raise RuntimeError(f"{args[0]} failed: {result.stderr}")
    return result.stdout


def mcp_search(binary, root, env):
    with tempfile.TemporaryFile() as errors:
        child = subprocess.Popen([str(binary), "mcp", "--root", str(root), "--no-jev"],
                                 cwd=root, env=env, stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=errors, text=True)
        messages = queue.Queue()
        def read():
            for line in child.stdout:
                messages.put(json.loads(line))
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        def send(value):
            child.stdin.write(json.dumps({"jsonrpc": "2.0", **value}) + "\n")
            child.stdin.flush()
        def request(identifier, method, params):
            send({"id": identifier, "method": method, "params": params})
            while True:
                value = messages.get(timeout=15)
                if value.get("id") == identifier:
                    assert "error" not in value, value
                    return value["result"]
        try:
            request(1, "initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                       "clientInfo": {"name": "release-smoke", "version": "1"}})
            send({"method": "notifications/initialized"})
            tools = request(2, "tools/list", {})
            assert any(t["name"] == "search" for t in tools["tools"])
            answer = request(3, "tools/call", {"name": "search", "arguments": {
                "question": "where is archive checksum verification implemented?"}})
            assert not answer.get("isError"), answer
            packet = answer.get("structuredContent") or json.loads(answer["content"][0]["text"])
            assert packet["results"][0]["path"] == "archive.rs", packet
        finally:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            reader.join(timeout=5)


def smoke(archive):
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    expected = archive.with_name(archive.name + ".sha256").read_text().split()[0]
    assert digest == expected, "Archive checksum mismatch"
    with tempfile.TemporaryDirectory(prefix="oko release smoke ") as work:
        work = Path(work).resolve()
        download = work / "download"
        download.mkdir()
        with tarfile.open(archive) as bundle:
            for member in bundle.getmembers():
                path = download / member.name
                assert path.resolve().is_relative_to(download), "Unsafe archive path"
                if member.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    assert member.isfile(), "Archive contains a link or special file"
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(bundle.extractfile(member).read())
                    path.chmod(member.mode & 0o777)
        folders = list(download.iterdir())
        assert len(folders) == 1 and folders[0].is_dir()
        extracted = folders[0]
        build = json.loads((extracted / "BUILD.json").read_text())
        root = work / "project with spaces"
        root.mkdir()
        (root / "archive.rs").write_text("pub fn verify_archive_checksum() { compare_checksum(); }\n")
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("TYPESAFE_", "OKO_", "RIPGREP_"))}
        env.update(PATH="", OKO_CACHE_DIR=str(work / "cache"), TYPESAFE_API_KEY="",
                   TYPESAFE_BASE_URL="http://127.0.0.1:1")
        binary = extracted / "oko"
        assert run(binary, ["--version"], root, env).strip() == f"oko {build['version']}"
        direct = json.loads(run(binary, ["ask", "archive checksum", "--no-jev", "--json"], root, env))
        assert direct["results"][0]["path"] == "archive.rs"
        # An explicit override must not silently fall back to the bundled rg.
        invalid_override = {**env, "OKO_RIPGREP": str(work / "missing-rg")}
        rejected = subprocess.run([str(binary), "ask", "archive checksum", "--no-jev"],
                                  cwd=root, env=invalid_override, capture_output=True, timeout=15)
        assert rejected.returncode != 0, "Invalid explicit ripgrep override was ignored"
        install = work / "installed bin"
        run(binary, ["setup", "--root", str(root), "--install-dir", str(install), "--no-jev"], root, env)
        original = (root / ".codex/config.toml").read_bytes()
        shutil.rmtree(download)
        binary = install / "oko"
        run(binary, ["setup", "--root", str(root), "--install-dir", str(install), "--no-jev"], root, env)
        assert original == (root / ".codex/config.toml").read_bytes()
        assert str(download) not in original.decode()
        mcp_search(binary, root, env)

        # Exercise key loading and ranked search with a fake key and loopback provider.
        calls = []
        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                assert self.headers["Authorization"] == "Bearer release-smoke-fake-key"
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                calls.append(body)
                response = json.dumps({"answers": {
                    c["candidate"]: {"type": "noul", "noul": 0.95}
                    for c in body["state"]["candidates"]}}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(response)))
                self.end_headers()
                self.wfile.write(response)
        server = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            env.pop("TYPESAFE_API_KEY")
            env["TYPESAFE_BASE_URL"] = f"http://127.0.0.1:{server.server_port}"
            (root / ".env").write_text("TYPESAFE_API_KEY=release-smoke-fake-key\n")
            ranked = json.loads(run(binary, ["ask", "archive checksum", "--json"], root, env))
            assert ranked["ranking"] == "jev" and ranked["results"][0]["path"] == "archive.rs"
            assert len(calls) == 1
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
    print(f"PASS {archive.name}: checksum, bundled rg, setup, reinstall, MCP, fake-key ranking")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python3 scripts/smoke-release.py ARCHIVE.tar.gz")
    smoke(Path(sys.argv[1]).resolve())
