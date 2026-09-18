import { rankItems, type JevClientLike } from "./rank-items.js";
export { MAX_JEV_REQUEST_BYTES } from "./rank-items.js";

import { compareRankedChunks, RESULT_LIMIT, type Chunk } from "./search.js";

export type RankingMethod = "lexical" | "jev";

export interface SearchResult {
  path: string;
  startLine: number;
  endLine: number;
  score: number;
  text: string;
}

export interface RankingResult {
  method: RankingMethod;
  results: SearchResult[];
  notice?: string;
}

function toResult(chunk: Chunk, score: number): SearchResult {
  return {
    path: chunk.path,
    startLine: chunk.startLine,
    endLine: chunk.endLine,
    score,
    text: chunk.text,
  };
}

function lexicalResults(shortlist: Chunk[]): SearchResult[] {
  return shortlist.slice(0, RESULT_LIMIT).map((chunk) => toResult(chunk, chunk.lexicalScore));
}

export function rankLexically(shortlist: Chunk[]): RankingResult {
  return {
    method: "lexical",
    results: lexicalResults(shortlist),
    notice: "Lexical-only ranking requested via --no-jev.",
  };
}

export async function rankWithJev(
  question: string,
  shortlist: Chunk[],
  apiKey: string | undefined,
  clientFactory?: (apiKey: string) => JevClientLike,
): Promise<RankingResult> {
  if (!apiKey) {
    throw new Error("TYPESAFE_API_KEY is required for normal `oko ask`; use `--no-jev` for explicit lexical-only benchmarking.");
  }
  const chunks = [...shortlist];
  const ranked = await rankItems(question, chunks.map((chunk, index) => ({
    id: String(index), text: chunk.text,
    source: `${chunk.path}:${chunk.startLine}-${chunk.endLine}`,
  })), { apiKey, clientFactory, limit: 30 });
  const results = ranked.results
    .map(item => ({ chunk: chunks[Number(item.id)], score: item.score }))
    .sort((a, b) => b.score - a.score || compareRankedChunks(a.chunk, b.chunk))
    .slice(0, RESULT_LIMIT).map(({ chunk, score }) => toResult(chunk, score));
  return { method: "jev", results };
}
