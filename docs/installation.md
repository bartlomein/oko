# Installation and upgrades

[← Back to Oko](../README.md)

## Install script

Once the repository and release are public:

```sh
curl -fsSL https://raw.githubusercontent.com/bartlomein/oko/main/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

To inspect the script before running it:

```sh
curl -fsSL https://raw.githubusercontent.com/bartlomein/oko/main/install.sh -o install-oko.sh
less install-oko.sh
sh install-oko.sh
```

The installer defaults to `v0.4.0`, including when that version is published as a
prerelease. It does not rely on GitHub’s latest stable release endpoint. A draft
or private release is not anonymously downloadable.

To select a version or custom absolute installation paths, set these variables
on the `sh` process (not on `curl`):

```sh
curl -fsSL https://raw.githubusercontent.com/bartlomein/oko/main/install.sh |
  OKO_VERSION=v0.4.0 sh
```

| Variable | Default | Purpose |
| --- | --- | --- |
| `OKO_VERSION` | `v0.4.0` | Published release to download; the `v` prefix is optional. |
| `OKO_INSTALL_DIR` | `~/.local/share/oko` | Version directories containing binaries and license notices. |
| `OKO_BIN_DIR` | `~/.local/bin` | Directory containing the `oko` symlink. |

The installer checks HTTPS downloads against the release’s `SHA256SUMS`, checks
that the binaries run, then switches the `oko` symlink. Failed downloads or
validation leave the previous version active. Existing unmanaged executables or
symlinks are not overwritten: move your previous Oko installation yourself, or
choose another `OKO_BIN_DIR`. The bundled ripgrep remains beside Oko; your existing
`rg` command is unchanged. Previous successful installations remain under
`OKO_INSTALL_DIR/releases`.

The installer does not request your key, configure coding clients, or edit shell
startup files. It prints a PATH command if needed. Add that line to your shell
configuration and run it in your current terminal. No root access is required.

## Manual download

Download the matching archive and `SHA256SUMS` from
[GitHub Releases](https://github.com/bartlomein/oko/releases):

| Computer | Archive suffix |
| --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux x64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |

In the download directory, substitute your chosen filename:

```sh
archive=oko-v0.4.0-aarch64-apple-darwin.tar.gz
grep "  ${archive}$" SHA256SUMS | shasum -a 256 -c - &&
tar -xzf "$archive" &&
"./${archive%.tar.gz}/oko" --version
```

On Linux without `shasum`, use `sha256sum -c -`. Continue only after checksum
verification reports `OK`. Keep the extracted folder in a permanent location;
each archive includes Oko, its companion `rg`, and license notices. You can add
that folder to PATH or invoke Oko using its full path.

## Build from source

Install [Rust](https://rustup.rs/) and [ripgrep](https://github.com/BurntSushi/ripgrep#installation),
then run:

```sh
git clone https://github.com/bartlomein/oko.git
cd oko
cargo install --path . --bin oko --locked
oko --version
oko ask "where are search candidates ranked?" --no-jev
```

The repository pins its Rust toolchain in `rust-toolchain.toml`. If `oko` is not
found, ensure Cargo’s bin directory (`~/.cargo/bin` by default) is on PATH.
A private repository requires GitHub access; public cloning will work after the
repository is made public.

Next, run `oko auth login` to save your TypeSafe key, or keep using `--no-jev`.
Return to [connect your coding tool](../README.md#connect-your-coding-tool).

## Upgrade

Rerun the installer with `OKO_VERSION` set to the new published version. For a
manual installation, download and verify the new archive. For a source
installation, update your checkout and rerun `cargo install`.

Setup keeps its own stable copy: run the newly installed `oko setup` (with the same
`--client`) in each configured project to update it. Start a new agent session so it launches the
updated server. If your client keeps an old server running, reconnect its MCP
connection or restart the client. Saved credentials are separate from the binary.

### Upgrading to 0.4.0

Rerun setup in each connected project so it uses the new binary and the updated
search guidance: `oko setup` for Codex, `oko setup --client claude` for Claude
Code, `oko setup --client opencode` for OpenCode, or `--client all`. If you
connected Claude Code or OpenCode by hand, run setup once: it takes over the
existing Oko connection, leaves your other settings alone, and adds the guidance
that makes sessions faster in our [benchmark](../README.md#benchmarks). Nothing
else changes for existing users.

### Upgrading to 0.3.0

Coding agents need no changes: they read the search tool's text, which is now
smaller. The MCP `search` result is plain text and no longer includes
`structuredContent`, timings, or retrieval metadata. A script that calls the MCP
tool itself and parses that JSON must set `OKO_METRICS_FILE` and read the JSON
line Oko appends there for each search; see the
[MCP reference](mcp.md#timings-and-retrieval-metadata). `oko ask --json` is
unchanged. Rerun `oko setup` in each configured project so Codex uses the new
binary and the updated search guidance in `AGENTS.md`.

## Troubleshooting

| Problem | Fix |
| --- | --- |
| `oko: command not found` | Add the installation directory to PATH, or invoke the executable by its full path. |
| macOS blocks the executable | The builds are unsigned. Use [Apple’s per-app approval steps](https://support.apple.com/en-us/102445). |
| Missing TypeSafe key | Run `oko auth login`; check `oko auth status` for an overriding shell or `.env` value. |
| Credential store unavailable on Linux | Set `TYPESAFE_API_KEY` in the server environment or the project’s ignored `.env`. See [configuration](configuration.md). |
| Could not run `rg --files` | Keep the bundled `rg` beside Oko, or install ripgrep on PATH. Check whether `OKO_RIPGREP` overrides that location. |

## Without changing PATH

You can run the extracted executable directly:

```sh
/absolute/path/to/extracted/oko setup --root /absolute/path/to/project
```

Setup copies Oko and ripgrep to a stable user directory, so its connection
survives deleting the download. Setup does not add an `oko` command to PATH.
Manual client configurations must point to a permanent executable location.

## Platform support

Release CI builds and tests macOS Apple Silicon, macOS Intel, Linux x64, and Linux
ARM64. The Linux binaries require glibc 2.35+; they are not Alpine/musl binaries.
Windows downloads are not provided. See [release details](releasing.md) for the
exact runner baselines and package checks.
