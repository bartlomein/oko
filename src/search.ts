import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";
import { stemmer } from "stemmer";

export const MAX_FILE_BYTES = 256 * 1024;
export const CHUNK_LINES = 40;
export const CHUNK_OVERLAP = 5;
export const FUNCTION_CHUNK_LINES = 120;
export const SHORTLIST_LIMIT = 30;
export const RESULT_LIMIT = 5;

const STOP_WORDS = new Set([
  "a",
  "an",
  "and",
  "are",
  "by",
  "do",
  "does",
  "for",
  "how",
  "in",
  "is",
  "it",
  "its",
  "of",
  "on",
  "or",
  "the",
  "this",
  "that",
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
  return value
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .toLowerCase()
    .match(/[a-z0-9]+/g) ?? [];
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

export function chunkText(relativePath: string, text: string): Chunk[] {
  const lines = text.split(/\r\n|\n|\r/);
  if (lines.at(-1) === "") {
    lines.pop();
  }

  // Heuristic declaration boundaries, not an AST parser. Unknown syntax keeps
  // the fixed-window fallback; long declaration sections remain bounded.
  const declarations: number[] = [];
  if (/\.(?:rs|[cm]?js|jsx|ts|tsx|py)$/.test(relativePath)) {
    for (let index = 0; index < lines.length; index++) {
      if (!/^\s*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default)\s+)*(?:fn|function\*?|def)\s+[\w]+/.test(lines[index])) continue;
      let start = index;
      while (start > 0 && /^\s*(?:\/\/|#|\/\*\*|\*)/.test(lines[start - 1])) start--;
      declarations.push(start);
    }
  }

  const chunks: Chunk[] = [];
  const boundaries = [...new Set([0, ...declarations, lines.length])];
  const declarationStarts = new Set(declarations);
  for (let section = 0; section < boundaries.length - 1; section++) {
    const sectionStart = boundaries[section];
    const sectionEnd = boundaries[section + 1];
    const limit = declarationStarts.has(sectionStart) ? FUNCTION_CHUNK_LINES : CHUNK_LINES;
    for (let start = sectionStart; start < sectionEnd; start += limit - CHUNK_OVERLAP) {
      const end = Math.min(sectionEnd, start + limit);
      chunks.push({ path: relativePath, startLine: start + 1, endLine: end,
        text: lines.slice(start, end).join("\n"), lexicalScore: 0 });
      if (end === sectionEnd) break;
    }
  }

  return chunks;
}

export function rankLexically(chunks: Chunk[], question: string): Chunk[] {
  // Cache stems only for this search, so repeated code words are cheap without
  // accumulating a process-wide cache as workspaces change.
  const stems = new Map<string, string>();
  const normalize = (term: string): string => {
    let normalized = stems.get(term);
    if (normalized === undefined) {
      normalized = stemmer(term);
      stems.set(term, normalized);
    }
    return normalized;
  };
  const terms = [...new Set(tokenize(question).filter(term => !STOP_WORDS.has(term)).map(normalize))];
  if (terms.length === 0) return [];
  const ranked = chunks
    .map((chunk) => {
      const contentTerms = new Set(tokenize(chunk.text).map(normalize));
      const pathTerms = new Set(tokenize(chunk.path).map(normalize));
      const lexicalScore = terms.reduce((score, term) =>
        score + (contentTerms.has(term) ? 10 : 0) + (pathTerms.has(term) ? 3 : 0), 0);
      return { ...chunk, lexicalScore };
    })
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
