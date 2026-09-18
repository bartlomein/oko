import { pathToFileURL } from "node:url";
import { resolve } from "node:path";
import { config } from "dotenv";
import { open } from "node:fs/promises";
import { rankItems, parseItems, type ItemRanking } from "./rank-items.js";

import { rankLexically, rankWithJev, type RankingResult } from "./rerank.js";
import { searchWorkspace } from "./search.js";

export const USAGE = [
  "Usage: oko ask [--json] [--no-jev] \"question\"",
  "       oko rank --input items.json [--json] [--no-jev] \"question\"",
  "",
  "Search the current working directory for relevant code chunks.",
  "Normal searches require TYPESAFE_API_KEY and Jev reranking.",
  "Use --no-jev for lexical ask results or input-order rank results.",
].join("\n");

export class UsageError extends Error {}

export interface ParsedArguments {
  question: string;
  json: boolean;
  noJev: boolean;
  input?: string;
}

export function parseArguments(args: string[]): ParsedArguments {
  const [command, ...rest] = args;
  if (command !== "ask" && command !== "rank") {
    throw new UsageError(command ? `Unknown command: ${command}` : "A command is required.");
  }

  let json = false;
  let noJev = false;
  const questionParts: string[] = [];
  let input: string | undefined;
  for (let index = 0; index < rest.length; index++) {
    const argument = rest[index];
    if (argument === "--json") {
      json = true;
    } else if (argument === "--no-jev") {
      noJev = true;
    } else if (argument === "--input" && command === "rank") {
      if (input !== undefined || !rest[index + 1] || rest[index + 1].startsWith("-")) {
        throw new UsageError("Provide --input exactly once, followed by a JSON file path.");
      }
      input = rest[++index];
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

  if (command === "rank" && !input) throw new UsageError("oko rank requires --input items.json.");
  return { question, json, noJev, ...(input === undefined ? {} : { input }) };
}

async function readItems(path: string) {
  const handle = await open(path, "r");
  try {
    // A bounded read also protects against the file growing after it is opened.
    const bytes = Buffer.alloc(1024 * 1024 + 1);
    let total = 0;
    while (total < bytes.length) {
      const { bytesRead } = await handle.read(bytes, total, bytes.length - total, null);
      if (!bytesRead) break;
      total += bytesRead;
    }
    if (total === bytes.length) throw new Error("JSON input exceeds the 1 MiB limit.");
    let value: unknown;
    try { value = JSON.parse(bytes.subarray(0, total).toString("utf8")); }
    catch { throw new Error("Input file must contain valid JSON."); }
    return parseItems(value);
  } finally { await handle.close(); }
}

export function renderItems(question: string, ranking: ItemRanking): string {
  const lines = [`Ranking: ${ranking.method === "input" ? "input-order (--no-jev)" : "jev"}`, `Question: ${question}`, ""];
  if (!ranking.results.length) lines.push("No matching items.");
  ranking.results.forEach((item, index) => {
    lines.push(`${index + 1}. ${item.id}${item.source ? ` (${item.source})` : ""} score=${item.score}`,
      humanSnippet(item.text), "");
  });
  return lines.join("\n") + "\n";
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
    if (parsed.input !== undefined) {
      const items = await readItems(resolve(cwd, parsed.input));
      const ranking = await rankItems(parsed.question, items, { apiKey: env.TYPESAFE_API_KEY, noJev: parsed.noJev });
      if (ranking.omittedCount) process.stderr.write(`Notice: ${ranking.omittedCount} trailing items omitted to fit the request size budget.\n`);
      process.stdout.write(parsed.json
        ? JSON.stringify({ question: parsed.question, ranking: ranking.method, results: ranking.results, omittedCount: ranking.omittedCount }, null, 2) + "\n"
        : renderItems(parsed.question, ranking));
      return 0;
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
