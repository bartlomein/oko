#!/bin/sh
# Install a published Oko release. No root, Rust, or Node.js required.
# Keep all execution inside main: a truncated download must not partially install.
set -eu

fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }

main() {
    [ "$#" -eq 0 ] || fail "Use OKO_VERSION, OKO_INSTALL_DIR, or OKO_BIN_DIR environment variables; no arguments are accepted."
    version=${OKO_VERSION:-v0.2.0}
    case "$version" in v*) ;; *) version=v$version ;; esac
    printf '%s\n' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$' || fail 'Invalid OKO_VERSION.'
    [ -n "${HOME:-}" ] || fail 'HOME must be set.'
    install_dir=${OKO_INSTALL_DIR:-"$HOME/.local/share/oko"}
    bin_dir=${OKO_BIN_DIR:-"$HOME/.local/bin"}
    for path in "$install_dir" "$bin_dir"; do
        case "$path" in /*) ;; *) fail 'Installation directories must be absolute paths.' ;; esac
    done
    for tool in curl tar awk grep mktemp uname readlink; do
        command -v "$tool" >/dev/null 2>&1 || fail "Required command not found: $tool"
    done
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in arm64|aarch64) arch=aarch64 ;; x86_64|amd64) arch=x86_64 ;; *) fail "Unsupported processor: $arch" ;; esac
    case "$os" in
        Darwin) target=$arch-apple-darwin ;;
        Linux)
            libc=$(getconf GNU_LIBC_VERSION 2>/dev/null) || fail 'Linux requires glibc 2.35 or newer; Alpine/musl is unsupported.'
            printf '%s\n' "$libc" | awk '$1 == "glibc" {split($2,v,"."); if (v[1]>2 || (v[1]==2 && v[2]>=35)) exit 0} END {if (!v[1] || v[1]<2 || (v[1]==2 && v[2]<35)) exit 1}' || fail 'Linux requires glibc 2.35 or newer.'
            target=$arch-unknown-linux-gnu ;;
        *) fail "Unsupported operating system: $os (supported: macOS and Linux)." ;;
    esac
    if command -v sha256sum >/dev/null 2>&1; then
        checksum=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        checksum=shasum
    else
        fail 'Install sha256sum or shasum to verify downloads.'
    fi

    mkdir -p "$install_dir/releases" "$bin_dir"
    install_dir=$(cd "$install_dir" && pwd -P)
    bin_dir=$(cd "$bin_dir" && pwd -P)
    destination=$bin_dir/oko
    # Only replace links managed by this installer. Never overwrite another install.
    if [ -L "$destination" ]; then
        previous=$(readlink "$destination")
        case "$previous" in "$install_dir/releases/"*/oko) ;; *) fail "$destination belongs to another installation. Choose OKO_BIN_DIR or move it first." ;; esac
    elif [ -e "$destination" ]; then
        fail "$destination already exists. Choose OKO_BIN_DIR or move it first."
    fi

    work=$(mktemp -d "$install_dir/releases/$version.XXXXXXXX")
    link_dir=''
    installed=0
    trap 'if [ "$installed" = 0 ]; then rm -rf "$work"; fi; if [ -n "$link_dir" ]; then rm -rf "$link_dir"; fi' 0
    trap 'exit 1' 1 2 3 15
    archive=oko-$version-$target.tar.gz
    bundle=oko-$version-$target
    base=https://github.com/bartlomein/oko/releases/download/$version
    printf 'Downloading Oko %s for %s…\n' "$version" "$target"
    for file in "$archive" SHA256SUMS; do
        curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
            --connect-timeout 15 --max-time 180 --retry 2 \
            "$base/$file" --output "$work/$file" || fail "Cannot download $file. The release must be published and accessible."
    done
    expected=$(awk -v name="$archive" '$2 == name {print $1; count++} END {if (count != 1) exit 1}' "$work/SHA256SUMS") || fail 'Missing or duplicate archive checksum.'
    [ "${#expected}" -eq 64 ] || fail 'Invalid SHA-256 checksum.'
    case "$expected" in *[!0-9a-fA-F]*) fail 'Invalid SHA-256 checksum.' ;; esac
    if [ "$checksum" = sha256sum ]; then
        actual=$(sha256sum "$work/$archive" | awk '{print $1}')
    else
        actual=$(shasum -a 256 "$work/$archive" | awk '{print $1}')
    fi
    [ "$actual" = "$expected" ] || fail 'Archive checksum mismatch; installation unchanged.'

    # Reject paths outside the single release directory and all links/special files.
    tar -tzf "$work/$archive" > "$work/members" || fail 'Cannot read release archive.'
    while IFS= read -r member; do
        member=${member%/}
        case "$member" in "$bundle"|"$bundle/"*) ;; *) fail 'Unsafe archive path.' ;; esac
        case "/$member/" in */../*|*/./*|*//*) fail 'Unsafe archive path.' ;; esac
    done < "$work/members"
    tar -tvzf "$work/$archive" > "$work/types" || fail 'Cannot inspect release archive.'
    awk 'substr($0,1,1) != "-" && substr($0,1,1) != "d" {exit 1}' "$work/types" || fail 'Archive contains a link or special file.'
    tar -xzf "$work/$archive" -C "$work" || fail 'Cannot extract release archive.'
    [ -x "$work/$bundle/oko" ] && [ -x "$work/$bundle/rg" ] || fail 'Release is missing executable oko or rg.'
    reported=$("$work/$bundle/oko" --version) || fail 'Downloaded Oko cannot run on this system.'
    [ "$reported" = "oko ${version#v}" ] || fail 'Downloaded version does not match the requested release.'
    "$work/$bundle/rg" --version >/dev/null || fail 'Bundled ripgrep cannot run on this system.'
    rm -f "$work/$archive" "$work/SHA256SUMS" "$work/members" "$work/types"

    # Stage the symlink on the same filesystem; rename replaces it atomically.
    link_dir=$(mktemp -d "$bin_dir/.oko-link.XXXXXXXX")
    ln -s "$work/$bundle/oko" "$link_dir/oko"
    mv -f "$link_dir/oko" "$destination"
    installed=1
    printf 'Installed %s at %s\n' "$reported" "$destination"
    case ":${PATH:-}:" in
        *":$bin_dir:"*) ;;
        *)
            quoted=$(printf '%s' "$bin_dir" | sed "s/'/'\\\\''/g")
            printf "\nRun this in your terminal, and add it to your shell configuration:\nexport PATH='%s':\"\$PATH\"\n" "$quoted"
            ;;
    esac
    printf '\nNext, run inside your project: oko setup\nFor terminal search or other clients, save your key with: oko auth login\n'
}

main "$@"
