import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import test from "node:test";
import { join } from "node:path";
import { tmpdir } from "node:os";

import {
  CHUNK_LINES,
  FUNCTION_CHUNK_LINES,
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

test("natural-language word forms find code despite prose distractors", () => {
  const chunks = [
    ...chunkText("notes.md", "A record is saved by it in a file."),
    ...chunkText("src/persist.rs", "// Atomic write followed by rename.\nfn save() {}"),
  ];
  assert.equal(rankLexically(chunks, "Where is it saved atomically by writing and renaming?")[0].path, "src/persist.rs");
  assert.deepEqual(rankLexically(chunks, "where is it by the"), []);
});

test("snake_case and camelCase identifiers match separate query words", () => {
  for (const name of ["refresh_access_token", "refreshAccessToken", "HTTPAccessToken"]) {
    assert.ok(rankLexically(chunkText("src/client.ts", `const ${name} = true;`), "access token")[0]?.lexicalScore > 0);
  }
  assert.ok(rankLexically(chunkText("src/device_csv_parser.rs", "fn read() {}"), "device CSV")[0]?.lexicalScore > 0);
});

test("function sections retain declarations, comments and body with truthful ranges", () => {
  const lines = ["use example;", "", "/// Parses sensor records.", "pub fn parse_sensor() {",
    ...Array.from({ length: 65 }, (_, i) => `    let point_${i} = read();`), "}",
    "fn another() {}"];
  const chunks = chunkText("src/sensor.rs", lines.join("\n"));
  const found = rankLexically(chunks, "parsing sensor records")[0];
  assert.equal(found.startLine, 3);
  assert.equal(found.endLine, 70);
  assert.ok(found.text.includes("point_64"));
  for (const chunk of chunks) {
    assert.equal(chunk.text, lines.slice(chunk.startLine - 1, chunk.endLine).join("\n"));
  }
});

test("large functions stay bounded and every source line remains covered", () => {
  const lines = ["async function processRecords() {", ...Array(300).fill("  consume();"), "}"];
  const chunks = chunkText("worker.js", lines.join("\n"));
  assert.ok(chunks.every(c => c.endLine - c.startLine + 1 <= FUNCTION_CHUNK_LINES));
  for (let line = 1; line <= lines.length; line++) {
    assert.ok(chunks.some(c => c.startLine <= line && c.endLine >= line));
  }
  assert.deepEqual(chunkText("empty.rs", ""), []);
});

test("Rust attributes do not detach function documentation", () => {
  const text = ["use example;", "/// Requires adjacent clips.", '#[cfg(not(target_os = "windows"))]',
    "pub(crate) fn eligible() -> bool { true }"].join("\n");
  const found = rankLexically(chunkText("playback.rs", text), "adjacent clips")[0];
  assert.equal(found.startLine, 2);
  assert.ok(found.text.includes("fn eligible"));
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
