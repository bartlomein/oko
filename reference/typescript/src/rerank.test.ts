import assert from "node:assert/strict";
import test from "node:test";

import { MAX_JEV_REQUEST_BYTES, rankLexically, rankWithJev } from "./rerank.js";
import type { Chunk } from "./search.js";

const shortlist: Chunk[] = [
  {
    path: "src/first.ts",
    startLine: 1,
    endLine: 3,
    text: "first candidate",
    lexicalScore: 10,
  },
  {
    path: "src/second.ts",
    startLine: 4,
    endLine: 6,
    text: "second candidate",
    lexicalScore: 5,
  },
];

test("Jev receives one request with candidates plus an explicit none choice", async () => {
  const requests: unknown[] = [];
  const result = await rankWithJev(
    "Which candidate matters?",
    shortlist,
    "test-key",
    () => ({
      systemOne: async (request: unknown) => {
        requests.push(request);
        return {
          answers: {
            selection: {
              probabilities: {
                candidate_1: 0.2,
                candidate_2: 0.8,
                none: 0.1,
              },
            },
          },
        };
      },
    }),
  );

  assert.equal(requests.length, 1);
  assert.equal(result.method, "jev");
  assert.equal(result.results[0]?.path, "src/second.ts");
  assert.equal(result.results[0]?.score, 0.8);

  const request = requests[0] as {
    state: { question: string; candidates: Array<{ source: string; text: string }> };
    questions: { selection: { criteria: Record<string, unknown> } };
  };
  assert.equal(request.state.question, "Which candidate matters?");
  assert.deepEqual(
    request.state.candidates.map(({ source, text }) => [source, text]),
    [
      ["src/first.ts:1-3", "first candidate"],
      ["src/second.ts:4-6", "second candidate"],
    ],
  );
  assert.deepEqual(Object.keys(request.questions.selection.criteria), [
    "candidate_1",
    "candidate_2",
    "none",
  ]);
  assert.equal(JSON.stringify(request).split("first candidate").length - 1, 1);
});

test("Jev payload stays bounded, retains full chunks and maps only sent candidates", async () => {
  const large = Array.from({ length: 30 }, (_, index) => ({
    ...shortlist[0], path: `src/${index}.rs`, text: `// ${index}\n` + "界".repeat(1500),
  }));
  const result = await rankWithJev("find code", large, "test-key", () => ({
    systemOne: async (request) => {
      assert.ok(Buffer.byteLength(JSON.stringify(request), "utf8") <= MAX_JEV_REQUEST_BYTES);
      const sent = request.state.candidates;
      assert.ok(sent.length > 0 && sent.length < large.length);
      sent.forEach((c: { text: string }, i: number) => assert.equal(c.text, large[i].text));
      assert.equal(Object.keys(request.questions.selection.criteria).length, sent.length + 1);
      return { answers: { selection: { probabilities: { ...Object.fromEntries(sent.map((_: unknown, i: number) => [`candidate_${i + 1}`, i === 0 ? 0.8 : 0])), candidate_30: 1, none: 0.1 } } } };
    },
  }));
  assert.deepEqual(result.results.map(c => c.path), ["src/0.rs"]);
  assert.equal(large.length, 30);
});

test("an oversized first chunk fails before spending an API request", async () => {
  await assert.rejects(rankWithJev("find", [{ ...shortlist[0], text: "x".repeat(MAX_JEV_REQUEST_BYTES) }],
    "test-key", () => { throw new Error("must not create client"); }), /exceed the Jev request size budget/);
});

test("a missing API key fails normal Jev ranking clearly", async () => {
  await assert.rejects(
    rankWithJev("Which candidate matters?", shortlist, undefined),
    /TYPESAFE_API_KEY is required.*--no-jev/,
  );
});

test("a Jev failure fails instead of silently using lexical results", async () => {
  await assert.rejects(
    rankWithJev(
      "Which candidate matters?",
      shortlist,
      "test-key",
      () => ({
        systemOne: async () => {
          throw new Error("network unavailable");
        },
      }),
    ),
    /Jev ranking failed: network unavailable/,
  );
});

test("lexical-only ranking is explicitly marked for local benchmarking", () => {
  const result = rankLexically(shortlist);

  assert.equal(result.method, "lexical");
  assert.equal(result.notice, "Lexical-only ranking requested via --no-jev.");
  assert.equal(result.results[0]?.path, "src/first.ts");
  assert.equal(result.results[0]?.score, 10);
});

test("none wins when no candidate probability beats it", async () => {
  const result = await rankWithJev(
    "Which candidate matters?",
    shortlist,
    "test-key",
    () => ({
      systemOne: async () => ({
        answers: {
          selection: {
            probabilities: {
              candidate_1: 0.2,
              candidate_2: 0.1,
              none: 0.7,
            },
          },
        },
      }),
    }),
  );

  assert.equal(result.method, "jev");
  assert.deepEqual(result.results, []);
});
