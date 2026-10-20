#!/usr/bin/env python3
"""Package one CLI executable, or checksum the complete CLI release set."""
import argparse
import hashlib
from pathlib import Path
import shutil
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
TARGETS = (
    "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin", "aarch64-apple-darwin",
    "x86_64-linux-android", "aarch64-linux-android",
    "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc",
)


def asset(target):
    return f"forwiroh-{target}.{'zip' if 'windows' in target else 'tar.gz'}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/cli")
    parser.add_argument("--assemble", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    if args.assemble:
        names = [asset(target) for target in TARGETS]
        for name in names:
            if not (args.output / name).is_file():
                parser.error(f"Missing CLI asset: {name}")
        for name in ("install.sh", "install.ps1"):
            shutil.copy2(ROOT / name, args.output / name)
            names.append(name)
        (args.output / "CLI-SHA256SUMS").write_text("".join(
            f"{hashlib.sha256((args.output / name).read_bytes()).hexdigest()}  {name}\n"
            for name in sorted(names)
        ))
        return
    if not args.target or not args.binary or not args.binary.is_file():
        parser.error("Provide --target and an existing --binary, or --assemble")
    executable = "forwiroh.exe" if "windows" in args.target else "forwiroh"
    destination = args.output / asset(args.target)
    files = [(args.binary, executable), (ROOT / "LICENSE", "LICENSE")]
    if "windows" in args.target:
        with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for path, name in files:
                archive.write(path, name)
    else:
        with tarfile.open(destination, "w:gz", compresslevel=9) as archive:
            for path, name in files:
                info = archive.gettarinfo(path, arcname=name)
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mode = 0o755 if name == executable else 0o644
                with path.open("rb") as source:
                    archive.addfile(info, source)
    print(f"{destination.name}: {destination.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
