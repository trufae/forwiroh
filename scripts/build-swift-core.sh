#!/bin/bash
# Maintainer/CI build only. SwiftPM consumers never execute this script.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
profile="${FORWIROH_PROFILE:-release}"
export MACOSX_DEPLOYMENT_TARGET=14.0 IPHONEOS_DEPLOYMENT_TARGET=17.0
if [ "$#" = 0 ]; then
    echo "Usage: $0 RUST_TARGET [RUST_TARGET ...]" >&2
    exit 1
fi
for target in "$@"; do
    (
        unset SDKROOT CFLAGS
        case "$target" in
            *-apple-darwin)
                strip_debug=(xcrun strip -S -x)
                export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
                export CFLAGS="-mmacosx-version-min=14.0"
                ;;
            aarch64-apple-ios)
                strip_debug=(xcrun strip -S -x)
                export SDKROOT="$(xcrun --sdk iphoneos --show-sdk-path)"
                ;;
            aarch64-apple-ios-sim|x86_64-apple-ios)
                strip_debug=(xcrun strip -S -x)
                export SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)"
                ;;
            aarch64-linux-android|x86_64-linux-android)
                : "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to an installed Android NDK}"
                case "$(uname -s)" in
                    Darwin) host=darwin-x86_64 ;;
                    Linux) host=linux-x86_64 ;;
                    *) echo "Unsupported NDK build host" >&2; exit 1 ;;
                esac
                bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host/bin"
                strip_debug=("$bin/llvm-strip" --strip-debug)
                linker="$bin/${target}28-clang"
                key=$(printf '%s' "$target" | tr '[:lower:]-' '[:upper:]_')
                export "CARGO_TARGET_${key}_LINKER=$linker"
                cc_key=${target//-/_}
                export "CC_${cc_key}=$linker" "AR_${cc_key}=$bin/llvm-ar"
                ;;
            *-unknown-linux-gnu) strip_debug=("${STRIP:-strip}" --strip-debug) ;;
            *-unknown-linux-musl)
                strip_debug=("${STRIP:-strip}" --strip-debug)
                export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=-crt-static -C relocation-model=pic"
                key=$(printf '%s' "$target" | tr '[:lower:]-' '[:upper:]_')
                linker_key="CARGO_TARGET_${key}_LINKER"
                cc_key="CC_${target//-/_}"
                if [ -z "${!linker_key:-}" ]; then
                    command -v zig >/dev/null || { echo "Install Zig for musl linking, or set $linker_key" >&2; exit 1; }
                    export FORWIROH_MUSL_TARGET="${target/-unknown/}"
                    export "$linker_key=$PWD/scripts/zig-musl-linker.sh"
                fi
                if [ -z "${!cc_key:-}" ]; then export "$cc_key=musl-gcc"; fi
                ;;
            *) echo "Unsupported Rust target: $target" >&2; exit 1 ;;
        esac
        # Request only staticlib: also emitting rlib defers cross-crate LTO to
        # the consumer, leaving LLVM bitcode and unoptimized code in the archive.
        cargo rustc --locked --profile "$profile" -p forwiroh --lib --crate-type staticlib --target "$target"
        mkdir -p "dist/libraries/$target"
        library="dist/libraries/$target/libforwiroh.a"
        cp "$CARGO_TARGET_DIR/$target/$profile/libforwiroh.a" "$library"
        # Rust's prebuilt standard library can still contain DWARF. Preserve
        # external symbols and relocations needed by the consuming linker.
        "${strip_debug[@]}" "$library"
    )
done
