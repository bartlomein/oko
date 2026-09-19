# Installation and upgrades

[← Back to Oko](../README.md)

For prebuilt downloads, follow the [README installation steps](../README.md#install).
Each archive includes Oko, ripgrep (`rg`), and license notices. Keep the two
executables together; Oko uses its companion `rg` before searching PATH.

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

Download and verify the new archive, then repeat the installation steps with its
filename. For a source installation, update your checkout and rerun `cargo install`.

Codex setup keeps its own stable copy: run the newly installed `oko setup` in each
configured project to update it. Start a new agent session so it launches the
updated server. If your client keeps an old server running, reconnect its MCP
connection or restart the client. Saved credentials are separate from the binary.

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

Codex setup copies Oko and ripgrep to a stable user directory, so its connection
survives deleting the download. Setup does not add an `oko` command to PATH.
Manual client configurations must point to a permanent executable location.

## Platform support

Release CI builds and tests macOS Apple Silicon, macOS Intel, Linux x64, and Linux
ARM64. The Linux binaries require glibc 2.35+; they are not Alpine/musl binaries.
Windows downloads are not provided. See [release details](releasing.md) for the
exact runner baselines and package checks.
