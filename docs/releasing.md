# Building and releasing Oko

CI runs formatting, Clippy, the Rust suite, release compilation, archive creation,
and package smoke tests on four native GitHub runners. No TypeSafe, OpenAI or
Anthropic credentials are required. The single interactive OS credential-store
test remains ignored; CI does not prove a real user's Keychain/Secret Service
login or a coding agent's choice to use Oko.

## Try a candidate before tagging

Push a branch and open a pull request, or run the **CI** workflow manually on
the desired branch. Download the four `oko-<target>` workflow artifacts after
all jobs pass. Each contains a versioned `.tar.gz` archive and its `.sha256`
sidecar. This does not create or publish a GitHub release.

Build one package locally on its native platform:

```sh
cargo build --release --locked --target aarch64-apple-darwin --bin oko
python3 scripts/release.py package --target aarch64-apple-darwin --output target/release-bundles
python3 scripts/smoke-release.py target/release-bundles/oko-v0.2.0-aarch64-apple-darwin.tar.gz
```

Change the target and version for your platform. Maintainer scripts need Python
3.10+, curl and the pinned Rust toolchain; users of the resulting archives do not.
The package smoke test uses a temporary project and installation directory with
spaces, an empty PATH, fake credentials and a loopback mock provider. It checks
the checksum, direct search with bundled ripgrep, setup, repeat setup, removal
of the original download, MCP initialization/tool discovery/search, and normal
ranking using a project `.env`. It never touches the user's installed Oko or key.

## Prepare a draft release

1. Commit the reviewed code and version in `Cargo.toml`/`Cargo.lock`; ensure CI passes.
2. Push a version tag matching Cargo exactly, for example `v0.2.0`.
3. Wait for **Draft release** to finish all four native checks and package tests.
4. Review the draft's four archives, `SHA256SUMS`, and generated release notes.
5. Download and try the candidate with a coding client, then explicitly publish it.

Tag mismatch fails before compilation. The workflow creates a **draft**, never
an automatically public release, and refuses to overwrite an existing release.
Only the final draft-creation job has `contents: write`; build jobs are read-only,
checkout credentials are not retained, and third-party actions use commit SHAs.
The release job verifies all four checksum sidecars before upload. Keep a draft
until all assets are present, which also fits GitHub's immutable-release flow.
See [GitHub's immutable releases documentation](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).

## Platform and distribution limits

- Apple Silicon runs on `macos-14`; Intel runs on `macos-15-intel`.
- Linux x64 and ARM64 use Ubuntu 22.04 native runners. The Oko binaries target
  glibc 2.35+; the bundled ripgrep uses the upstream musl build. These archives
  do not promise compatibility with Alpine or older glibc distributions.
- macOS executables are not Developer ID signed/notarized. Gatekeeper approval
  may be necessary after a browser download; signing is a separate release step.
- Windows is not part of this initial binary-release matrix.

## Updating pinned inputs

`rust-toolchain.toml` pins the compiler/components used for both local development
and CI. `scripts/release/ripgrep.json` pins the upstream ripgrep version and each
archive's SHA-256, obtained from its official GitHub release assets. Update all
four hashes together and verify them against upstream before committing.

Packaging includes Oko's license/notices, upstream ripgrep license/COPYING files,
and license files for dependencies reachable in the target's Cargo resolve graph
(including build dependencies). Some crates omit their license text from the
published crate; `scripts/release/license-sources.json` pins those texts to exact
upstream commits and hashes. Missing notices or a changed download hash fail
packaging instead of silently shipping without them. Review attribution whenever
dependencies change. Dependency identifiers and SPDX expressions are listed in
`licenses/dependencies.json`; local registry paths are not included.

`BUILD.json` records the version, target, compiler, source commit, dirty status
and ripgrep version. Checksums detect corruption; they are not a substitute for
an independently authenticated signature. No signing keys are required by these
workflows, and they do not change repository visibility.
