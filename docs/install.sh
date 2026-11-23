#!/bin/sh
# Installs the opera command-line tool from its GitHub release, on Linux and macOS:
#
#     curl -LsSf https://opera-rs.opengeos.org/install.sh | sh
#
# Settings, from the environment:
#   OPERA_VERSION        the release to install (0.1.0 or v0.1.0); the latest by default
#   OPERA_INSTALL_DIR    where to put the binary; $XDG_BIN_HOME or ~/.local/bin by default
#   OPERA_DOWNLOAD_URL   the directory the archives are downloaded from, for a mirror; the
#                        GitHub release by default
#
# On Windows, use install.ps1. Elsewhere: pip install opera-rs, or cargo install opera-cli.

set -eu

REPO="opengeos/opera-rs"

say() {
    printf 'opera-installer: %s\n' "$1"
}

err() {
    say "error: $1" >&2
    exit 1
}

has() {
    command -v "$1" > /dev/null 2>&1
}

# download URL FILE: FILE "-" writes to standard output.
download() {
    if has curl; then
        if [ "$2" = "-" ]; then
            curl --proto-redir '=https' --tlsv1.2 -fsSL "$1"
        else
            curl --proto-redir '=https' --tlsv1.2 -fsSL "$1" -o "$2"
        fi
    elif has wget; then
        wget -q "$1" -O "$2"
    else
        err "neither curl nor wget is installed"
    fi
}

# The Rust target triple of the release binary for this machine.
target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) err "no opera binary for $arch; install it with pip install opera-rs or cargo install opera-cli" ;;
    esac
    case "$os" in
        Linux) echo "$arch-unknown-linux-musl" ;;
        Darwin)
            # An x86_64 shell under Rosetta on Apple silicon: install the native binary.
            if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2> /dev/null || true)" = 1 ]; then
                arch=aarch64
            fi
            echo "$arch-apple-darwin"
            ;;
        MINGW* | MSYS* | CYGWIN*)
            err "on Windows, run in PowerShell: irm https://opera-rs.opengeos.org/install.ps1 | iex"
            ;;
        *) err "no opera binary for $os; install it with pip install opera-rs or cargo install opera-cli" ;;
    esac
}

# The tag of the latest release: from where github.com/$REPO/releases/latest redirects with curl
# (not rate-limited as the GitHub API is), from the API with wget.
latest() {
    if has curl; then
        url=$(curl --proto-redir '=https' --tlsv1.2 -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") || return 1
        tag="${url##*/}"
    else
        json=$(download "https://api.github.com/repos/$REPO/releases/latest" -) || return 1
        tag=$(printf '%s\n' "$json" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
    fi
    case "$tag" in
        v[0-9]*) echo "$tag" ;;
        *) return 1 ;;
    esac
}

sha256() {
    if has sha256sum; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif has shasum; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        err "neither sha256sum nor shasum is installed, to verify the download"
    fi
}

main() {
    target=$(target)

    version="${OPERA_VERSION:-}"
    if [ -z "$version" ]; then
        version=$(latest) ||
            err "could not find the latest release of $REPO; set OPERA_VERSION to the version to install"
    fi
    version="v${version#v}"
    case "$version" in
        v[0-9]*) ;;
        *) err "invalid version: $version" ;;
    esac
    case "$version" in
        *[!0-9A-Za-z._+-]*) err "invalid version: $version" ;;
    esac

    install_dir="${OPERA_INSTALL_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
    base="${OPERA_DOWNLOAD_URL:-https://github.com/$REPO/releases/download/$version}"
    base="${base%/}"
    name="opera-cli-$version-$target"

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    say "downloading opera $version for $target"
    download "$base/$name.tar.gz" "$tmp/$name.tar.gz" ||
        err "could not download opera $version for $target; install it with pip install opera-rs or cargo install opera-cli"
    download "$base/$name.tar.gz.sha256" "$tmp/$name.tar.gz.sha256" ||
        err "could not download the checksum of $name.tar.gz"
    expected=$(cut -d ' ' -f 1 < "$tmp/$name.tar.gz.sha256")
    case "$expected" in
        *[!0-9a-fA-F]* | "") err "invalid checksum format in $name.tar.gz.sha256" ;;
    esac
    [ "${#expected}" -eq 64 ] || err "invalid checksum length in $name.tar.gz.sha256"
    actual=$(sha256 "$tmp/$name.tar.gz")
    [ "$(printf '%s' "$actual" | tr 'A-F' 'a-f')" = "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" ] ||
        err "checksum mismatch for $name.tar.gz"

    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    mkdir -p "$install_dir"
    stage="$install_dir/opera.tmp"
    cp "$tmp/$name/opera" "$stage"
    chmod 755 "$stage"
    if ! installed_version=$("$stage" --version 2> /dev/null); then
        rm -f "$stage"
        err "the downloaded opera binary does not run"
    fi
    mv -f "$stage" "$install_dir/opera"
    say "installed $installed_version in $install_dir"

    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *)
            say "$install_dir is not on your PATH; add it in your shell's startup file:"
            # $PATH is for the startup file to expand, not this script.
            # shellcheck disable=SC2016
            printf '\n    export PATH="%s:$PATH"\n\n' "$install_dir"
            ;;
    esac
}

main "$@"
