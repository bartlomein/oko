import assert from "node:assert/strict";
import test from "node:test";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { parseArguments, renderHuman, renderJson, UsageError } from "./cli.js";

test("parseArguments accepts a question and JSON flag in either order", () => {
  assert.deepEqual(parseArguments(["ask", "where", "is", "it?", "--json"]), {
    question: "where is it?",
    json: true,
    noJev: false,
  });
  assert.deepEqual(parseArguments(["ask", "--json", "where is it?"]), {
    question: "where is it?",
    json: true,
    noJev: false,
  });
  assert.deepEqual(parseArguments(["ask", "--no-jev", "where is it?"]), {
    question: "where is it?",
    json: false,
    noJev: true,
  });
});

test("parseArguments rejects missing questions and unknown flags", () => {
  assert.throws(() => parseArguments(["ask"]), UsageError);
  assert.throws(() => parseArguments(["ask", "--nope", "question"]), UsageError);
});

test("rank command requires a JSON file and keeps ask arguments compatible", () => {
  assert.deepEqual(parseArguments(["rank", "--input", "tickets.json", "refund", "--json"]),
    { question: "refund", input: "tickets.json", json: true, noJev: false });
  for (const args of [["rank", "refund"], ["rank", "--input", "--json", "refund"],
    ["ask", "--input", "tickets.json", "refund"], ["rank", "--input", "a", "--input", "b", "q"]]) {
    assert.throws(() => parseArguments(args), UsageError);
  }
});

test("rank CLI reads relative JSON paths without ripgrep and rejects invalid or oversized input", () => {
  const cwd = mkdtempSync(join(tmpdir(), "oko-items-"));
  const cli = fileURLToPath(new URL("./cli.js", import.meta.url));
  const env = { ...process.env, PATH: "", TYPESAFE_API_KEY: "" };
  try {
    writeFileSync(join(cwd, "items.json"), JSON.stringify([{ id: "row-1", text: "Invoice", source: "tickets/1" }]));
    const result = JSON.parse(execFileSync(process.execPath, [cli, "rank", "--input", "items.json", "invoice", "--no-jev", "--json"], { cwd, env, encoding: "utf8" }));
    assert.equal(result.ranking, "input");
    assert.deepEqual(result.results, [{ id: "row-1", text: "Invoice", source: "tickets/1", score: 0 }]);
    for (const content of ["not JSON", "x".repeat(1024 * 1024 + 1)]) {
      writeFileSync(join(cwd, "bad.json"), content);
      const failed = spawnSync(process.execPath, [cli, "rank", "--input", "bad.json", "q", "--no-jev", "--json"], { cwd, env, encoding: "utf8" });
      assert.equal(failed.status, 1);
      assert.equal(failed.stdout, "");
      assert.match(failed.stderr, /valid JSON|1 MiB/);
    }
  } finally { rmSync(cwd, { recursive: true, force: true }); }
});

test("human and JSON output include ranking, paths, ranges, and scores", () => {
  const ranking = {
    method: "lexical" as const,
    results: [
      {
        path: "src/playback.ts",
        startLine: 10,
        endLine: 20,
        score: 13,
        text: "selected gapless playback",
      },
    ],
  };

  const human = renderHuman("where is playback selected?", ranking);
  assert.match(human, /Ranking: lexical/);
  assert.match(human, /src\/playback\.ts:10-20 score=13/);

  const json = JSON.parse(renderJson("where is playback selected?", ranking)) as {
    ranking: string;
    results: Array<{ path: string; startLine: number; endLine: number; score: number }>;
  };
  assert.equal(json.ranking, "lexical");
  assert.deepEqual(json.results[0], {
    path: "src/playback.ts",
    startLine: 10,
    endLine: 20,
    score: 13,
    text: "selected gapless playback",
  });
});

test("lexical-only output visibly identifies the explicit opt-out", () => {
  const human = renderHuman("where is playback selected?", {
    method: "lexical",
    notice: "Lexical-only ranking requested via --no-jev.",
    results: [],
  });

  assert.match(human, /Ranking: lexical-only \(--no-jev\)/);
});
