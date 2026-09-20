# Search reference

[← Back to Oko](../README.md)

## Code search

Oko discovers files with `rg --files`, reads UTF-8 text files up to 256 KiB,
rejects binary/invalid UTF-8 content, and builds a deterministic shortlist.
Search splits snake_case and camelCase names, ignores common question words,
and matches English word forms using the Porter stemmer.

Candidates use BM25 with `k1=1.2` and `b=0.75`: uncommon query words carry
more weight, repeated words have diminishing returns, and document length is
normalized. Two rankings select up to 100 local candidates each: content/path
matches (`content + 0.3 * path`) and the same evidence boosted by function names.
Reciprocal rank fusion (`k=60`) merges their positions into a shortlist of up to
30 candidates. A bounded cross-file identifier hint favors referenced
declarations; this is a lexical heuristic, not a resolved call graph.
Identical source candidates are counted once. Final selection suppresses
same-file excerpts overlapping at least half the shorter range, while distinct
functions and the lightly overlapping windows of long functions remain eligible.
There is no blanket penalty for additional matches from the same file.
For normal implementation searches, up to 15 of the 30 slots are reserved for
matching source candidates from the same ranking applied to supported code files.
Remaining slots come from the broad ranking, with overlapping excerpts counted
once. This prevents documentation from crowding out all implementations while
preserving access to prose and unsupported file types. Files in conventional
test locations or with conventional test names (`tests/`, `__tests__/`,
`test_*.py`, `*.test.ts`, `*_test.go`, ...) do not take the reserved slots
unless the question mentions tests: they repeat the vocabulary of what they
exercise and outnumber it, and on replayed agent questions they held about a
third of the shortlist. They still compete in the broad ranking. General,
explanation, and lexical-only searches retain the broad ranking.
The last four slots go to short matching chunks (under 60 lines) that directly
adjoin one of the five strongest candidates in the same file. A helper a few
lines long has too few words to rank by itself, yet a question about its larger
neighbour often needs it. A neighbour must match the question; adjacency only
decides between otherwise weak candidates. On 169 replayed agent questions these
two rules raised the share of expected locations reaching the shortlist from 76%
to 89% without lowering it for any task.
Declaration hints cover common Rust, Python, JavaScript/TypeScript, Go, Java,
C#, C/C++, Kotlin and Swift syntax. Other syntax and non-code files retain
content/path search. No repository-specific paths or framework rules are used.

Term counts, path tokens, and symbol features are reused across searches when
their source contents are unchanged. Global statistics refresh when the corpus
changes. Ranking, previews, and returned context all use the same captured
snapshot for each request. Only selected chunks are cloned for ranking results.
Deep search retains full-snapshot statistics even
when filtering to source files. Scores are relevance values, not probabilities;
shortlist order reflects rank fusion while reported lexical scores retain the
underlying BM25 relevance. Compact previews prioritize declaration headers over
ordinary variable assignments and preserve a bounded multiline header prefix.
They also prioritize up to 12 contiguous lines of a query-matching control-flow
block, so nearby predicates and outcomes can survive the preview budget.
This is an indentation-based context hint, not a language parser; multiline
conditions and unsupported syntax use the existing preview selection.
Preview selection itself adds no model requests. Normal search uses one Jev call
on a hit, with the existing 30-item, 32,000-byte request budget. See the
[rank-fusion audit](../benchmarks/rank-fusion.md) for offline results and limits;
the [earlier shortlist audit](../benchmarks/shortlist.md) documents the prior design.

The earlier [BM25 validation](../benchmarks/bm25.md) scored 42/45 first-result hits and
45/45 top-five hits in both ordinary and deep modes on the existing fixture,
with medians of 1.35 s and 1.95 s respectively. These development results match
the earlier agent comparison on this fixture, not general accuracy parity.

Recognized Rust, JavaScript/TypeScript `function`, and Python `def` declarations
start sections of up to 120 lines, keeping nearby comments with their functions.
This is a declaration heuristic, not an AST parser. Longer sections are split;
other syntax uses 40-line windows. Results preserve source text, relative paths,
and inclusive 1-based line ranges.

Normal `ask` sends the question and up to 30 shortlisted code chunks to TypeSafe
AI for an initial Jev request. If all candidates are rejected, ordinary `ask` and
MCP search make at most one recovery request: up to eight leading candidates with
fuller previews plus up to eight previously unconsidered candidates. The original
question, intent, directory scope, and relevance threshold stay unchanged. MCP
reuses the captured snapshot and prepared index; recovery does not rescan files.
An identical-evidence retry is skipped. Genuine misses can still return nothing.
Deep mode retains its existing investigation budget; generic `rank` is unchanged.

It never sends the full repository. Each request is limited to 32,000 bytes before
the model field is added. This is a byte budget, not a token count. Requests have
a 10-second timeout; HTTP errors are not retried. MCP retrieval metadata (written
to `OKO_METRICS_FILE`, not returned to the agent) includes
`attempts`, `recoveryCandidates`, and `recovered`. `requestBytes`, `previewMs`, and
`rerankMs` sum both attempts; `rankedCandidates` and `omittedCandidates` describe
the initial attempt. Recovery can therefore add one provider request and up to
another request timeout only on an empty first result.
Missing credentials or provider failures exit nonzero.

`--no-jev` skips the API and returns lexical ranking. JSON goes to stdout;
diagnostics go to stderr. Intent-aware instructions refine the default code ranking.

## Jev-only investigation

```sh
oko ask --deep --max-steps 5 "Where is authentication handled?"
oko ask --deep --json "Where are optional values interpolated?"
```

`--deep` adds a search/read/decision loop using the same `TYPESAFE_API_KEY`.
No Codex, OpenCode, OpenAI or Anthropic connection is used. Normal `ask` remains
the fast single-pass path.

Oko proposes searches from the question's words and adjacent word pairs, with
source-code searches for implementation intent. It also offers nearby chunks
and definitions of symbols observed in results. Jev chooses the next offered
action or finishes, then reranks newly discovered evidence with existing results.
Jev selects typed actions: it does not generate arbitrary queries or shell commands.
This is a constrained investigation loop, not a full OpenCode-style coding agent.

`--max-steps N` optionally limits local search/read actions, including the initial
search. A step can require a ranking call and an action-selection call; JSON
reports the actual `jevCalls`. No step cap is imposed when omitted. The loop
also stops when Jev chooses to finish or all offered actions are exhausted.
Actions that expose the same evidence are deduplicated. An empty answer cannot
finish while untried actions remain; Jev must choose another action. Actions do
not repeat. Each provider call retains the existing ten-second timeout,
32 KB request budget and no automatic retries. No total wall-time or token-cost
budget is implemented. Use a step limit when bounding API use matters.

The repository is read once into a snapshot using the same ignored-file, UTF-8,
256 KiB file-size and chunking rules as ordinary search. Local actions only use
that snapshot; there are no edits, shell commands or reads of model-provided paths.
Progress goes to stderr. JSON includes `investigation` with steps, call count,
action trace, omitted candidates and stop reason. `complete` is true only when
Jev chooses to finish; a budget stop returns current findings with `complete:false`.
That flag is the controller's stopping decision, not proof the answer is correct.
`--deep` cannot be combined with `--no-jev` or used with generic `rank` inputs.

Run the synthetic live smoke benchmark with `node scripts/benchmark-investigation.mjs`
after building the release binary. It generates toy source and distractor docs,
uses Jev, and saves reports under `benchmarks/results/`. On September 18, 2026,
deep search found the expected function first in 3/3 repeats versus 0/3 for
single-pass search; median times were 1.17 s and 0.47 s respectively. Deep search
used two steps and three Jev calls per run, stopping with actions exhausted.
This is a development case, not a held-out accuracy evaluation.

For a real-repository comparison using only Jev:

```sh
node scripts/compare-investigation.mjs /path/to/repository /path/to/fixture.json
```

The fixture uses the same source-hashed expected locations as the agent comparison.
The runner compares ordinary search with five-step deep search over three repeats,
rotates execution order, and records accuracy, elapsed time, Jev calls and action
traces. Repository snippets are sent to Jev. Results are saved under
`benchmarks/results/deep-comparison-*/`; source and executable snapshots are
checked before and after the run.

[Real-repository validation](../benchmarks/jev-investigation.md): caching processed
words reduced deep mode's median time from 19.23 s to 1.82 s on the 15-question
fixture. The follow-up scored 34/45 first-result hits and 41/45 top-five hits
for deep mode versus 35/45 and 42/45 for ordinary search (1.19 s median).
Deep mode remains experimental; this optimization targets speed, not accuracy.

## Ranking intent

Both `ask` and `rank` accept `--intent implementation|explanation|general`:

For editing requests, implementation relevance means the existing code that needs
to change. A requested new heading, value, or behavior does not need to exist in
the source yet. Code mentioned only as something to leave unchanged is not the
edit target. This uses the same ranking call and preserves no-match results.

| Intent | Prefers | Default for |
|---|---|---|
| `implementation` | Code that performs the behavior or owns the requested edit, ahead of docs, examples, tests, or callers | `ask` |
| `explanation` | Evidence explaining or demonstrating how or why something works: code, configuration, docs, or comments; explanatory prose is not required | Explicit selection |
| `general` | Items that directly help answer all or part of the question | `rank` and the Rust library |

The command chooses the default; Oko does not ask AI to guess the intent.
Intent changes the instructions within the existing Jev request. For normal
code search, implementation intent also reserves source candidates as described
in [code search](search.md#code-search); supplied-item `rank` does not apply this source selection. Supplied-item ranking makes one call. Normal code search adds at most one
changed-evidence recovery call on an empty result, with the same per-request
32,000-byte budget. HTTP errors are not retried. Instructions and candidate text share that budget. No
file types are excluded from discovery. `--no-jev` makes
zero calls and bypasses intent-based ranking, preserving its existing results.

```sh
oko ask "where is authentication handled?" --intent general
oko rank --input articles.json "how does authentication work?" --intent explanation
```

For library callers, set `RankOptions.intent` to `RankingIntent::Implementation`,
`RankingIntent::Explanation`, or `RankingIntent::General` (the default). Existing
callers using `..Default::default()` keep general ranking. Full struct literals
must supply the new field. `prepare_request` retains general behavior;
`ranking::prepare_request_with_intent` exposes explicit request preparation.

Initial validation on 2026-09-18: code locations improved from 4/5 to 5/5 correct
first results in one live pass (1.19 s median), generic tickets remained 5/5, and
all four synthetic implementation/explanation checks passed. These are small
development smoke tests; the [three-repeat Codex comparison](benchmarks.md#compare-against-codex-cli) predates
intent support and has not been rerun with the new default.
