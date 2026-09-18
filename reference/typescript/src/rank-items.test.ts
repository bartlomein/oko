import assert from "node:assert/strict";
import test from "node:test";
import { parseItems, rankItems } from "./rank-items.js";

test("generic items reject duplicate ids, empty text, invalid sources and oversized lists", () => {
  for (const value of [null, {}, [{ id: "x", text: "" }], [{ id: "x", text: "a", source: 1 }],
    [{ id: "x", text: "a" }, { id: "x", text: "b" }], Array(31).fill({ id: "x", text: "a" })]) {
    assert.throws(() => parseItems(value));
  }
  assert.deepEqual(parseItems([{ id: "x", text: "a", privateColumn: "omit" }]), [{ id: "x", text: "a" }]);
});

test("generic reranking preserves external IDs and sources, including reserved-looking IDs", async () => {
  const items = [{ id: "none", text: "Invoice", source: "row/4" }, { id: "candidate_1", text: "Refund" }];
  const result = await rankItems("refund", items, { apiKey: "test", clientFactory: () => ({
    systemOne: async request => {
      assert.deepEqual(request.state.candidates.map(c => c.id), ["none", "candidate_1"]);
      return { answers: { selection: { probabilities: { candidate_1: 0.1, candidate_2: 0.8, none: 0.1 } } } };
    },
  }) });
  assert.deepEqual(result.results, [{ id: "candidate_1", text: "Refund", score: 0.8 }]);
  assert.deepEqual(items[0], { id: "none", text: "Invoice", source: "row/4" });
});

test("input baseline needs no API key and explicitly keeps input order", async () => {
  const result = await rankItems("refund", [{ id: "invoice", text: "Invoice" }, { id: "refund", text: "Refund" }], { noJev: true, limit: 1 });
  assert.equal(result.method, "input");
  assert.equal(result.results[0].id, "invoice");
  assert.equal(result.omittedCount, 0);
  await assert.rejects(rankItems("", [], { noJev: true }), /non-empty/);
  await assert.rejects(rankItems("q", [], { noJev: true, limit: 0 }), /limit/);
});

test("bad provider probabilities fail rather than returning fabricated rankings", async () => {
  for (const probabilities of [{ candidate_1: 0.5 }, { none: 0, candidate_1: NaN }, { none: 0, candidate_1: 2 }]) {
    await assert.rejects(rankItems("q", [{ id: "x", text: "X" }], { apiKey: "test",
      clientFactory: () => ({ systemOne: async () => ({ answers: { selection: { probabilities } } }) }),
    }), /invalid or missing/);
  }
});
