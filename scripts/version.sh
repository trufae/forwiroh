#!/bin/sh
# Show the current Forwiroh version, or set it in every file that carries it.
# Usage: scripts/version.sh [X.Y.Z]
set -eu
cd "$(dirname "$0")/.."

cur=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
if [ $# -eq 0 ]; then echo "$cur"; exit 0; fi

v=${1#v}
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' ||
	{ echo "usage: $0 [X.Y.Z]" >&2; exit 1; }
if [ "$v" = "$cur" ]; then echo "already at $v"; exit 0; fi

# BSD and GNU sed agree on in-place edits once a backup suffix is given.
sedi() { f=$1; shift; sed -i.bak "$@" "$f"; rm -f "$f.bak"; }

sedi Cargo.toml "s|^version = \".*\"|version = \"$v\"|"
sedi Cargo.lock -E "/^name = \"forwiroh(-cli)?\"$/,/^version = /s|^version = \".*\"|version = \"$v\"|"
for f in apps/ios/Forwiroh/Info.plist apps/ios/PacketTunnel/Info.plist apps/macos/Forwiroh/Info.plist; do
	sedi "$f" -E "s|<string>[0-9]+\.[0-9]+\.[0-9]+</string>|<string>$v</string>|"
done
sedi apps/android/app/build.gradle "s|versionName '[^']*'|versionName '$v'|"
code=$(sed -n 's/.*versionCode \([0-9]*\).*/\1/p' apps/android/app/build.gradle)
sedi apps/android/app/build.gradle "s|versionCode $code|versionCode $((code + 1))|"
sedi apps/gtk/meson.build "1s|version: '[^']*'|version: '$v'|"

echo "$cur -> $v"
echo "Next: add docs/releases/$v.md, then run scripts/check-release.py v$v"
