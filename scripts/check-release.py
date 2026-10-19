#!/usr/bin/env python3
"""Check release versions before building or publishing (Python 3.11+)."""
import pathlib
import plistlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
if not re.fullmatch(r"\d+\.\d+\.\d+", version):
    sys.exit(f"Expected a stable semantic version, got {version!r}")
if len(sys.argv) > 1 and sys.argv[1] != f"v{version}":
    sys.exit(f"Tag {sys.argv[1]!r} does not match Cargo.toml version v{version}")

versions = {}
for package in tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]:
    if package["name"] in ("forwiroh", "forwiroh-cli"):
        versions[f"Cargo.lock: {package['name']}"] = package["version"]
for name in ("ios/Forwiroh", "ios/PacketTunnel", "macos/Forwiroh"):
    path = ROOT / "apps" / name / "Info.plist"
    versions[str(path.relative_to(ROOT))] = plistlib.loads(path.read_bytes())["CFBundleShortVersionString"]
versions["Android"] = re.search(r"versionName '([^']+)'", (ROOT / "apps/android/app/build.gradle").read_text())[1]
versions["GTK"] = re.search(r"version: '([^']+)'", (ROOT / "apps/gtk/meson.build").read_text())[1]
for name, actual in versions.items():
    if actual != version:
        sys.exit(f"{name}: expected {version}, got {actual}")
notes = ROOT / "docs/releases" / f"{version}.md"
if not notes.is_file():
    sys.exit(f"Missing release notes: {notes.relative_to(ROOT)}")
print(f"Release v{version}: versions and release notes agree")
