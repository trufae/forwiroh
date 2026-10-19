#!/bin/sh
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
# Xcode supplies these; standalone builds default to an arm64 iPhone library.
SRCROOT="${SRCROOT:-$(CDPATH= cd "$(dirname "$0")" && pwd)}"
PLATFORM_NAME="${PLATFORM_NAME:-iphoneos}"
ARCHS="${ARCHS:-arm64}"
DERIVED_FILE_DIR="${DERIVED_FILE_DIR:-$SRCROOT/Frameworks}"
cd "$SRCROOT/../.."
arch="${CURRENT_ARCH:-undefined_arch}"
if [ "$arch" = undefined_arch ]; then set -- $ARCHS; arch="$1"; fi
case "$PLATFORM_NAME:$arch" in
  iphoneos:arm64) target=aarch64-apple-ios ;;
  iphonesimulator:arm64) target=aarch64-apple-ios-sim ;;
  iphonesimulator:x86_64) target=x86_64-apple-ios ;;
  *) echo "Unsupported Rust Apple target: $PLATFORM_NAME:$arch" >&2; exit 1 ;;
esac
# Xcode's SDKROOT is correct for C dependencies (ring) on this destination.
SDKROOT="${SDKROOT:-$(xcrun --sdk "$PLATFORM_NAME" --show-sdk-path)}"
export SDKROOT
# Avoid inheriting simulator deployment flags into host build scripts.
export IPHONEOS_DEPLOYMENT_TARGET=17.0
cargo build --locked --release -p forwiroh --target "$target"
mkdir -p "$DERIVED_FILE_DIR"
cp "target/$target/release/libforwiroh.a" "$DERIVED_FILE_DIR/libforwiroh.a"
echo "Built $target: $DERIVED_FILE_DIR/libforwiroh.a"
