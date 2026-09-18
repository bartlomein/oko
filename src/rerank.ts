import { choice, TypeSafeClient } from "@typesafe-ai/sdk";

import { compareRankedChunks, RESULT_LIMIT, type Chunk } from "./search.js";

// Conservative payload cap, not an exact model-token count. Preserve complete
// chunks and drop the lowest lexical candidates when context grows too large.
export const MAX_JEV_REQUEST_BYTES = 32_000;

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

export function rankLexically(shortlist: Chunk[]): RankingResult {
  return {
    method: "lexical",
    results: lexicalResults(shortlist),
    notice: "Lexical-only ranking requested via --no-jev.",
  };
}

function candidateId(index: number): string {
  return `candidate_${index + 1}`;
}

function isProbability(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function createRequest(question: string, candidates: Chunk[]) {
  return {
    state: {
      question,
      candidates: candidates.map((chunk, index) => ({
        id: candidateId(index), path: chunk.path,
        startLine: chunk.startLine, endLine: chunk.endLine, text: chunk.text,
      })),
    },
    questions: {
      selection: choice(
        "Which code chunk best answers the question? Choose none when no candidate is sufficient.",
        Object.fromEntries([
          ...candidates.map((chunk, index) => [candidateId(index),
            `The code in state candidate ${candidateId(index)} at ${chunk.path}:${chunk.startLine}-${chunk.endLine}.`]),
          ["none", "None of the code chunks answers the question."],
        ]),
      ),
    },
  };
}

export async function rankWithJev(
  question: string,
  shortlist: Chunk[],
  apiKey: string | undefined,
  clientFactory: JevClientFactory = createJevClient,
): Promise<RankingResult> {
  if (!apiKey) {
    throw new Error(
      "TYPESAFE_API_KEY is required for normal `oko ask`; use `--no-jev` for explicit lexical-only benchmarking.",
    );
  }

  if (shortlist.length === 0) {
    return { method: "jev", results: [] };
  }

  try {
    const candidates = [...shortlist];
    let request = createRequest(question, candidates);
    while (candidates.length && Buffer.byteLength(JSON.stringify(request), "utf8") > MAX_JEV_REQUEST_BYTES) {
      candidates.pop();
      request = createRequest(question, candidates);
    }
    if (candidates.length === 0) {
      throw new Error("Question and first code chunk exceed the Jev request size budget.");
    }
    const client = clientFactory(apiKey);
    const response = await client.systemOne(request);

    const probabilities = response?.answers?.selection?.probabilities;
    if (!probabilities || typeof probabilities !== "object") {
      throw new Error("Jev returned no candidate probabilities.");
    }

    const noneScore = isProbability(probabilities.none) ? probabilities.none : 0;
    const ranked = candidates
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
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`Jev ranking failed: ${message}`, { cause: error });
  }
}
