#!/bin/sh
set -eu
mac_root=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$mac_root/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET=14.0
export CFLAGS="${CFLAGS:-} -mmacosx-version-min=14.0"
cd "$repo_root"
cargo rustc --locked --release -p forwiroh --lib --crate-type staticlib
app="$mac_root/build/Forwiroh.app"
mkdir -p "$app/Contents/MacOS"
cp "$mac_root/Forwiroh/Info.plist" "$app/Contents/Info.plist"
swiftc -swift-version 6 -Osize -whole-module-optimization -parse-as-library \
  -target "$(uname -m)-apple-macosx14.0" \
  -sdk "$(xcrun --sdk macosx --show-sdk-path)" \
  -import-objc-header "$repo_root/crates/forwiroh/include/forwiroh.h" \
  "$mac_root"/Forwiroh/*.swift "$repo_root/target/release/libforwiroh.a" \
  -framework SwiftUI -framework AppKit -framework Network -framework Security \
  -framework SystemConfiguration -framework CoreFoundation -lresolv -liconv \
  -Xlinker -dead_strip -o "$app/Contents/MacOS/Forwiroh"
xcrun strip -S -x "$app/Contents/MacOS/Forwiroh"
codesign --force --sign - --identifier org.radare.forwiroh.macos "$app"
echo "Built $app"
