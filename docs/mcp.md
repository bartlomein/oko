# MCP reference

[← Back to Oko](../README.md)

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
Claude Code still require manual MCP configuration. For downloads, see the [installation guide](installation.md); see [release maintenance](releasing.md).

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
0.5 cutoff, as a focused window labelled `lower confidence` (`lowerConfidence`),
only beside at least one accepted match and only while the response is under
6,000 bytes. Nothing accepted remains an empty result. In the same replay 11 of
30 such excerpts held expected code that was otherwise missing; at 0.2 it would
have been 12 of 65. Together these two rules raised responses containing
everything the task needed from 67% to 84% for 12% more bytes, with no question
losing coverage.

After the excerpts, a normal search names up to six further candidates by
`path:start-end` only, under `Other candidates, not shown, best first:`. Jev
judges every shortlisted candidate in the same request, so these
cost no extra call and one line each; candidates Jev rated irrelevant (below
0.2) and anything overlapping a shown excerpt are left out. In keyword order the
heading is `Other keyword matches, not shown:`. An agent that needs more can
open one of these instead of starting a blind search. `retrieval.candidates` in
`OKO_METRICS_FILE` records every candidate's path, lines, and relevance, which
shows whether missed code was judged irrelevant, fell below the threshold, or
was never shortlisted.

When the line that best matches the question is in a comment directly above a
declaration in the winning chunk, the declaration is treated as the match: doc
comments often repeat the question better than the code they document.

When a ranked match lies in a function with a known boundary of at most 256
lines, Oko returns the full implementation instead of the usual 60-line source
window, for every match and not only the first: a window that stops a few lines
short of the relevant statement costs a follow-up read, or a wrong answer.
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

Normal mode uses one Jev ranking request for a nonempty shortlist, with an
independent relevance judgment for each candidate. Deep mode is
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
Normal/deep searches send selected snippets to TypeSafe, as described in the [privacy overview](../README.md#privacy).

The server advertises when to use Oko, but connecting it does not force the agent
to choose it over native search. Codex project setup is available above. Release
packages are tested with the CLI and stdio MCP protocol on each CI target.
Automated Rust tests cover actual stdio messages and mock Jev
requests without real credentials.
