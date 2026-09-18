# Oko

Oko is a small TypeScript CLI for finding relevant code in the current working directory.

## Usage

```sh
npm install
npm run build
node dist/cli.js ask "where is gapless playback selected?"
node dist/cli.js ask "where is gapless playback selected?" --json
```

The published command is `oko` after installing the package globally or using it through a package runner:

```sh
oko ask "where is gapless playback selected?"
```

Oko discovers files with `rg --files`, reads only bounded UTF-8 text files, chunks them by lines, and ranks a deterministic shortlist lexically. Results include relative paths, inclusive 1-based line ranges, scores, and the ranking method.

If `TYPESAFE_API_KEY` is set, Oko sends exactly the question and the shortlisted code chunks to TypeSafe AI for one Jev reranking request. It never sends the full repository. If that request fails, Oko warns on stderr and returns the lexical results instead. Without the key, Oko stays local.

JSON output is written only to stdout, so diagnostics can safely be read from stderr.

## Development

```sh
npm run typecheck
npm test
```

Node.js 20 or newer is required.
