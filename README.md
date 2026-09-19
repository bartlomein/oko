# Oko

Find relevant code by asking a question, or rank your own documents and records.
Oko is an experimental Rust CLI and library that combines local BM25 search with
[Jev](https://typesafe.ai/) reranking. Results include file paths, line numbers,
and source snippets.

Oko runs without Node.js. Ordinary searches make one Jev request; optional deep
search lets Jev choose additional searches and reads. You can also use local
BM25 search without an API key.

**Data sent to Jev:** normal search sends your question and selected source
snippets to TypeSafe AI. Deep search can send more snippets across multiple
requests. Supplied-item ranking sends the items you provide, within the request
budget. Use `--no-jev` for local-only operation.

## Quickstart

Install a current stable [Rust toolchain](https://rustup.rs/) and
[ripgrep](https://github.com/BurntSushi/ripgrep), then clone and install Oko:

```sh
git clone https://github.com/bartlomein/oko.git
cd oko
cargo install --path . --bin oko --locked
oko --help
```

Try a local search in this repository immediately—no API key required:

```sh
oko ask "where are search candidates ranked?" --no-jev
```

For Jev reranking, obtain a TypeSafe AI API key and save it once:

```sh
oko auth login
```

Paste the key at the hidden prompt. Oko stores it in your operating system's
credential store, so it works across project directories.

Oko searches **the directory you run it from**:

```sh
cd /path/to/your/project
oko ask "where is authentication handled?"
oko ask "where is authentication handled?" --json
oko ask "how does authentication work?" --intent explanation
oko ask "where is authentication handled?" --deep --max-steps 5
```

Normal searches require the Jev key; `--no-jev` does not. Deep search always uses
Jev. For documents or database rows supplied as JSON, see [Rank documents or database results](#rank-documents-or-database-results).

## Configuration

Manage your saved TypeSafe AI key:

```sh
oko auth login   # Save a key, or replace the saved key
oko auth status  # Show whether a key is configured and its active source
oko auth logout  # Delete the saved key from this computer
```

Login uses hidden terminal input; do not pass keys as command arguments. Status
never displays the key. Login and status do not validate it with TypeSafe; the
next Jev request checks whether it works. Logout deletes the local saved copy,
not the key at TypeSafe. Revoke a key through TypeSafe when needed.

Saved keys use macOS Keychain, Windows Credential Manager, or Linux Secret
Service. Linux needs an available, unlocked Secret Service and session D-Bus.
If secure storage is unavailable, Oko reports an error and never saves a
plaintext fallback. For headless servers and CI, supply `TYPESAFE_API_KEY` through
the environment or an ignored `.env` instead.

Credential priority is **shell environment > current directory `.env` > saved
OS credential**. An explicitly empty environment or `.env` value disables
lower-priority credentials. Login/logout do not edit these overrides; remove
them yourself when switching to a saved key. `--no-jev` bypasses credential
loading entirely.

For project-specific configuration, copy `.env.example` to `.env` and set
`TYPESAFE_API_KEY`. Values are literal, without `$VARIABLE` expansion. Oko only
loads `.env` from the directory you run it from, not its installation folder.
This repository ignores `.env` and `.env.*` except `.env.example`; ensure your
other projects ignore their `.env` files too.

`TYPESAFE_BASE_URL` and `TYPESAFE_DEFAULT_MODEL` may be set in the process
environment. Their defaults are `https://api.typesafe.ai` and `jev-latest`.
These two settings are not read from `.env`.

Code search automatically caches prepared search data in the operating system's
user-cache directory, outside the repository. The first search builds the cache;
later CLI invocations and MCP searches reuse it. MCP also retains the latest
search scope in memory. Every search still discovers files and checks their
contents, so edits, deletions, ignored files, and branch changes refresh the
results. Different search directories have separate cache identities.
Larger scopes use up to four file readers. Unchanged files reuse cached chunk
boundaries, and an unchanged workspace also reuses corpus ranking statistics.
File preparation also uses up to four workers for larger scopes. JavaScript and
TypeScript syntax facts are cached per file; edits invalidate those facts, while
relationships are resolved against the current snapshot. Fresh-process
disk loading runs alongside the source scan; memory reuse skips disk loading.
Returned source is reconstructed from freshly read bytes, with content hashes
and cache validation checked before reuse. Cache format upgrades rebuild once.
The default location is `~/Library/Caches/oko/search` on macOS,
`$XDG_CACHE_HOME/oko/search` (or `~/.cache/oko/search`) on Linux, and
`%LOCALAPPDATA%\oko\search` on Windows.

Set `OKO_CACHE_DIR` in the process environment to choose the cache directory, or
`OKO_NO_CACHE=1` to bypass memory and disk reuse. These settings are not read from
`.env`. An override inside the searched directory uses memory reuse only, so Oko
does not add cache files to the repository it is searching. Cache files contain
source-derived search features and identifiers; keep the directory private.
They are disposable: missing, incompatible, damaged, or
unwritable caches fall back to preparing current files. Caching does not change
ranking or remove Jev requests.

For a local release build instead of installation, run
`cargo build --release --locked --bin oko`. The executable is
`target/release/oko` (`oko.exe` on Windows). Build separately for each operating
system and architecture. Code search requires `rg` on PATH; supplied-item
ranking does not. Node.js is only needed for optional developer benchmark runners.

## Set up Codex for a project

From the project you want to search, run your Oko executable with `setup`:

```sh
oko setup
# Or select a project explicitly:
oko setup --root /absolute/path/to/project
```

Setup installs stable per-user copies of Oko and your available ripgrep binary,
so deleting the original download does not break the connection. On macOS they
live under `~/Library/Application Support/Oko/bin`; other Unix systems use
`~/.local/share/oko/bin`. Setup requires ripgrep either beside the downloaded
Oko binary or on PATH. It does not download dependencies yet.

For Jev, setup reuses the project's `.env` key or your saved OS credential. If a
key is provided through the invoking shell, it saves that key in the OS credential
store so GUI clients can access it. Otherwise it prompts for a key with hidden
input. An explicitly empty key override must be removed first. Setup does not
validate the key with TypeSafe or make paid API requests.

Setup writes a project-local `.codex/config.toml` MCP entry and a managed search
guidance section in `AGENTS.md` (or `AGENTS.override.md` when present). Existing
unrelated settings, comments, and instructions are preserved. Re-running setup
updates its own entry without duplicating instructions. An existing Oko entry
not created by setup is left untouched and reported as a conflict.

Configuration is written atomically, with private backups of changed existing
files under the installation's `setup-backups` directory. Setup prints backup
paths. It adds the machine-local configuration and `.env` to `.gitignore`; this
does not untrack files already committed to Git. Credentials are never embedded
in the generated MCP configuration. The pinned ripgrep path works even when the
GUI has a different PATH than your terminal.

Setup launches the installed server and verifies MCP initialization and discovery
of the search tool. **This verifies the connection, not Codex's actual selection
of Oko or Jev accuracy.** Open the project in Codex, trust it if prompted, and
start a new session. Check `/mcp`, then ask a code-location question.

This first setup flow is **per project and for Codex**. Run setup again for another
project. Codex desktop and CLI share project MCP configuration for trusted
projects; setup does not require a separate Codex CLI installation. OpenCode and
Claude Code setup, downloadable releases, and packaging are subsequent work.

For local-only setup use `oko setup --no-jev`. Use `--no-instructions` to leave
agent instruction files untouched, and `--install-dir DIRECTORY` to choose the
stable binary location. These options also support isolated setup tests.

## Local MCP server (experimental)

The same Rust binary can expose one `search` tool to MCP clients over stdio:

```sh
oko auth login
oko mcp --root /absolute/path/to/project
```

Your coding tool normally starts this process; it is not an interactive terminal
command. Configure a local/stdio MCP connection with the absolute path to the
Oko executable as its command and `mcp`, `--root`, and the absolute project path
as its arguments. No HTTP listener or extra runtime is needed. The current
working directory is used when `--root` is omitted. Set the root explicitly in
GUI clients; their working directory may not be your project.

The server exposes `search` with these inputs:

- `question`: required, nonblank, at most 4096 bytes.
- `directory`: optional subdirectory inside the configured root.
- `intent`: `implementation` (default), `explanation`, or `general`.
- `deep`: optional, defaults to `false`.
- `max_steps`: deep mode only, 1–5, defaults to 5.

Results include an automatic context packet: up to three ranked matches with
source excerpts and up to two supporting definitions or callers. Paths are relative
to the returned search directory, with inclusive line ranges. Context is drawn
from the same file snapshot as the search, deduplicated, and bounded. Detected
function headers remain lexical hints; JavaScript/TypeScript additionally use
cached Tree-sitter function boundaries. Shortened excerpts are marked.
Related lookup preserves call qualification, excludes
unresolved receiver calls, and omits ambiguous definitions instead of filling
the packet with namesakes. It supports simple local Rust module paths and
imports; unsupported syntax falls back to the primary source excerpts. This
is not compiler-level name or type resolution. Related code is found locally;
context expansion adds no model call.

For JavaScript, TypeScript, and TSX, the cached syntax index can attach directly
referenced constants, validators, types, and verified callers. It follows local
bindings, explicit relative imports and aliases, and unambiguous extension
substitution such as `.js` to `.ts`. Explicit `tsconfig.json` path mappings are
supported when their base can be established. Unknown inherited configuration,
re-exports, namespace imports, ambiguous modules, and methods requiring runtime
type information are omitted. Other languages retain conservative lexical
lookup. Parser errors or limits abstain from syntax relationships and retain the
existing lexical fallback. Supporting snippets prioritize explicitly named
symbols, then direct runtime dependencies, callers, and static types. Name and
path relevance break ties within those groups. This does not change the search
ranking sent to Jev.

`definitionComplete` means the returned excerpt contains a proven full definition;
it does not mean every dependency or caller is included. A `resolved_caller`
includes the actual call location in `referencedFrom` and its primary definition
in `target`. Supporting evidence is removed if its primary anchor is trimmed away.

When a top-ranked function has a known boundary and is at most 256 lines,
Oko considers its full implementation instead of the usual 60-line source window.
If the primary source match lacks a complete function boundary, Oko retains its
winning chunk when it fits within 256 lines and known declaration boundaries.
It still marks that excerpt as incomplete; retaining a chunk does not prove a
complete function. This avoids discarding late evidence after Jev selected it.
Under the response cap, lower-ranked matches are dropped first, followed by
related definitions, before shortening the primary excerpt. Helpers whose
references disappear are omitted too. Alternatives remain when they fit; a
complete primary excerpt can accompany a packet-level `truncated` flag because
other evidence was omitted. Larger or uncertain functions retain focused,
bounded excerpts. This policy adds no provider requests.

Multiline signatures are scanned within a fixed limit. When a declaration's
extent cannot be established, Oko returns bounded source context marked
`truncated` rather than treating a header as a complete implementation.
The cached TypeScript parser establishes function boundaries for union and
structural return annotations; uncertain lexical-only boundaries use the fallback.

Normal searches rank compact, line-labelled previews instead of full chunks.
Preview size adapts to the existing 32,000-byte Jev request budget. Winner IDs
map back to original source, so preview markers are never mistaken for source.
Previews prioritize declaration names, attached source annotations or comments,
and implementation statements. Adjacent decorator context can be recovered from
the already loaded corpus; this adds no file reads or provider requests. These
are lexical hints, not parser-verified classifications of tests or functions.
The CLI's `ask` also uses these ranking previews, while retaining its existing
up-to-five-result output. Generic `rank` input is unchanged.
See [source evidence validation](benchmarks/source-evidence.md) for the snippet
repair checks and the limits of the ranking evidence changes.

MCP results include both structured JSON and equivalent text JSON for client
compatibility. The **16,000-byte response cap includes both copies and JSON
escaping**, excluding the small JSON-RPC envelope. Context is trimmed to fit
and `truncated` is set; repeated question text and deep action labels are
separately shortened and marked. Deep results retain investigation counters
and stop reasons. Tool failures return an error without stopping the server.

The response includes `timings` for preparation (including credential lookup),
scan, shortlist, context building, and total server work. Normal `retrieval`
metadata reports candidate counts, budgeted request bytes (before the transport
adds its model field), preview building, and client-side reranking time
(HTTP preparation, provider wait, and parsing).
Deep mode reports investigation time instead. These times exclude Codex's
reasoning, answer generation, and client transport overhead. No request bodies
or credentials are logged. Timing and context metadata are automatic.
`timings.cache` reports cache status, reused/rebuilt file counts, and the time
spent scanning, loading, validating/rebuilding file data, building corpus statistics,
and saving. A disk hit reconstructs source from cached boundaries and validates
saved features; zero rebuilt files means no file tokenization, stemming, or
syntax parsing was repeated. `aggregateReused` indicates whether corpus statistics were reused;
additions, edits, removals, and scope changes rebuild the affected statistics.
`navigationMs` measures rebuilding the syntax lookup indexes from cached facts.
`scanLoadOverlapped` reports concurrent disk loading and scanning. Individual
phase durations can overlap and must not be added to estimate total time.
`shortlistMs` measures query ranking after preparation. CLI `ask --json` exposes
the same cache metadata in its `cache` field.

See [context packet validation](benchmarks/context-packet.md) for offline
coverage checks, local overhead measurements, and validation limits.

Normal mode uses one Jev ranking request for a nonempty shortlist, with an
independent relevance judgment for each candidate. Deep mode is
bounded to five local actions in MCP, unlike the CLI's optional unbounded mode.
Each provider call retains its ten-second timeout. Configure a client tool timeout
of 120 seconds when using deep mode; large repository scans can take longer.
One search runs at a time; concurrent calls receive a busy error. Cancellation
stops before the next search phase or provider call; it does not interrupt a
filesystem scan or an already-running synchronous HTTP request.

Credentials are resolved on each call from the server environment, the configured
root's `.env`, or the OS credential store. Model-selected subdirectories do not
change the credential source. Discovery and startup do not require a key. For
local-only testing, launch `oko mcp --root /path/to/project --no-jev`; this skips
credential loading and rejects deep searches.

Searches rescan current files on each call and cannot select a directory outside
the configured root. File discovery ignores user ripgrep configuration and skips
files resolving outside the searched directory. This is an application boundary,
not an operating-system sandbox. Existing ignored-file and file-size rules apply.
Normal/deep searches send selected snippets to TypeSafe, as described above.

The server advertises when to use Oko, but connecting it does not force the agent
to choose it over native search. Codex project setup is available above. Published binaries and real-client
adoption tests are separate next steps. Automated Rust tests cover actual stdio messages and mock Jev
requests without real credentials.

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
preserving access to prose and unsupported file types. General, explanation,
and lexical-only searches retain the broad ranking.
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
This adds no model requests: normal search still uses one Jev call with the
existing 30-item, 32,000-byte request budget. See the
[rank-fusion audit](benchmarks/rank-fusion.md) for offline results and limits;
the [earlier shortlist audit](benchmarks/shortlist.md) documents the prior design.

The earlier [BM25 validation](benchmarks/bm25.md) scored 42/45 first-result hits and
45/45 top-five hits in both ordinary and deep modes on the existing fixture,
with medians of 1.35 s and 1.95 s respectively. These development results match
the earlier agent comparison on this fixture, not general accuracy parity.

Recognized Rust, JavaScript/TypeScript `function`, and Python `def` declarations
start sections of up to 120 lines, keeping nearby comments with their functions.
This is a declaration heuristic, not an AST parser. Longer sections are split;
other syntax uses 40-line windows. Results preserve source text, relative paths,
and inclusive 1-based line ranges.

Normal `ask` sends the question and up to 30 shortlisted code chunks to TypeSafe
AI for one Jev request. It never sends the full repository. A 32,000-byte request
budget drops trailing candidates while retaining complete chunks. This is a
byte budget, not a token count. Requests have a 10-second timeout and no retries.
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

[Real-repository validation](benchmarks/jev-investigation.md): caching processed
words reduced deep mode's median time from 19.23 s to 1.82 s on the 15-question
fixture. The follow-up scored 34/45 first-result hits and 41/45 top-five hits
for deep mode versus 35/45 and 42/45 for ordinary search (1.19 s median).
Deep mode remains experimental; this optimization targets speed, not accuracy.

## Ranking intent

Both `ask` and `rank` accept `--intent implementation|explanation|general`:

| Intent | Prefers | Default for |
|---|---|---|
| `implementation` | Code that performs the behavior, ahead of docs, examples, tests, or callers | `ask` |
| `explanation` | Content explaining how or why something works, including docs and comments | Explicit selection |
| `general` | Items that directly help answer all or part of the question | `rank` and the Rust library |

The command chooses the default; Oko does not ask AI to guess the intent.
Intent changes the instructions within the existing Jev request. For normal
code search, implementation intent also reserves source candidates as described
above; supplied-item `rank` does not apply this source selection. A normal
nonempty ranking still makes one call, with the same 32,000-byte budget and no
retries. Instructions and candidate text share that budget. No
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
development smoke tests; the full three-repeat Codex comparison below predates
intent support and has not been rerun with the new default.

## Rank documents or database results

Provide a JSON array of items:

```json
[{"id":"ticket-42","text":"Charged twice for the same order.","source":"tickets/42"}]
```

```sh
oko rank --input examples/items.json "Who needs a refund?" --json
```

Files are limited to 1 MiB and 30 items. IDs must be unique, nonempty strings of
at most 200 UTF-16 code units (the existing JavaScript contract); text must be
nonempty. `source` is optional. Unknown fields are discarded.

Results preserve IDs and sources, and return up to five items with relevance
scores above `0.5`. Each candidate gets an independent yes/no relevance judgment
using TypeSafe's Noul primitive; all judgments share one request. Several items
can score highly at once, and the scores do not need to sum to one. The `0.5`
cutoff is an initial decision threshold, not a calibrated accuracy guarantee.
These scores are not comparable with older versions' Choice scores.

`omittedCount` counts items dropped by the 32,000-byte request budget, not items
outside the top five or below the relevance cutoff. Supply candidates in your
existing search order. Independent questions share that budget with the item
text, so large inputs may lose more trailing items than before. Normal code
search fits its previews to the available space. `--no-jev` preserves the input
order with zero scores; it is an input-order baseline, not keyword search.
Oko does not fetch URLs or connect to databases.

See [independent relevance validation](benchmarks/independent-relevance.md) for
the before/after comparison, request-budget tradeoffs, and remaining limitations.

Any programming language can invoke the CLI and consume its JSON output.
Rust applications can use the library directly:

```rust,no_run
use oko::{rank_items, RankItem, RankOptions};

fn main() -> anyhow::Result<()> {
    let items = vec![RankItem {
        id: "ticket-42".into(),
        text: "Charged twice for the same order.".into(),
        source: Some("tickets/42".into()),
    }];
    let result = rank_items("Who needs a refund?", &items, &RankOptions {
        api_key: std::env::var("TYPESAFE_API_KEY").ok(),
        ..Default::default()
    })?;
    println!("{:?}", result.results);
    Ok(())
}
```

The library is synchronous and returns `Result`; async callers should run it on
a blocking worker. It does not load `.env` or the OS credential store itself; pass `api_key` explicitly. Use `no_jev: true` to preserve
input order without a request. `parse_items` validates unknown JSON first.


## Accuracy benchmarks

Benchmarks are optional. Normal use does not require Telemetry Studio or another
private checkout. `cargo test --locked` requires no API key or external checkout. The published code
accuracy results use a small, repeatedly used Telemetry Studio development
fixture; they are not a general claim of matching Codex or OpenCode. The code
benchmark requires a matching checkout and validates its source hashes. The
item benchmark uses synthetic records and is portable.

The application includes native Rust benchmark commands:

```sh
oko benchmark --repo /path/to/telemetry-studio --repeats 1
oko benchmark-items --repeats 1
```

These commands use the same credential priority as searches, including saved
keys. Run them from Oko's directory to save reports under `benchmarks/results/`
(ignored). Each command makes five Jev requests per repeat.
Code benchmarking validates source hashes against the embedded
`benchmarks/telemetry-studio.json` fixture before requesting Jev. It reads the
target repository without editing it. Item benchmarking uses only synthetic
support tickets, including a no-match question.

Reports contain errors, median end-to-end CLI time, and accuracy. Code metrics
separate function-range overlap and exact implementation-range overlap, for both
the first result and top five. Merely returning the correct file does not count.
Both Rust benchmarks time fresh CLI processes; the historical TypeScript item
benchmark measured API ranking time only, so those times are not comparable.
Five questions are a development smoke test, not proof of general accuracy.

To check code-versus-explanation preference on two synthetic candidate sets
(four Jev requests), run `node scripts/benchmark-intents.mjs`. The questions and
candidate contents remain the same while the requested intent changes. Its
fixture is `benchmarks/ranking-intents.json`; reports are ignored under
`benchmarks/results/intents-*/`. The ticket benchmark checks general ranking.

Add `--no-jev` to either benchmark for an offline baseline. Code benchmarks also
accept `--baseline /path/to/report.json` to rescore an earlier report; repository
commit, working-tree status, and expected-source hashes must match. Repeats are
limited to 1–10. Errors count as misses and produce a nonzero exit status.

## Development

Optional Node.js benchmark runners read keys from the shell or this repository's
`.env`; they do not read the OS credential store.

### Compare against Codex CLI

See [the measured Codex comparison](benchmarks/codex-comparison.md) for the first
three-repeat run: Oko 1.22 s median and 12/15 correct first, Codex 19.20 s and
15/15 correct first. Both returned the correct location in the top five on all
trials. These are five repeated development questions, not a general benchmark.

```sh
cargo build --release --locked --bin oko
node scripts/compare-codex.mjs /path/to/telemetry-studio --repeats 3 --model gpt-6-astra --effort medium
```

This developer runner requires Node.js 20.12+ and an authenticated Codex CLI that
supports the selected model. It compares Oko with Jev against fresh, read-only
`codex exec` sessions on the same five verified questions. Three repeats make
15 Oko/Jev requests and 15 Codex sessions. Oko's key comes from this repository's
`.env` or the shell; Codex reuses its saved authentication.

Codex receives only the question and output instructions, without expected
answers. Its final response follows `benchmarks/codex-output.schema.json`.
Both tools return up to five locations with ranges capped at 120 lines. The
runner grades exact/function overlap, first/top-five accuracy, failures, and
end-to-end time. Codex usage and executed commands are saved for inspection;
raw source snippets and provider stderr are not saved. Reports are ignored under
`benchmarks/results/comparison-*/`. Trials rotate tool order, run sequentially, and stop
on the first error. Timeouts are 180 seconds per invocation, with no retries.

The CLI model and reasoning setting are pinned; user config is ignored and
sessions are ephemeral. Repository instructions still apply. The target
repository snapshot and Oko binary are checked before/after the run. Results
compare complete code-location workflows, including AI and network time, not
raw search-engine performance. These development questions are not held out.

Use `--codex-bin /absolute/path/to/codex` to select another installed executable.
On this machine, the shell's CLI 0.143.0 rejects `gpt-6-astra`; the compatible
desktop-bundled executable is `/Applications/ChatGPT.app/Contents/Resources/codex`
(0.153.4). The runner records the actual CLI version in each report.

```sh
node --test scripts/compare-codex.test.mjs
```

### Compare against Codex and OpenCode

See [the expanded comparison](benchmarks/agent-comparison.md) for methodology
and recorded results.

```sh
cargo build --release --locked --bin oko
node scripts/compare-agents.mjs /path/to/telemetry-studio --repeats 3 --model gpt-5.5 --effort medium
```

This runner uses **the same model and reasoning setting in both agent CLIs**:
Codex receives `gpt-5.5`; OpenCode receives `openai/gpt-5.5` with the `medium`
variant. Both CLIs must be installed and authenticated for that model. Oko
continues to use Jev; it is the system being compared, not a third GPT agent.
Override executable paths with `--codex-bin PATH` and `--opencode-bin PATH`.
No global configuration is changed.

The expanded fixture has 15 source-verified questions: the original five
`development` questions and ten `new` questions written before this comparison.
Reports show those groups separately. Three repeats make 45 trials per tool,
135 total. These are mostly localized function lookups in one Rust project,
not a broad evaluation of coding agents or search across documents/databases.

OpenCode runs fresh sessions with external plugins disabled and a dedicated
read-only agent using its native read, glob, and grep tools. Shell, edits,
web, and delegation are denied. These are tool permissions, not an OS sandbox.
Codex uses its read-only sandbox and usual local shell tools. Same model does
not make system prompts, tool access, provider caching, or context identical.
OpenCode stores local sessions; Codex sessions are ephemeral. No sessions are
shared. The runner stores parsed locations, step token counters, and tool types,
not raw source/tool outputs.

Each tool rotates through execution positions across repeats. Trials run
sequentially with a 180-second timeout and no automatic retries. Errors count
as misses and stop the run; incomplete reports are explicitly marked. All
returned paths/ranges are checked, and repository snapshots and source hashes
are verified. Median latency excludes failed trials. “Exact” accuracy means
line-range overlap with the verified implementation, not an exact boundary match.

Use `--tools codex,opencode` to run just those agents, or `--fixture PATH` for
a different repository's JSON questions and source-verified expected locations.
The default expanded fixture is `benchmarks/telemetry-studio-expanded.json`.
The offline runner tests require neither the target repository nor API keys:

```sh
node --test scripts/compare-codex.test.mjs
```

### Rust checks

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Third-party parser/stemmer notices are in `THIRD_PARTY_NOTICES.md`.

## License

Oko is licensed under the [MIT License](LICENSE). Attribution and license terms
for the included parser and stemmer code are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
