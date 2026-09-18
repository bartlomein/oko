# Oko

Oko is a small TypeScript CLI for finding relevant code in the current working directory.

## Usage

```sh
npm install
npm run build
node dist/cli.js ask "where is gapless playback selected?"
node dist/cli.js ask "where is gapless playback selected?" --json
node dist/cli.js ask "where is gapless playback selected?" --no-jev
```

Put your TypeSafe AI API key in a `.env` file in the directory where you run Oko:

```dotenv
TYPESAFE_API_KEY=your-api-key
```

You can copy `.env.example` to `.env` to get started. Oko loads this file automatically;
existing shell environment variables take precedence. `.env` is ignored by Git.

The published command is `oko` after installing the package globally or using it through a package runner:

```sh
oko ask "where is gapless playback selected?"
oko ask "where is gapless playback selected?" --no-jev
```

Oko discovers files with `rg --files`, reads only bounded UTF-8 text files, and
ranks a deterministic shortlist. Search splits snake_case and camelCase names,
ignores common question words, and matches English word forms with the
[Porter stemmer](https://github.com/words/stemmer) (for example, `renaming` and `rename`).
Recognized Rust, JavaScript/TypeScript `function`, and Python `def` declarations
start sections of up to 120 lines, keeping nearby comments and function bodies
together. This is a declaration heuristic, not an AST parser: long sections are
split, and other syntax uses 40-line windows. Results retain exact source text,
relative paths, inclusive 1-based line ranges, scores, and the ranking method.

Normal `oko ask` searches require `TYPESAFE_API_KEY`. Oko sends the question and
up to 30 shortlisted code chunks to TypeSafe AI for one Jev reranking request;
it never sends the full repository. Each snippet appears once. A conservative
32,000-byte request budget drops the lowest-ranked candidates when necessary,
keeping the retained chunks complete. This is a byte budget, not an exact token
count. Missing credentials and Jev request failures exit non-zero instead of
silently changing the ranking method.

For local A/B benchmarking, pass `--no-jev`. This explicitly skips the Jev request, uses the lexical ranking only, and reports `lexical-only (--no-jev)` in human output. The opt-out is the only normal way to run without `TYPESAFE_API_KEY`.

JSON output is written only to stdout, so diagnostics can safely be read from stderr.

## Rank documents or database results

Supply a JSON array of `{ "id": "unique-id", "text": "searchable content", "source": "optional URL or label" }`.
Oko accepts up to 30 items and a JSON file up to 1 MiB. IDs and text must be
non-empty strings; IDs must be unique and at most 200 characters. Other fields
are discarded, so select the text you want to send explicitly.

```sh
node dist/cli.js rank --input examples/items.json "Who needs a refund?" --json
```

`rank` loads `.env` from the current directory and uses the same Jev request
budget as code search. It preserves item IDs and optional source strings. It
returns up to five items, or an empty list if none beats the `none` choice.
`omittedCount` reports trailing items excluded by the request budget (not the
number outside the top five). Supply candidates in your existing search order.
`--no-jev` keeps that input order with zero scores; it is a baseline, not keyword
search. Scores are relative to the supplied candidates, not universal relevance
probabilities. Only supplied text/source/IDs are sent; Oko does not fetch URLs or
connect to a database.

The reusable ESM API is exported from the package, after building:

```js
import { rankItems } from 'oko';

const result = await rankItems('Who needs a refund?', [
  { id: 'row-42', text: 'Charged twice for the same order.', source: 'tickets/42' },
], { apiKey: process.env.TYPESAFE_API_KEY, limit: 5 });
console.log(result.results);
```

The library does not load `.env` itself; the caller provides `apiKey`.
Use `noJev: true` for input order without an API call. Validation failures and
provider errors reject the promise. `parseItems` is also exported for validating
unknown JSON before ranking.

Run the synthetic support-ticket benchmark (five Jev requests, no customer data):

```sh
npm run benchmark:items
```

Its report compares input order with Jev, includes a no-match question, and is
saved under `benchmarks/results/`. It measures ranking API time, unlike the code
benchmark's end-to-end CLI time. Neither small benchmark establishes general
search accuracy.

## Search benchmark

From the Oko directory, run:

```sh
npm run benchmark -- /Users/bart/dev/telemetry-studio
```

This loads Oko's `.env`, runs five source-verified questions through lexical-only
search and Jev, and writes timestamped Markdown/JSON reports under
`benchmarks/results/` (ignored by Git). It makes at most five paid Jev requests.
For three repetitions per question, append `3` (at most 15 requests).

Reports measure function-location hits and exact implementation hits separately,
each for the first result and top five, plus errors and median end-to-end CLI time.
Function hits overlap the verified function range; exact hits overlap the original
implementation lines in `benchmarks/telemetry-studio.json`. Merely matching the
file does not count. Source hashes prevent stale
expected answers from silently being used; review the source before updating them.
Only paths and line ranges are saved in results, not returned source snippets.

To compare with an earlier JSON report, pass its path after the repeat count:

```sh
npm run benchmark -- /path/to/telemetry-studio 1 /path/to/baseline/report.json
```

The earlier results are rescored with the same metrics, preserving their recorded
timings. The repository commit, working-tree status and verified source hashes
must match. Original reports are never overwritten. Larger chunks can carry more
context to Jev, so latency and request size may increase.

These five questions are a small development smoke test, not a general accuracy
claim. Use additional held-out questions before claiming broader improvements.
The benchmark reads the target repository without editing it.

## Development checks

```sh
npm run typecheck
npm test
```

Node.js 20 or newer is required.
