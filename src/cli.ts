import { pathToFileURL } from "node:url";
import { resolve } from "node:path";
import { config } from "dotenv";

import { rankLexically, rankWithJev, type RankingResult } from "./rerank.js";
import { searchWorkspace } from "./search.js";

export const USAGE = [
  "Usage: oko ask [--json] [--no-jev] \"question\"",
  "",
  "Search the current working directory for relevant code chunks.",
  "Normal searches require TYPESAFE_API_KEY and Jev reranking.",
  "Use --no-jev for explicit local lexical-only benchmarking.",
].join("\n");

export class UsageError extends Error {}

export interface ParsedArguments {
  question: string;
  json: boolean;
  noJev: boolean;
}

export function parseArguments(args: string[]): ParsedArguments {
  const [command, ...rest] = args;
  if (command !== "ask") {
    throw new UsageError(command ? `Unknown command: ${command}` : "A command is required.");
  }

  let json = false;
  let noJev = false;
  const questionParts: string[] = [];
  for (const argument of rest) {
    if (argument === "--json") {
      json = true;
    } else if (argument === "--no-jev") {
      noJev = true;
    } else if (argument.startsWith("-")) {
      throw new UsageError(`Unknown flag: ${argument}`);
    } else {
      questionParts.push(argument);
    }
  }

  const question = questionParts.join(" ").trim();
  if (!question) {
    throw new UsageError("A non-empty question is required.");
  }

  return { question, json, noJev };
}

function humanSnippet(text: string): string {
  const lines = text.split("\n");
  const excerpt = lines.slice(0, 8).join("\n");
  return lines.length > 8 ? `${excerpt}\n…` : excerpt;
}

export function renderHuman(question: string, ranking: RankingResult): string {
  const rankingLabel = ranking.notice ? "lexical-only (--no-jev)" : ranking.method;
  const lines = [`Ranking: ${rankingLabel}`, `Question: ${question}`, ""];

  if (ranking.results.length === 0) {
    lines.push("No matching chunks.");
    return `${lines.join("\n")}\n`;
  }

  ranking.results.forEach((result, index) => {
    lines.push(
      `${index + 1}. ${result.path}:${result.startLine}-${result.endLine} score=${result.score}`,
    );
    lines.push(
      humanSnippet(result.text)
        .split("\n")
        .map((line) => `   ${line}`)
        .join("\n"),
    );
    lines.push("");
  });

  return `${lines.join("\n")}\n`;
}

export function renderJson(question: string, ranking: RankingResult): string {
  return `${JSON.stringify(
    {
      question,
      ranking: ranking.method,
      notice: ranking.notice,
      results: ranking.results,
    },
    null,
    2,
  )}\n`;
}

export async function run(args: string[], cwd: string): Promise<number> {
  let parsed: ParsedArguments;
  try {
    parsed = parseArguments(args);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`Error: ${message}\n\n${USAGE}\n`);
    return 1;
  }

  try {
    const env = { ...process.env };
    const loaded = config({ path: resolve(cwd, ".env"), quiet: true, processEnv: env });
    if (loaded.error && (loaded.error as NodeJS.ErrnoException).code !== "ENOENT") {
      throw loaded.error;
    }
    const shortlist = await searchWorkspace(cwd, parsed.question);
    const ranking = parsed.noJev
      ? rankLexically(shortlist)
      : await rankWithJev(
          parsed.question,
          shortlist,
          env.TYPESAFE_API_KEY?.trim() || undefined,
        );
    if (ranking.notice) {
      process.stderr.write(`Notice: ${ranking.notice}\n`);
    }
    process.stdout.write(
      parsed.json
        ? renderJson(parsed.question, ranking)
        : renderHuman(parsed.question, ranking),
    );
    return 0;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`Error: ${message}\n`);
    return 1;
  }
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  run(process.argv.slice(2), process.cwd()).then((code) => {
    process.exitCode = code;
  });
}
