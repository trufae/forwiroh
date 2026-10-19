#!/usr/bin/env python3
"""Assemble already-built archives into SwiftPM packages; never invokes Rust.

Accepts dist/libraries/<rust-target>/libforwiroh.a (CI download layout) or
explicit --library rust-target=/absolute/path/libforwiroh.a arguments.
"""
import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
APPLE = {
    "aarch64-apple-darwin": "macos",
    "x86_64-apple-darwin": "macos",
    "aarch64-apple-ios": "ios",
    "aarch64-apple-ios-sim": "simulator",
    "x86_64-apple-ios": "simulator",
}
NATIVE = {
    "aarch64-unknown-linux-gnu": ["aarch64-unknown-linux-gnu"],
    "x86_64-unknown-linux-gnu": ["x86_64-unknown-linux-gnu"],
    "aarch64-unknown-linux-musl": ["aarch64-unknown-linux-musl", "aarch64-swift-linux-musl"],
    "x86_64-unknown-linux-musl": ["x86_64-unknown-linux-musl", "x86_64-swift-linux-musl"],
}
for arch in ("aarch64", "x86_64"):
    NATIVE[f"{arch}-linux-android"] = [f"{arch}-unknown-linux-android"] + [
        f"{arch}-unknown-linux-android{api}" for api in range(28, 41)
    ]


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def archive(directory, destination, extra=None):
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as output:
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                output.write(path, path.relative_to(directory.parent))
        if extra:
            for prefix, source in extra.items():
                for path in sorted(source.rglob("*")):
                    if path.is_file():
                        output.write(path, pathlib.Path(directory.name) / prefix / path.relative_to(source))
    return hashlib.sha256(destination.read_bytes()).hexdigest()


def headers(destination):
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "crates/forwiroh/include/forwiroh.h", destination)
    (destination / "module.modulemap").write_text('module CForwiroh {\n    header "forwiroh.h"\n    export *\n}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--libraries", type=pathlib.Path, default=ROOT / "dist/libraries")
    parser.add_argument("--library", action="append", default=[], metavar="TARGET=PATH")
    parser.add_argument("--require-all", action="store_true", help="Require every release architecture")
    parser.add_argument("--release-base-url", help="Immutable HTTPS release download directory")
    args = parser.parse_args()
    version = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
    libraries = {p.parent.name: p.resolve() for p in args.libraries.glob("*/libforwiroh.a")}
    for value in args.library:
        target, path = value.split("=", 1)
        libraries[target] = pathlib.Path(path).resolve()
    if not libraries:
        parser.error("No archives found; run scripts/build-swift-core.sh first")
    for target, path in libraries.items():
        if target not in APPLE and target not in NATIVE:
            parser.error(f"Unsupported target {target}")
        if not path.is_file() or path.open("rb").read(8) != b"!<arch>\n":
            parser.error(f"Not a static archive: {path}")
    missing = (APPLE.keys() | NATIVE.keys()) - libraries.keys()
    if args.require_all and missing:
        parser.error(f"Missing release targets: {', '.join(sorted(missing))}")
    if args.release_base_url and (not args.require_all or not args.release_base_url.startswith("https://")):
        parser.error("Remote manifests require --require-all and an HTTPS release URL")

    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="swiftpm-", dir=dist) as tmp:
        staging = pathlib.Path(tmp)
        artifacts = staging / "Artifacts"
        artifacts.mkdir()
        entries = []
        if any(t in APPLE for t in libraries):
            include = staging / "include"
            headers(include)
            command = ["xcodebuild", "-create-xcframework"]
            for group in ("macos", "ios", "simulator"):
                inputs = [str(path) for t, path in libraries.items() if APPLE.get(t) == group]
                if not inputs:
                    continue
                output = staging / group / "libforwiroh.a"
                output.parent.mkdir()
                if len(inputs) == 1:
                    shutil.copy2(inputs[0], output)
                else:
                    subprocess.run(["xcrun", "lipo", "-create", *inputs, "-output", str(output)], check=True)
                command += ["-library", str(output), "-headers", str(include)]
            path = artifacts / "ForwirohApple.xcframework"
            subprocess.run([*command, "-output", str(path)], check=True)
            entries.append({"name": "ForwirohApple", "path": f"Artifacts/{path.name}"})

        if any(t in NATIVE for t in libraries):
            path = artifacts / "ForwirohNative.artifactbundle"
            headers(path / "include")
            variants = []
            for target, library in sorted(libraries.items()):
                if target not in NATIVE:
                    continue
                destination = path / target / "libforwiroh.a"
                destination.parent.mkdir()
                shutil.copy2(library, destination)
                variants.append({"path": f"{target}/libforwiroh.a", "supportedTriples": NATIVE[target],
                                 "staticLibraryMetadata": {"headerPaths": ["include"],
                                                           "moduleMapPath": "include/module.modulemap"}})
            write_json(path / "info.json", {"schemaVersion": "1.0", "artifacts": {
                "ForwirohNative": {"version": version, "type": "staticLibrary", "variants": variants}}})
            entries.append({"name": "ForwirohNative", "path": f"Artifacts/{path.name}"})

        write_json(artifacts / "manifest.json", entries)
        write_json(artifacts / "build-info.json", {"version": version, "targets": sorted(libraries),
                   "source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                   "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
                   "cargoLockSHA256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest()})
        remote = []
        sums = []
        for entry in entries:
            path = staging / entry["path"]
            name = f"{path.name}.zip"
            checksum = archive(path, dist / name)
            sums.append(f"{checksum}  {name}")
            if args.release_base_url:
                remote.append({"name": entry["name"], "url": f"{args.release_base_url.rstrip('/')}/{name}",
                               "checksum": checksum})

        # Local checkout can immediately be used with .package(path: ...).
        if (ROOT / "Artifacts").exists():
            shutil.rmtree(ROOT / "Artifacts")
        shutil.move(artifacts, ROOT / "Artifacts")
        package = staging / "Forwiroh"
        package.mkdir()
        for name in ("Package.swift", "LICENSE"):
            shutil.copy2(ROOT / name, package)
        for name in ("Sources", "Tests"):
            shutil.copytree(ROOT / name, package / name)
        shutil.copy2(ROOT / "docs/swiftpm.md", package / "README.md")
        filename = f"Forwiroh-{version}-swiftpm.zip"
        sums.append(f"{archive(package, dist / filename, {'Artifacts': ROOT / 'Artifacts'})}  {filename}")
        if remote:
            write_json(package / "Artifacts/manifest.json", remote)
            filename = f"Forwiroh-{version}-swiftpm-remote.zip"
            sums.append(f"{archive(package, dist / filename)}  {filename}")
        (dist / "SHA256SUMS").write_text("\n".join(sums) + "\n")
        print(f"Packaged {', '.join(sorted(libraries))}")
        print(f"Ready for .package(path: \"{ROOT}\"); distributions in {dist}")


if __name__ == "__main__":
    main()
