# Contributing to Oko

Thanks for helping. Bug reports and ideas go in
[issues](https://github.com/bartlomein/oko/issues): include your OS,
`oko --version` and steps to reproduce, without API keys or private source code.

## Build and test

Build from source with the Rust toolchain pinned in `rust-toolchain.toml`, then
run the same checks as CI:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The tests need no API key or private repository.

## Benchmarks

Live benchmarks are optional and use your own model and TypeSafe accounts.
[Development and benchmarks](docs/benchmarks.md) explains the harnesses;
[benchmark results](docs/benchmark-results.md) has every published run.

The README's chart and results table come from one data file: edit
`docs/images/agent-sessions.json`, then run
`python3 scripts/render-readme-charts.py` and paste the printed table into the
README.

## Releases

[Releasing](docs/releasing.md) covers versions, tags and the draft-release
workflow.
