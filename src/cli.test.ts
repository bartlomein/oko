import assert from "node:assert/strict";
import test from "node:test";

import { parseArguments, renderHuman, renderJson, UsageError } from "./cli.js";

test("parseArguments accepts a question and JSON flag in either order", () => {
  assert.deepEqual(parseArguments(["ask", "where", "is", "it?", "--json"]), {
    question: "where is it?",
    json: true,
  });
  assert.deepEqual(parseArguments(["ask", "--json", "where is it?"]), {
    question: "where is it?",
    json: true,
  });
});

test("parseArguments rejects missing questions and unknown flags", () => {
  assert.throws(() => parseArguments(["ask"]), UsageError);
  assert.throws(() => parseArguments(["ask", "--nope", "question"]), UsageError);
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
