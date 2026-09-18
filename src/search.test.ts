import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import test from "node:test";
import { join } from "node:path";
import { tmpdir } from "node:os";

import {
  CHUNK_LINES,
  MAX_FILE_BYTES,
  chunkText,
  rankLexically,
  searchWorkspace,
  type Chunk,
} from "./search.js";

test("chunkText uses deterministic inclusive line ranges", () => {
  const text = Array.from({ length: CHUNK_LINES + 6 }, (_, index) => `line ${index + 1}`).join("\n");
  const chunks = chunkText("src/example.ts", text);

  assert.deepEqual(
    chunks.map(({ startLine, endLine }) => [startLine, endLine]),
    [
      [1, CHUNK_LINES],
      [CHUNK_LINES - 4, CHUNK_LINES + 6],
    ],
  );
});

test("rankLexically applies score first and lexical path order as the tie-breaker", () => {
  const chunks: Chunk[] = [
    {
      path: "src/z.ts",
      startLine: 1,
      endLine: 2,
      text: "gapless playback",
      lexicalScore: 0,
    },
    {
      path: "src/a.ts",
      startLine: 1,
      endLine: 2,
      text: "gapless playback",
      lexicalScore: 0,
    },
  ];

  const ranked = rankLexically(chunks, "where is gapless playback selected?");
  assert.deepEqual(
    ranked.map(({ path, lexicalScore }) => [path, lexicalScore]),
    [
      ["src/a.ts", 20],
      ["src/z.ts", 20],
    ],
  );
});

test("rankLexically never returns more than the shortlist limit", () => {
  const chunks: Chunk[] = Array.from({ length: 35 }, (_, index) => ({
    path: `src/${String(index).padStart(2, "0")}.ts`,
    startLine: 1,
    endLine: 1,
    text: "gapless",
    lexicalScore: 0,
  }));

  assert.equal(rankLexically(chunks, "gapless").length, 30);
});

test("searchWorkspace respects ignores and skips binary, invalid, and oversized files", async () => {
  const cwd = await mkdtemp(join(tmpdir(), "oko-search-"));
  try {
    await mkdir(join(cwd, ".git"));
    await writeFile(join(cwd, ".gitignore"), "ignored.ts\n", "utf8");
    await writeFile(join(cwd, "good.ts"), "const needle = true;\n", "utf8");
    await writeFile(join(cwd, "ignored.ts"), "const needle = false;\n", "utf8");
    await writeFile(join(cwd, "binary.dat"), Buffer.from([0x6e, 0x65, 0x00, 0x64, 0x6c, 0x65]));
    await writeFile(join(cwd, "invalid.dat"), Buffer.from([0xc3, 0x28]));
    await writeFile(join(cwd, "large.txt"), Buffer.alloc(MAX_FILE_BYTES + 1, "needle"));

    const results = await searchWorkspace(cwd, "needle");
    assert.deepEqual(results.map(({ path }) => path), ["good.ts"]);
  } finally {
    await rm(cwd, { recursive: true, force: true });
  }
});
