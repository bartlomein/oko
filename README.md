# Oko

Oko is a Rust CLI and library for finding relevant code and ranking supplied text.
The application runs without Node.js. The original TypeScript implementation is
kept under `reference/typescript/` for behavior and performance comparisons.

See [the measured Rust comparison](benchmarks/rust-comparison.md): local code
search was effectively unchanged; supplied-item CLI overhead fell from about
40 ms to 5 ms, with matching behavior on the tested fixtures.

## Install and run

Build with a current stable Rust toolchain (edition 2024) and install
[ripgrep](https://github.com/BurntSushi/ripgrep) for code discovery:

```sh
cargo install --path . --bin oko --locked
oko --help
cd /path/to/your/project
oko ask "where is authentication handled?"
oko ask "where is authentication handled?" --json
oko ask "where is authentication handled?" --no-jev
```

For a local release build instead, run `cargo build --release --locked --bin oko`.
The executable is `target/release/oko` (`oko.exe` on Windows). Build for each
operating system and CPU architecture you distribute to. No JavaScript wrapper
is required. Code search requires `rg` on PATH; supplied-item ranking does not.

Put your TypeSafe AI API key in `.env` **in the directory where you run Oko**:

```dotenv
TYPESAFE_API_KEY=your-api-key
```

Use `.env.example` as a template. Existing shell variables take precedence,
including an explicitly empty value. Values are literal, without `$VARIABLE`
expansion. `.env` and `.env.*` are ignored by Git except `.env.example`.
When invoking Oko from another repository, put the key in that repository's
ignored `.env` or export `TYPESAFE_API_KEY` in your shell.

`TYPESAFE_BASE_URL` and `TYPESAFE_DEFAULT_MODEL` may be set in the process
environment. Their defaults are `https://api.typesafe.ai` and `jev-latest`.
As in the TypeScript implementation, these two settings are not read from `.env`.

## Code search

Oko discovers files with `rg --files`, reads UTF-8 text files up to 256 KiB,
rejects binary/invalid UTF-8 content, and builds a deterministic shortlist.
Search splits snake_case and camelCase names, ignores common question words,
and matches English word forms using the Porter stemmer.

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
diagnostics go to stderr. Neither changing language nor packaging changes the
ranking model: the Rust port preserves the TypeScript search and request logic.

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

Results preserve IDs and sources, and return up to five items whose scores beat
the `none` choice. `omittedCount` counts items dropped by the request byte budget,
not items outside the top five. Supply candidates in your existing search order.
`--no-jev` preserves that order with zero scores; it is an input-order baseline,
not keyword search. Scores are relative to the candidates, not universal
relevance probabilities. Oko does not fetch URLs or connect to databases.

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
a blocking worker. It does not load `.env` itself. Use `no_jev: true` to preserve
input order without a request. `parse_items` validates unknown JSON first.
The former JavaScript import API lives only in the TypeScript reference.

## Accuracy benchmarks

The application includes native Rust benchmark commands:

```sh
oko benchmark --repo /path/to/telemetry-studio --repeats 1
oko benchmark-items --repeats 1
```

Run these from Oko's directory to load its `.env` and save reports under
`benchmarks/results/` (ignored). Each command makes five Jev requests per repeat.
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

Add `--no-jev` to either benchmark for an offline baseline. Code benchmarks also
accept `--baseline /path/to/report.json` to rescore an earlier report; repository
commit, working-tree status, and expected-source hashes must match. Repeats are
limited to 1–10. Errors count as misses and produce a nonzero exit status.

## Compare Rust against TypeScript

Node.js 20+ is needed only for these developer comparison tools and the preserved
reference, not for the Rust application:

```sh
npm --prefix reference/typescript ci
npm --prefix reference/typescript test
cargo test --locked
cargo build --release --locked
node scripts/parity-runtimes.mjs
node scripts/compare-runtimes.mjs /path/to/telemetry-studio
```

The reference comes from commit `933eabdb6dfdaef148790bc9a3147e0721f76841`.
Parity tests compare chunks, ordering, scores, prepared Jev requests, byte budgets,
stemming, filesystem filtering, and CLI validation. The runtime comparison
performs one warmup and five measured runs per question per runtime, alternating
execution order. It verifies identical JSON output and unchanged source/binary
snapshots. Both reports are saved under `benchmarks/results/`.

Runtime comparisons disable Jev and incur no API charges. Code runs measure
local lexical search; item runs measure startup/JSON/input-order overhead, not
AI ranking. Builds are excluded. Filesystem caches are warm and these are local
machine measurements, not Linux-server measurements or cold-disk benchmarks.
Live provider latency must be measured separately.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

`oko-parity` is a development diagnostic binary and never calls Jev. Install only
`--bin oko` for ordinary use. Third-party parser/stemmer notices are in
`THIRD_PARTY_NOTICES.md`.
