#!/bin/sh
# Install a checksum-verified CLI from one immutable GitHub release.
set -eu

main() {
    repo=${FORWIROH_REPOSITORY:-trufae/forwiroh}
    version=${FORWIROH_VERSION:-}
    command -v curl >/dev/null || { echo 'curl is required' >&2; exit 1; }
    case "$(uname -m)" in
        x86_64|amd64) arch=x86_64 ;;
        arm64|aarch64) arch=aarch64 ;;
        *) echo 'Unsupported CPU (available: x86_64 and arm64)' >&2; exit 1 ;;
    esac
    binary=forwiroh
    extension=tar.gz
    case "$(uname -s)" in
        Darwin) target=$arch-apple-darwin ;;
        Linux)
            if [ "$(uname -o 2>/dev/null || true)" = Android ] || [ -n "${ANDROID_ROOT:-}" ]; then
                target=$arch-linux-android
            else
                target=$arch-unknown-linux-musl
            fi
            ;;
        MINGW*|MSYS*|CYGWIN*) target=$arch-pc-windows-msvc; binary=forwiroh.exe; extension=zip ;;
        *) echo 'Unsupported OS (available: Linux, Android, macOS, Windows)' >&2; exit 1 ;;
    esac
    install_dir=${FORWIROH_INSTALL_DIR:-${HOME:?}/.local/bin}
    if [ -z "${FORWIROH_INSTALL_DIR:-}" ] && [ -n "${PREFIX:-}" ] && [ "${target#*-linux-android}" != "$target" ]; then
        install_dir=$PREFIX/bin
    fi
    if [ -z "$version" ]; then
        latest=$(curl -fsSLI --proto '=https' -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
        version=${latest##*/}
    fi
    expr "$version" : 'v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*$' >/dev/null || {
        echo "Invalid release tag: $version" >&2; exit 1;
    }
    base=https://github.com/$repo/releases/download/$version
    asset=forwiroh-$target.$extension
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' 0
    trap 'exit 1' HUP INT TERM
    curl -fsSL --proto '=https' "$base/CLI-SHA256SUMS" -o "$tmp/CLI-SHA256SUMS"
    expected=$(awk -v name="$asset" '$2 == name { print $1 }' "$tmp/CLI-SHA256SUMS")
    case "$expected" in ''|*[!0-9a-fA-F]*) echo "Missing/invalid checksum for $asset" >&2; exit 1 ;; esac
    [ "${#expected}" -eq 64 ] || { echo 'Invalid SHA-256 length' >&2; exit 1; }
    curl -fsSL --proto '=https' "$base/$asset" -o "$tmp/$asset"
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
    elif command -v shasum >/dev/null; then
        actual=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')
    else
        echo 'sha256sum or shasum is required' >&2; exit 1
    fi
    [ "$actual" = "$expected" ] || { echo 'SHA-256 mismatch; installation aborted' >&2; exit 1; }
    if [ "$extension" = zip ]; then
        unzip -p "$tmp/$asset" "$binary" > "$tmp/$binary"
    else
        tar -xzf "$tmp/$asset" -C "$tmp" "$binary"
    fi
    chmod 755 "$tmp/$binary"
    "$tmp/$binary" --version
    mkdir -p "$install_dir"
    # Stage in the destination filesystem, then atomically replace the executable.
    stage=$(mktemp "$install_dir/.forwiroh.XXXXXX")
    trap 'rm -rf "$tmp"; rm -f "$stage"' 0
    cp "$tmp/$binary" "$stage"
    chmod 755 "$stage"
    mv -f "$stage" "$install_dir/$binary"
    printf 'Installed %s to %s\n' "$version" "$install_dir/$binary"
    case ":$PATH:" in *":$install_dir:"*) ;; *) printf 'Add to PATH: export PATH="%s:$PATH"\n' "$install_dir" ;; esac
}

main "$@"
