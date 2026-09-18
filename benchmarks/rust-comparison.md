# Rust rewrite comparison — 2026-09-18

Branch: `codex/rust-rewrite`. TypeScript reference: `933eabdb6dfdaef148790bc9a3147e0721f76841`.
Measured on macOS ARM64, using a Rust release build and Node 22.14.0.
Linux runtime behavior and performance have not been measured.

## Local speed

| Workload | TypeScript median | Rust median | Interpretation |
|---|---:|---:|---|
| Repository lexical search | 699.54 ms | 695.09 ms | Essentially unchanged |
| Supplied-item input-order CLI | 39.81 ms | 4.77 ms | 8.35x faster; about 35 ms saved |

Each workload has five questions, one warmup and five measured fresh-process runs
per question per runtime (25 measured samples per runtime). Execution order
alternates. Builds are excluded; filesystem caches are warm. No network requests
are included. Generic input-order timing is startup/JSON overhead, not AI ranking.
Exact JSON output matched on every run. Repository and implementation snapshots
were unchanged during measurement.

A direct Rust port did not meaningfully improve code-search latency on this
repository. Further search-speed work needs profiling and algorithm/allocation
improvements; a language change alone is not evidence of a speedup.

## Correctness and live accuracy

- 30 Rust tests pass, including actual CLI execution against local mock HTTP.
- 29 preserved TypeScript tests pass.
- 55 differential checks pass, including 7,552 stemmer inputs, complete chunks
  and shortlists, payload sizes, canned provider responses, dotenv parsing,
  Unicode ordering, and filesystem/input validation.
- Live code benchmark: 4/5 correct first, 5/5 correct in the top five, matching
  the saved TypeScript baseline. Lexical-only remains 1/5 first and 3/5 top five.
- Live synthetic ticket benchmark: 5/5 correct, including abstaining on no match.

The Rust live code median was 1.686 seconds versus 1.151 seconds in the earlier
TypeScript run. Those were separate five-request runs, not an interleaved
controlled comparison, so the difference cannot be attributed to the language.
Provider latency varies. The historical TypeScript ticket benchmark timed API
ranking only; the Rust ticket benchmark times the whole CLI, so those timings
are not comparable. These fixtures are smoke tests, not general accuracy proof.

## Reproduce

From the repository root:

```sh
npm --prefix reference/typescript ci
npm --prefix reference/typescript test
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
node scripts/parity-runtimes.mjs
node scripts/compare-runtimes.mjs /path/to/telemetry-studio
target/release/oko benchmark --repo /path/to/telemetry-studio
target/release/oko benchmark-items
```

The last two commands each make five paid Jev requests and load the API key from
the current directory's `.env` or the environment. Code fixture source hashes
must match before requests run. Add `--no-jev` for offline native benchmarks.

Local detailed reports (generated and ignored by Git):

- `benchmarks/results/runtimes-2026-09-18T11-34-00.815Z/report.json`
- `benchmarks/results/parity-2026-09-18T11-33-58.250Z/report.json`
- `benchmarks/results/2026-09-18T11-35-19.432Z/report.json`
- `benchmarks/results/items-2026-09-18T11-35-41.149Z/report.json`

The production CLI/library and benchmark commands are Rust. JavaScript remains
only in the preserved reference and developer comparison harness. Install
`--bin oko` for normal use; `oko-parity` is a development diagnostic. No Node.js
runtime is required by the application. Code discovery still requires ripgrep.
