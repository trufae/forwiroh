#!/usr/bin/env python3
"""Test SDK sources against a local shared core without changing release artifacts."""
import json, shutil, subprocess, sys, tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
library = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'target/debug'
with tempfile.TemporaryDirectory(prefix='forwiroh-swift-') as temporary:
    package = Path(temporary)
    shutil.copytree(root / 'Sources/Forwiroh', package / 'Sources/Forwiroh')
    shutil.copytree(root / 'Tests/ForwirohTests', package / 'Tests/ForwirohTests')
    (package / 'Tests/Fixtures').mkdir()
    shutil.copy2(root / 'crates/forwiroh/tests/fixtures/control-success.json', package / 'Tests/Fixtures')
    legacy = package / 'Tests/LegacyForwirohTests'
    legacy.mkdir()
    shutil.copy2(root / 'Tests/compat/baseline-93883c1/ForwirohTests.swift', legacy)
    (package / 'CForwiroh').mkdir()
    shutil.copy2(root / 'crates/forwiroh/include/forwiroh.h', package / 'CForwiroh/forwiroh.h')
    (package / 'CForwiroh/module.modulemap').write_text('module CForwiroh { header "forwiroh.h" link "forwiroh" export * }\n')
    flags = ', '.join(map(json.dumps, ['-L', str(library), '-Xlinker', '-rpath', '-Xlinker', str(library)]))
    (package / 'Package.swift').write_text('''// swift-tools-version: 6.2
import PackageDescription
let package = Package(name: "ForwirohAPITests", platforms: [.macOS(.v14)], targets: [
    .systemLibrary(name: "CForwiroh", path: "CForwiroh"),
    .target(name: "Forwiroh", dependencies: ["CForwiroh"], linkerSettings: [.unsafeFlags([''' + flags + '''])]),
    .testTarget(name: "ForwirohTests", dependencies: ["Forwiroh"]),
    .testTarget(name: "LegacyForwirohTests", dependencies: ["Forwiroh"]),
])
''')
    subprocess.run(['swift', 'test', '--package-path', str(package), '--jobs', '2'], check=True)
