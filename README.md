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

Run this on macOS or Linux once the release is public:

```sh
curl -fsSL https://raw.githubusercontent.com/bartlomein/oko/main/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

The installer detects your processor, downloads Oko and its bundled ripgrep,
verifies the checksum, and installs them without administrator access. Add the
`export PATH` line to `~/.zshrc` or `~/.bashrc` to keep `oko` available in new
terminals. Existing installations from other sources are left untouched.

Supported downloads: macOS Apple Silicon and Intel; Linux ARM64 and x64 with
glibc 2.35+ (Ubuntu 22.04 or newer). Alpine and Windows are not supported by the
installer. macOS binaries are not yet signed or notarized.

Prefer to inspect the script, install manually, or select a version?
See [installation options](docs/installation.md).

For Codex, continue with `oko setup` below—it prompts for your
[TypeSafe AI](https://typesafe.ai/) key. For terminal use or other coding tools,
save the key with `oko auth login`. Oko uses your operating system’s credential
store; headless Linux can use an environment variable or ignored `.env` instead.
See [key configuration](docs/configuration.md).

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
