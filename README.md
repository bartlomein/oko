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

The published command is `oko` after installing the package globally or using it through a package runner:

```sh
oko ask "where is gapless playback selected?"
oko ask "where is gapless playback selected?" --no-jev
```

Oko discovers files with `rg --files`, reads only bounded UTF-8 text files, chunks them by lines, and ranks a deterministic shortlist lexically. Results include relative paths, inclusive 1-based line ranges, scores, and the ranking method.

Normal `oko ask` searches require `TYPESAFE_API_KEY`. Oko sends exactly the question and the shortlisted code chunks to TypeSafe AI for one Jev reranking request; it never sends the full repository. Missing credentials and Jev request failures exit non-zero instead of silently changing the ranking method.

For local A/B benchmarking, pass `--no-jev`. This explicitly skips the Jev request, uses the lexical ranking only, and reports `lexical-only (--no-jev)` in human output. The opt-out is the only normal way to run without `TYPESAFE_API_KEY`.

JSON output is written only to stdout, so diagnostics can safely be read from stderr.

## Development

```sh
npm run typecheck
npm test
```

Node.js 20 or newer is required.
