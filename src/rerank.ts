import { choice, TypeSafeClient } from "@typesafe-ai/sdk";

import { compareRankedChunks, RESULT_LIMIT, type Chunk } from "./search.js";

export type RankingMethod = "lexical" | "jev" | "fallback";

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
  warning?: string;
}

interface JevClientLike {
  systemOne(request: any): Promise<any>;
}

type JevClientFactory = (apiKey: string) => JevClientLike;

function createJevClient(apiKey: string): JevClientLike {
  return new TypeSafeClient({
    apiKey,
    retry: { maxRetries: 0 },
  });
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

function candidateId(index: number): string {
  return `candidate_${index + 1}`;
}

function isProbability(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export async function rankWithJev(
  question: string,
  shortlist: Chunk[],
  apiKey: string | undefined,
  clientFactory: JevClientFactory = createJevClient,
): Promise<RankingResult> {
  if (!apiKey || shortlist.length === 0) {
    return { method: "lexical", results: lexicalResults(shortlist) };
  }

  try {
    const client = clientFactory(apiKey);
    const criteria = Object.fromEntries([
      ...shortlist.map((chunk, index) => [candidateId(index), chunk.text]),
      ["none", "None of the code chunks answers the question."],
    ]);
    const response = await client.systemOne({
      state: {
        question,
        candidates: shortlist.map((chunk, index) => ({
          id: candidateId(index),
          path: chunk.path,
          startLine: chunk.startLine,
          endLine: chunk.endLine,
          text: chunk.text,
        })),
      },
      questions: {
        selection: choice(
          "Which code chunk best answers the question? Choose none when no candidate is sufficient.",
          criteria,
        ),
      },
    });

    const probabilities = response?.answers?.selection?.probabilities;
    if (!probabilities || typeof probabilities !== "object") {
      throw new Error("Jev returned no candidate probabilities.");
    }

    const noneScore = isProbability(probabilities.none) ? probabilities.none : 0;
    const ranked = shortlist
      .map((chunk, index) => ({
        chunk,
        score: isProbability(probabilities[candidateId(index)])
          ? probabilities[candidateId(index)]
          : 0,
      }))
      .filter(({ score }) => score > noneScore)
      .sort(
        (left, right) =>
          right.score - left.score || compareRankedChunks(left.chunk, right.chunk),
      )
      .slice(0, RESULT_LIMIT)
      .map(({ chunk, score }) => toResult(chunk, score));

    return { method: "jev", results: ranked };
  } catch {
    return {
      method: "fallback",
      results: lexicalResults(shortlist),
      warning: "Jev ranking failed; using lexical fallback.",
    };
  }
}
