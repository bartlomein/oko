# Oko

**Find the code you need by asking a question.**

Oko searches your project and returns relevant files, line numbers, and source
snippets. Use it in your terminal or connect it to Codex, Claude Code, or OpenCode
through MCP. It runs locally as a Rust binary and uses [TypeSafe AI’s Jev](https://typesafe.ai/)
to rank the results.

[Install](#install) · [Connect your coding tool](#connect-your-coding-tool) · [CLI examples](#use-in-your-terminal) · [Documentation](#documentation)

```sh
oko ask "where is authentication handled?"
```

No Node.js or Rust runtime needed for downloaded binaries. Oko needs a TypeSafe AI
API key for Jev ranking; local keyword search also works without a key.

## Install

> **Release status:** `v0.2.0` is a draft prerelease. Public downloads will be
> available once it is published. Until then, use [build from source](docs/installation.md#build-from-source).

### 1. Download Oko

Get your archive and `SHA256SUMS` from [GitHub Releases](https://github.com/bartlomein/oko/releases).

| Your computer | Archive |
| --- | --- |
| Mac — Apple Silicon (M1 or newer) | `oko-v0.2.0-aarch64-apple-darwin.tar.gz` |
| Mac — Intel | `oko-v0.2.0-x86_64-apple-darwin.tar.gz` |
| Linux — x64 | `oko-v0.2.0-x86_64-unknown-linux-gnu.tar.gz` |
| Linux — ARM64 | `oko-v0.2.0-aarch64-unknown-linux-gnu.tar.gz` |

macOS builds are tested on macOS 14 (Apple Silicon) and 15 (Intel). Linux needs
glibc 2.35+ (Ubuntu 22.04 or newer); Alpine and Windows downloads are not available.

### 2. Extract and install

Open a terminal in the folder containing both downloads. Set `archive` to the
filename you chose above, then paste:

```sh
archive=oko-v0.2.0-aarch64-apple-darwin.tar.gz

grep "  ${archive}$" SHA256SUMS | shasum -a 256 -c - &&
tar -xzf "$archive" &&
mkdir -p "$HOME/.local/bin" &&
cp "${archive%.tar.gz}/oko" "${archive%.tar.gz}/rg" "$HOME/.local/bin/" &&
export PATH="$HOME/.local/bin:$PATH" &&
oko --version
```

This installs Oko and its bundled search helper, `rg`, into `~/.local/bin`
(replacing existing copies there). Add `export PATH="$HOME/.local/bin:$PATH"`
to your shell configuration (`~/.zshrc` or `~/.bashrc`) to keep the command available
in new terminals. On Linux without `shasum`, use `sha256sum -c -` instead.

If macOS blocks the download, follow [Apple’s instructions to allow the app](https://support.apple.com/en-us/102445).
The binaries are not yet signed or notarized.

### 3. Add your key

Get an API key from [TypeSafe AI](https://typesafe.ai/), then run:

```sh
oko auth login
```

Paste your key at the hidden prompt. Oko saves it in your operating system’s
credential store. Oko itself does not need an OpenAI or Anthropic key; your coding
tool keeps its own model connection. For headless Linux, use an environment
variable or ignored `.env` instead: [key configuration](docs/configuration.md).

To try Oko without a key, skip login and use `--no-jev` in the examples below.

## Connect your coding tool

### Codex app or CLI

Run this from the project you want to search:

```sh
cd /path/to/your/project
oko setup
```

Setup installs stable copies, configures the project’s MCP connection, adds search
guidance, and checks the connection. Open that project in Codex, trust it if
prompted, and start a new session. Repeat setup for each project.

For a key-free connection, use `oko setup --no-jev`.

### Claude Code or OpenCode

Follow the short [Claude Code](docs/clients.md#claude-code) or
[OpenCode](docs/clients.md#opencode) setup instructions. These clients need a manual
MCP entry; `oko setup` currently configures Codex only.

Then ask your agent:

> Use Oko to find where authentication is handled in this project.

Your coding tool starts Oko automatically. You do not need to run a server yourself.
Connecting Oko makes it available; it does not force the agent to use it for every
search. Exact names and literal text can still be searched with native grep.

## Use in your terminal

Oko searches the directory you run it from:

```sh
cd /path/to/your/project
oko ask "where is authentication handled?"
oko ask "how are retries handled?" --json
oko ask "how does authentication work?" --deep --max-steps 5
oko ask "where is authentication handled?" --no-jev
```

Start with normal search. Deep mode can investigate further, but makes additional
Jev requests and can take longer. `--no-jev` uses local keyword ranking only.

To replace, check, or remove your saved key, use `oko auth login`,
`oko auth status`, or `oko auth logout`.

## Privacy

File discovery and initial search run locally. Jev ranking sends your question and
selected source snippets to TypeSafe AI. Deep mode can send more snippets across
multiple requests. **Use `--no-jev` for local-only searches.** Oko does not edit your
source files. Prepared search data is cached outside your repository;
[configuration](docs/configuration.md) explains key storage and cache controls.

## Documentation

| I want to… | Guide |
| --- | --- |
| Build from source, upgrade, or troubleshoot installation | [Installation](docs/installation.md) |
| Connect an agent or understand the MCP tool | [Client setup](docs/clients.md) · [MCP reference](docs/mcp.md) |
| Configure keys, `.env`, or caching | [Configuration](docs/configuration.md) |
| Use search options or rank my own documents and records | [Search](docs/search.md) · [Document ranking](docs/ranking.md) |
| Contribute, evaluate results, or prepare a release | [Development and benchmarks](docs/benchmarks.md) · [Releasing](docs/releasing.md) |

## Help and contributing

Found a bug or have an idea? [Open an issue](https://github.com/bartlomein/oko/issues).
Include your OS, `oko --version`, and steps to reproduce, without API keys or
private source code. For code changes, build from source and run:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The Rust tests need no API key or private repository. Live benchmarks are optional.

## License

[MIT](LICENSE). Dependency attributions are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
