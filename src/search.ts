import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";

export const MAX_FILE_BYTES = 256 * 1024;
export const CHUNK_LINES = 40;
export const CHUNK_OVERLAP = 5;
export const SHORTLIST_LIMIT = 30;
export const RESULT_LIMIT = 5;

const STOP_WORDS = new Set([
  "a",
  "an",
  "and",
  "are",
  "do",
  "for",
  "how",
  "in",
  "is",
  "of",
  "on",
  "or",
  "the",
  "to",
  "what",
  "when",
  "where",
  "which",
  "who",
  "why",
]);

export interface Chunk {
  path: string;
  startLine: number;
  endLine: number;
  text: string;
  lexicalScore: number;
}

function tokenize(value: string): string[] {
  return value.toLowerCase().match(/[a-z0-9_]+/g) ?? [];
}

function compareText(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function compareChunks(left: Chunk, right: Chunk): number {
  return (
    right.lexicalScore - left.lexicalScore ||
    compareText(left.path, right.path) ||
    left.startLine - right.startLine ||
    left.endLine - right.endLine
  );
}

function scoreChunk(chunk: Chunk, question: string): number {
  const terms = [...new Set(tokenize(question))].filter((term) => !STOP_WORDS.has(term));
  const contentTerms = new Set(tokenize(chunk.text));
  const pathTerms = new Set(tokenize(chunk.path));

  return terms.reduce((score, term) => {
    return score + (contentTerms.has(term) ? 10 : 0) + (pathTerms.has(term) ? 3 : 0);
  }, 0);
}

export function chunkText(relativePath: string, text: string): Chunk[] {
  const lines = text.split(/\r\n|\n|\r/);
  if (lines.at(-1) === "") {
    lines.pop();
  }

  const chunks: Chunk[] = [];
  const stride = Math.max(1, CHUNK_LINES - CHUNK_OVERLAP);

  for (let start = 0; start < lines.length; start += stride) {
    const end = Math.min(lines.length, start + CHUNK_LINES);
    chunks.push({
      path: relativePath,
      startLine: start + 1,
      endLine: end,
      text: lines.slice(start, end).join("\n"),
      lexicalScore: 0,
    });

    if (end === lines.length) {
      break;
    }
  }

  return chunks;
}

export function rankLexically(chunks: Chunk[], question: string): Chunk[] {
  const ranked = chunks
    .map((chunk) => ({ ...chunk, lexicalScore: scoreChunk(chunk, question) }))
    .sort(compareChunks);
  return ranked.filter((chunk) => chunk.lexicalScore > 0).slice(0, SHORTLIST_LIMIT);
}

async function discoverFileNames(cwd: string): Promise<string[]> {
  return new Promise((resolve, reject) => {
    const child = spawn("rg", ["--files"], {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    child.on("error", (error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") {
        reject(new Error("Could not run `rg --files`; install ripgrep and try again."));
        return;
      }
      reject(error);
    });
    child.on("close", (code) => {
      if (code !== 0 && code !== 1) {
        reject(new Error(`File discovery failed: ${stderr.trim() || `rg exited with code ${code}`}`));
        return;
      }

      const files = stdout
        .split(/\r?\n/)
        .filter(Boolean)
        .map((file) => file.replaceAll("\\", "/"))
        .sort(compareText);
      resolve([...new Set(files)]);
    });
  });
}

async function readBoundedTextFile(cwd: string, relativePath: string): Promise<string | null> {
  const absolutePath = path.resolve(cwd, relativePath);
  const stats = await fs.stat(absolutePath);
  if (stats.size > MAX_FILE_BYTES) {
    return null;
  }

  const bytes = await fs.readFile(absolutePath);
  if (bytes.length > MAX_FILE_BYTES || bytes.includes(0)) {
    return null;
  }

  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

export async function searchWorkspace(cwd: string, question: string): Promise<Chunk[]> {
  const files = await discoverFileNames(cwd);
  const chunks: Chunk[] = [];

  for (const relativePath of files) {
    try {
      const text = await readBoundedTextFile(cwd, relativePath);
      if (text === null || text.trim() === "") {
        continue;
      }
      chunks.push(...chunkText(relativePath, text));
    } catch {
      // Files can disappear or become unreadable while a search is running.
      // Skipping them keeps one bad file from aborting the whole search.
    }
  }

  return rankLexically(chunks, question);
}

export function compareRankedChunks(left: Chunk, right: Chunk): number {
  return compareChunks(left, right);
}
