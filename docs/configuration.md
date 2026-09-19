# Configuration

[← Back to Oko](../README.md)

## Configuration

Manage your saved TypeSafe AI key:

```sh
oko auth login   # Save a key, or replace the saved key
oko auth status  # Show whether a key is configured and its active source
oko auth logout  # Delete the saved key from this computer
```

Login uses hidden terminal input; do not pass keys as command arguments. Status
never displays the key. Login and status do not validate it with TypeSafe; the
next Jev request checks whether it works. Logout deletes the local saved copy,
not the key at TypeSafe. Revoke a key through TypeSafe when needed.

Saved keys use macOS Keychain, Windows Credential Manager, or Linux Secret
Service. Linux needs an available, unlocked Secret Service and session D-Bus.
If secure storage is unavailable, Oko reports an error and never saves a
plaintext fallback. For headless servers and CI, supply `TYPESAFE_API_KEY` through
the environment or an ignored `.env` instead.

Credential priority is **shell environment > current directory `.env` > saved
OS credential**. An explicitly empty environment or `.env` value disables
lower-priority credentials. Login/logout do not edit these overrides; remove
them yourself when switching to a saved key. `--no-jev` bypasses credential
loading entirely.

For project-specific configuration, copy `.env.example` to `.env` and set
`TYPESAFE_API_KEY`. Values are literal, without `$VARIABLE` expansion. Oko only
loads `.env` from the directory you run it from, not its installation folder.
This repository ignores `.env` and `.env.*` except `.env.example`; ensure your
other projects ignore their `.env` files too.

`TYPESAFE_BASE_URL` and `TYPESAFE_DEFAULT_MODEL` may be set in the process
environment. Their defaults are `https://api.typesafe.ai` and `jev-latest`.
These two settings are not read from `.env`.

Code search automatically caches prepared search data in the operating system's
user-cache directory, outside the repository. The first search builds the cache;
later CLI invocations and MCP searches reuse it. MCP also retains the latest
search scope in memory. Every search still discovers files and checks their
contents, so edits, deletions, ignored files, and branch changes refresh the
results. Different search directories have separate cache identities.
Larger scopes use up to four file readers. Unchanged files reuse cached chunk
boundaries, and an unchanged workspace also reuses corpus ranking statistics.
File preparation also uses up to four workers for larger scopes. JavaScript and
TypeScript syntax facts are cached per file; edits invalidate those facts, while
relationships are resolved against the current snapshot. Fresh-process
disk loading runs alongside the source scan; memory reuse skips disk loading.
Returned source is reconstructed from freshly read bytes, with content hashes
and cache validation checked before reuse. Cache format upgrades rebuild once.
The default location is `~/Library/Caches/oko/search` on macOS,
`$XDG_CACHE_HOME/oko/search` (or `~/.cache/oko/search`) on Linux, and
`%LOCALAPPDATA%\oko\search` on Windows.

Set `OKO_CACHE_DIR` in the process environment to choose the cache directory, or
`OKO_NO_CACHE=1` to bypass memory and disk reuse. These settings are not read from
`.env`. An override inside the searched directory uses memory reuse only, so Oko
does not add cache files to the repository it is searching. Cache files contain
source-derived search features and identifiers; keep the directory private.
They are disposable: missing, incompatible, damaged, or
unwritable caches fall back to preparing current files. Caching does not change
ranking or remove Jev requests.

For a local release build instead of installation, run
`cargo build --release --locked --bin oko`. The executable is
`target/release/oko` (`oko.exe` on Windows). Build separately for each operating
system and architecture. Code search uses `OKO_RIPGREP`, then an `rg` binary
beside Oko, then `rg` on PATH; supplied-item
ranking does not. Node.js is only needed for optional developer benchmark runners.
