#!/usr/bin/env python3
"""Validate source coverage and extension wiring without rewriting signing settings."""
import plistlib
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "apps/ios"
project = plistlib.loads(subprocess.check_output([
    "plutil", "-convert", "xml1", "-o", "-", str(root / "Forwiroh.xcodeproj/project.pbxproj")
]))
objects = project["objects"]
targets = {o["name"]: o for o in objects.values() if o.get("isa") == "PBXNativeTarget"}
assert set(targets) == {"Forwiroh", "PacketTunnel"}, "Missing app or VPN extension target"
for name, target in targets.items():
    phases = [objects[p] for p in target["buildPhases"]]
    compiled = {
        objects[objects[file]["fileRef"]]["path"]
        for phase in phases if phase["isa"] == "PBXSourcesBuildPhase"
        for file in phase["files"]
    }
    expected = {str(p.relative_to(root)) for directory in (root / name, root / "Shared")
                for p in directory.glob("*.swift")}
    assert compiled == expected, f"{name}: source mismatch: {compiled ^ expected}"
    configs = objects[target["buildConfigurationList"]]["buildConfigurations"]
    for config in configs:
        settings = objects[config]["buildSettings"]
        assert (root / settings["CODE_SIGN_ENTITLEMENTS"]).is_file(), "Missing entitlements"
        assert settings["FORWIROH_APP_GROUP"] == "group.org.radare.forwiroh", "Wrong shared group"
        assert settings["FORWIROH_KEYCHAIN_GROUP"] == "org.radare.forwiroh.shared", "Wrong key group"
        assert settings["SWIFT_OBJC_BRIDGING_HEADER"] == "Shared/forwiroh.h", "Wrong C bridge"
        if name == "PacketTunnel":
            assert settings["APPLICATION_EXTENSION_API_ONLY"] == "YES", "Extension-only APIs disabled"
    assert any(p["isa"] == "PBXShellScriptBuildPhase" and "build-rust.sh" in p["shellScript"]
               for p in phases), f"{name}: missing Rust build"
embed = [objects[p] for p in targets["Forwiroh"]["buildPhases"]
         if objects[p]["isa"] == "PBXCopyFilesBuildPhase"]
assert any(str(p["dstSubfolderSpec"]) == "13" and any(
    objects[f]["fileRef"] == targets["PacketTunnel"]["productReference"] for f in p["files"]
) for p in embed), "VPN extension is not embedded"
print("iOS project: Swift source coverage, Rust builds, shared entitlements and VPN embedding verified")
