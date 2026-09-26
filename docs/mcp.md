# MCP reference

[← Back to Oko](../README.md)

## Set up a project

From the project you want to search, run your Oko executable with `setup`:

```sh
oko setup                          # Codex (the default)
oko setup --client claude          # Claude Code
oko setup --client opencode        # OpenCode 1.x
oko setup --client claude,opencode # several, or: --client all
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

What setup connects depends on the tool:

| Tool | Connection | Guidance |
| --- | --- | --- |
| Codex | `.codex/config.toml` in the project | `AGENTS.md`, or `AGENTS.override.md` when present |
| Claude Code | `claude mcp add-json --scope local` with `alwaysLoad`, stored by Claude Code for you and this project | `CLAUDE.md`, the file it links to inside the project, or `AGENTS.md` when `CLAUDE.md` imports `@AGENTS.md` |
| OpenCode 1.x | `opencode.json` in the project | `AGENTS.md` |

The guidance is a managed section between `oko:search` markers. Existing
unrelated settings, comments, and instructions are preserved. Re-running setup
updates its own entry without duplicating instructions. An existing Oko entry
not created by setup is left untouched and reported as a conflict.

Claude Code setup needs the `claude` command on PATH (or its path in
`OKO_CLAUDE`). OpenCode setup rewrites `opencode.json` as formatted JSON when it
adds the entry; it does not edit `opencode.jsonc`, a file with comments, or the
v2 `mcp.servers` layout, and says so without changing anything. A new
`opencode.json` is added to `.gitignore`. One that already existed is not, since
it may be shared: it now holds this machine’s paths, so keep that change out of
shared commits.

Configuration is written atomically, with private backups of changed existing
files under the installation's `setup-backups` directory. Setup prints backup
paths. It adds the machine-local configuration and `.env` to `.gitignore`; this
does not untrack files already committed to Git. Credentials are never embedded
in the generated MCP configuration. The pinned ripgrep path works even when the
GUI has a different PATH than your terminal.

Setup launches the installed server and verifies MCP initialization and discovery
of the search tool. **This verifies the connection, not the agent's actual selection
of Oko or Jev accuracy.** Start a new session (in Codex, trust the project if
prompted). Check `/mcp` or `opencode mcp list`, then ask a code-location question.

Setup is **per project**. Run it again for another project. Codex desktop and CLI
share project MCP configuration for trusted projects; setup does not require a
separate Codex CLI installation. For downloads, see the [installation guide](installation.md); see [release maintenance](releasing.md).

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
to the searched directory, with inclusive line ranges. Context is drawn
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

### Result format

The tool result is one plain-text block, in ranked order, with no JSON escaping,
scores, or serving metadata:

````text
src/email/retry-backoff.constant.ts:1-7 (whole file)
```
1	import { type QueueJobBackoffOptions } from '...';
2
3	export const EMAIL_SEND_RETRY_BACKOFF = {
...
```

Definition referenced from src/email/retry-backoff.constant.ts:1:
src/queue/job-options.ts:12-20 (complete definition)
```
12	export type QueueJobBackoffOptions = {
...
```
````

Each excerpt is headed `path:start-end (label)` and followed by the exact current
source in a fence longer than any backtick run it contains. Every source line is
prefixed with its file line number and a tab. Models count lines unreliably: given
only a starting line and a hundred lines of code, agents located the right
statements but reported ranges one to six lines off. The tool description tells
agents to cite these numbers, to drop the prefix when editing, and not to
re-read lines already shown. In a three-client run, Codex re-read the returned
range in four of five sessions and OpenCode in all five even when the first
response held everything the task needed, at four to six seconds per turn. The prefix costs
about a tenth more response bytes; the structured packet in `OKO_METRICS_FILE`
keeps unnumbered text. The label describes
only that excerpt, never its relevance or whether the results answer the whole
question; the tool description says so, because an agent that stops at the first
plausible excerpt misses multi-location answers:

- `whole file`: the excerpt is the entire file. A file without a provable
  declaration boundary, such as a constants or configuration module, is still
  whole and is not reported as incomplete.
- `complete definition`: a proven full definition (`definitionComplete`); it does
  not mean every dependency or caller is included. `2 complete definitions` means
  the match and the short definition or definitions that directly follow it
  (`definitions`), described below.
- `partial excerpt`: the enclosing code continues outside the range (`truncated`),
  so the file should be read when the rest matters.
- `body abridged`: a complete definition too long to show whole (over 256 lines).
  Its signature, the start of its body, the ranked evidence when that sits
  deeper, and its end are shown; each gap is marked
  `… N lines omitted (path:a-b) …` and listed in `omitted`.
- `outline`: the same for a class the parser knows the members of: the class
  header and one line per member (at most 60), with the gaps marked.
- `exact name match`: the definition of a name the question used, shown
  although the ranker did not accept it (`exactName`). A name match, not a
  relevance claim; see "Names" below.

Supporting excerpts are introduced by `Definition referenced from`, `Caller of`
(parser-resolved bindings, with the reference or target location), or
`Possible definition referenced from` (a lexical name match only). Supporting
evidence is removed if its primary anchor is trimmed away. A search of a
subdirectory starts with `Paths are relative to <directory>/.`; deep searches
state their step count and stop reason; an empty result says so and suggests
rephrasing or grep. When whole matches or related excerpts were dropped to fit
the response cap, the text ends with a note saying so; having more accepted
matches than the three shown is not reported as a size cut, because those are
named after the excerpts. Tool failures are a single error text.

The same packet as structured JSON (`results`, `related`, `truncated`, and per
excerpt `wholeFile`, `definitionComplete`, `truncated`, `symbol`, `score`) is
available to operators through [`OKO_METRICS_FILE`](#timings-and-retrieval-metadata),
never to the agent.

An excerpt that begins at a declaration also includes the decorators, attributes,
and comments directly above it, up to a blank line, other code, or another
declaration: `@classmethod` is part of what a method is, and an edit to a
function usually touches its comment.

A short definition (up to 20 lines, 30 in total) that directly follows a shown
complete definition comes with it when the shown code references it by name or
the question asks about it by name: the predicate a parser calls, the sibling
method the question names. Only blank lines may separate them. On 169 replayed
agent questions, over a quarter of the expected code missing from a response
began one or two lines after a shown definition, and agents read on regardless,
at a model turn each. A definition tied to neither stays out, so responses do
not grow with unrelated neighbours.

Most responses use one or two of the three excerpt slots and a fraction of the
response cap. A spare slot is given to a candidate Jev rated from 0.35 up to its
0.5 cutoff, as a focused window labelled `possible match` (`lowerConfidence`),
only beside at least one accepted match and only while the response is under
6,000 bytes. Nothing accepted remains an empty result. In the same replay 11 of
30 such excerpts held expected code that was otherwise missing; at 0.2 it would
have been 12 of 65. Together these two rules raised responses containing
everything the task needed from 67% to 84% for 12% more bytes, with no question
losing coverage. The following-definition rule alone reaches 79%. The label
states what the excerpt is and gives no instruction such as "check it": agents
that follow instructions literally turn that into extra reads. In agent runs no
client made more calls after receiving one, and Claude used the useful ones
without a follow-up.

After the excerpts, a normal search names up to six further candidates by
`path:start-end` only, under `Other candidates, not shown, best first:`. They
come from everything Jev judged for the search: the shortlist, the files
connected to its strongest matches, and the next keyword matches (see
[search](search.md)). They cost one line each; candidates Jev rated irrelevant (below
0.2) and anything overlapping a shown excerpt are left out. In keyword order the
heading is `Other keyword matches, not shown:`. An agent that needs more can
open one of these instead of starting a blind search. `retrieval.candidates` in
`OKO_METRICS_FILE` records every candidate's path, lines, and relevance, which
shows whether missed code was judged irrelevant, fell below the threshold, or
was never shortlisted.

When the line that best matches the question is in a comment directly above a
declaration in the winning chunk, the declaration is treated as the match: doc
comments often repeat the question better than the code they document.

When a ranked match lies in a definition (function, method, class, type or
constant) with a known boundary of at most 256 lines, Oko returns the full
implementation instead of the usual 60-line source window, for every match and
not only the first: a window that stops a few lines short of the relevant
statement costs a follow-up read, or a wrong answer. A longer complete
definition is abridged rather than windowed (`body abridged`, `outline`), so
its signature and end are always visible.
Under the response cap, lower-ranked complete definitions are first narrowed
back to that window, lowest rank first, before any match is dropped.
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
See [source evidence validation](../benchmarks/source-evidence.md) for the snippet
repair checks and the limits of the ranking evidence changes.

The agent receives a single copy of the evidence: no `structuredContent` and no
output schema. Clients that receive both forms show the model two copies or only
the JSON one, and the calling agent pays for every byte on each later turn. The
**16,000-byte response cap covers the serialized result including JSON escaping
of the text**, excluding the small JSON-RPC envelope. Context is trimmed to fit
and marked as described above. Tool failures return an error without stopping
the server.

### Timings and retrieval metadata

Serving metadata does not inform the agent's next step, so it is not part of the
tool result. Set `OKO_METRICS_FILE` in the server's process environment to append
one JSON line per completed search: the question (shortened to 512 bytes and
marked), searched directory, ranking mode, `timings`, `retrieval`, deep
`investigation` counters with shortened action labels, `responseBytes`,
`responseLimitBytes`, and the structured packet. The file contains source
excerpts and the question; keep it private. It is not read from `.env`, failed
searches record nothing, and a write failure is reported on stderr without
failing the search. The benchmark launchers set it per trial.

### Startup preparation

The server starts preparing the configured root in a background thread as soon
as it launches, before the client has finished connecting. A coding agent
usually spends several seconds starting up and composing its first request, so
the first search normally finds the snapshot in memory and only reconciles
changes, instead of paying for a disk load or a cold build. It still rescans
current files: preparation never makes a search return older source. A search
that arrives while preparation is running waits for it rather than repeating it
(`timings.cacheWaitMs`), which costs one extra reconciliation compared with
doing the work itself. Preparation failures are left for the first search to
report. Set `OKO_NO_PREWARM=1` to prepare on the first search instead, for
example when many servers are started for sessions that rarely search;
preparation is also skipped with `OKO_NO_CACHE=1`, because nothing would be
retained. A search of a subdirectory prepares that scope separately.

When `OKO_METRICS_FILE` is set, finished preparation is recorded as
`{"event":"prewarm","cache":{...}}` with the same fields as `timings.cache`,
always before the line of any search that uses it. That search reports
`memory`; the event shows whether the session started `cold` or from `disk`.

Each search line includes `timings` for preparation (including credential lookup),
scan, shortlist, context building, and total server work. Normal `retrieval`
metadata reports candidate counts, budgeted request bytes (before the transport
adds its model field), preview building, and client-side reranking time
(HTTP preparation, provider wait, and parsing).
Deep mode reports investigation time instead. These times exclude Codex's
reasoning, answer generation, and client transport overhead. No request bodies
or credentials are logged.
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

See [context packet validation](../benchmarks/context-packet.md) for offline
coverage checks, local overhead measurements, and validation limits.

Normal mode judges a nonempty shortlist in one Jev request, with an
independent relevance judgment for each candidate, and judges connected files
and further keyword matches in two requests beside it. Only the shortlist's
request decides what is shown or whether the search falls back to keyword order. Deep mode is
bounded to five local actions in MCP, unlike the CLI's optional unbounded mode.
A normal MCP search waits four seconds for each Jev request (`OKO_JEV_TIMEOUT_MS`,
500–10000). Jev usually answers in about half a second; when it is slow,
unreachable, rate-limited, or returns a server error, the search returns the
keyword-ranked shortlist instead of an error. The result then begins with a
line saying that the matches are in keyword order and should be verified, and
the metrics line reports `ranking: "lexical-fallback"` and
`retrieval.lexicalFallback` (`timeout`, `unreachable`, or `unavailable`).
Rejections that need an operator, such as an invalid key, are still errors. If
Jev judged the first shortlist irrelevant and the recovery request then fails,
the result stays empty: keyword order does not overrule that judgment. The CLI
and deep mode keep the ten-second timeout and report provider failures, so
measurements never mistake keyword order for Jev's. Configure a client tool timeout
of 120 seconds when using deep mode; large repository scans can take longer.
Up to four searches run at once and further calls wait for a slot; they share one
workspace snapshot, so parallel searches do not repeat a scan. Cancellation
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
Normal/deep searches send selected snippets to TypeSafe, as described in the [privacy overview](../README.md#privacy).

The server advertises when to use Oko, but connecting it does not force the agent
to choose it over native search. Codex project setup is available above. Release
packages are tested with the CLI and stdio MCP protocol on each CI target.
Automated Rust tests cover actual stdio messages and mock Jev
requests without real credentials.

## Names

Agents ask for code by name far more often than by behaviour, and a
one-line definition can lose a keyword shortlist to files that repeat its
name. Oko indexes every definition of the languages it parses (JavaScript,
TypeScript, Python, Go, Rust, Ruby, Java and Kotlin: functions, classes, methods, types,
modules and constants, with their qualified names such as `Server.handle`,
`Context.Next`, `Searcher.new`, `Discourse.Upload.url` or `Javalin.start`) and
looks the question's identifier-shaped words up in that index: qualified
names, `snake_case`, `camelCase`, `PascalCase`, anything in backticks, and a
Capitalized word beside a code noun ("Upload model"). Up to three such
definitions lead the shortlist as whole-definition chunks, so the ranker
judges them; one it rejects is still shown, labelled `exact name match`, after
the ranker's first choice. A pinned definition alone is an answer, so "No
relevant code found" never appears while a named definition exists.

Same-named definitions are chosen by the question's own hints (a container or
a path segment named in it), then non-test over test, exported over not,
shorter path. A name defined five or more times outside tests (`render`,
`Page`) needs such a hint; otherwise a note says how many places define it.
Other definitions of a pinned name are listed in a note. The metrics record
the identifiers found, the pins and the notes under `floor`.

## Long prompts

A prompt of 25 words or more (Codex and OpenCode send the task text rather
than a question) gets a second, focused query beside the raw one: its
identifiers, quoted literals and first sentence, with constraint clauses
("must remain unchanged", "do not touch the tests") dropped. The two keyword
rankings are fused by reciprocal rank, the raw order breaking ties, so
nothing the raw question found is lost; Jev still judges against the raw
text, which is where the exclusions belong. The metrics record the focused
terms and how many candidates they added under `focused`.

## Several questions in one call

`questions` takes two to eight independent questions. Each gets its own
shortlist, floor and ranking at the same time, on its own thread; only the
first question sends the requests judged beside its shortlist and the
recovery call, so a four-question call costs about six Jev requests where
four separate calls cost twelve. The answer is one packet: winners are taken
one per question before any question's second, every excerpt is labelled
with the question it answers (`Q2`; `Q1+Q3` when two questions led to the
same code), a duplicate is shown once, and a question that found nothing is
named in a note. The excerpt cap grows by two per extra question (up to
twelve) and the response cap by 5,000 bytes (up to 36,000). The first
question leads the notes, the floor and the metrics; `retrieval.questions`
records each question's results, pins, Jev calls and timing.

Shapes that used to be refused are answered as meant: a `question` beside
`questions` joins them, one entry in `questions` is a plain question, more
than eight keep the first eight with a note, `symbols` beside `questions`
adds those definitions (labelled `symbols`), `mode: usages` or `enumerate`
beside `questions` gives every question that names a definition its
listing, and unknown fields (`max_results`, `limit`) are ignored. Only `deep`
with `questions` is still an error.

## Names as a parameter

`symbols` takes exact names, comma-separated, up to twelve ("wsgi_app,
Flask.dispatch_request, Upload"), and returns their definitions whole, in
that order, with no ranking and no Jev call: a function or short class as
its full text, a long class as an outline. A name the index does not define
is reported in a note (`` `Nope`: no definition in the index. ``) while the
others answer. `question` may be omitted when `symbols` is given. The
parameter is a string rather than an array because an array costs an
`anyOf` in every turn's schema and Codex sends arrays as strings anyway.

`mode` turns the answer into a listing for the named definition (from
`symbols` or the question): `usages` is every use by line, as for a callers
question; `enumerate` is every file that uses it, one row per file with the
count and first line, up to 40 files, test files counted separately.

When a question names a definition that four or more code files use, the
answer carries one line summarising the dependents, most uses first, so
"what depends on X" needs no second call:

```
`Upload` is used by 116 files (301 uses): lib/file_store/to_s3_migration.rb:23 (16), lib/file_store/s3_store.rb:28 (11), …, +106 more; 86 test files. Ask "who uses Upload" for every file with path:line and the enclosing definition.
```

## Callers and tests

A question that asks who uses a name ("who calls `wsgi_app`", "callers of
Context.Next", "where is X used"), or a search with `intent: callers`, is
answered with a listing instead of ranked excerpts: every whole-word use of
the name across the index, each attributed to the definition it sits in and
classified as a call, import or reference, grouped by file with the
definition's own file first, up to 40 rows in 12 files and a count of the
rest. Hits in comments, tests and documentation files are counted, not
listed. No ranker runs, so the answer takes a few milliseconds. `oko ask
--intent callers` does the same on the command line.

```
Callers of Context.Next — 7 calls; 2 in comments, 37 in tests hidden
Defined at context.go:188

gin.go
  722	call	Engine.handleHTTPRequest	c.Next()
  766	call	serveError	c.Next()
```

A callers listing of more than twelve files, and `mode: enumerate`, switch
to the dependents shape: every production file that uses the name, one row
per enclosing definition as `path:line`, the definition's qualified name and
the line, grouped by area (`app/controllers (5 files)`) with counts. Nothing
is dropped for having few uses: over the budget (14 KB, or 8 KB inside a
several-question answer), extra rows within a file go first, then the line
text, then the definition name, leaving a bare `path:line` per file (a few
hundred files fit); only past that are whole areas summarised by name with a
hint to pass `directory`. Task, data, locale and script files are counted after the
code, the definition's own file is counted, and a closing section lists the
specs and tests that use the name as `path:line`, files named after the
definition first, each pointing at the file's `describe` (or test class,
`func Test…`, `def test_…`) of the name when it has one. For a Ruby class, lines that refer to it the Rails way
without spelling its constant count as uses and carry the rule that matched:
`belongs_to :upload`, `has_one :upload`, `has_many :uploads`,
`has_and_belongs_to_many :uploads` and `class_name: "Upload"`, derived with
ActiveRecord's own inflection rules (`OptimizedImage` → `optimized_image`,
`optimized_images`). The foreign key (`upload_id`) is not a use: it names a
column in serializers and params far more often than a dependency.

```
Files using Upload — 106 files, 285 uses in code; 16 in its own file; 86 test files (415 uses). One row per enclosing definition: path:line, definition, line.
app/controllers (5 files)
  app/controllers/metadata_controller.rb:118	MetadataController.default_manifest	upload = Upload.find_by(sha1: Upload.extract_sha1(image))
  app/controllers/uploads_controller.rb:89	UploadsController.create	render json: …  (+4 more in this file)
…
Specs and tests using Upload (86 files, 415 uses), named after it first:
  spec/models/upload_spec.rb:12	61
```

A question that asks for tests ("tests for `MultiDecoder`", "which specs
cover Upload") first lists the test files named after the definition's file
or name (`high`) or mentioning it (`medium`), each as `path:line` of its
first mention with the enclosing test function, and then the ranked code as
usual. The target of either question may be a plain word the index defines,
not only an identifier-shaped one.

## Unused definitions

A question that asks what nothing uses ("dead code in the gin package",
"unused helpers", "functions that are never called"), or a search with
`mode: unused`, is answered with a deletion-candidate listing: every
definition of the searched directory whose name appears nowhere in non-test
code of the workspace outside the definition itself. Private names with no
use anywhere come first, then private names only tests use (dead in
production, with up to three test locations as evidence), then exported
names in the same two groups (other repositories may use them; the
test-only ones are capped at twelve). Each row is `path:line`, kind,
qualified name and its evidence. Pass names in `symbols` to check only
those. The check is by name over the indexed code, so a method that
implements an interface, a name reached through reflection, a route or a
template, and public API can look unused; the answer says so. A short
question gets the listing alone; a longer one that mentions dead code among
other things gets it as a note above the ranked code.

```
Unused in production code under the workspace — 3 of 697 checked: 2 private, 1 exported; 2 of them are used by tests only. A row is a deletion candidate with its evidence.
private, used by tests only (2):
  utils.go:23	constant localhostIP — tests only: context_test.go:1169, context_test.go:1171, context_test.go:1175, +5 more
  utils.go:26	constant localhostIPv6 — tests only: context_test.go:2016
exported, no use in this repository (other repositories may use them) (1):
  errors.go:18	constant ErrorTypeRender — no reference anywhere in the workspace
```

## Coverage

The first line of every answer says what was searched:

```
Index: 25,873 of 26,687 files (814 skipped: 17 over size, 797 unreadable), 20,250 parsed for symbols (js, tsx, ts, jsx), watched
```

Files are discovered with `rg --files`. Text files up to 256 KiB are indexed
whole; files of a parsed language up to 1 MiB are indexed by their
definitions only (a generated table gets no chunks; a minified file is
skipped); binary, non-UTF-8, blank and larger files are skipped and counted.
`watched` means the index is kept current by the file watcher, `rescanned`
that this search re-validated it, `built now` that it was just created. When a
word of the question is the file stem of a skipped file, that file is named
with its size. The counts are recorded under `coverage` in the metrics.
