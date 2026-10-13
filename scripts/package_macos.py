#!/usr/bin/env python3
"""Bundle an already-built ARM64 desktop executable as a native macOS app."""
import argparse
import pathlib
import plistlib
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/openpatch-desktop")
parser.add_argument("--output", default="dist/OpenPatch.app")
args = parser.parse_args()
binary = pathlib.Path(args.binary)
if not binary.is_file():
    parser.error(f"Build the desktop executable first: {binary}")
app = pathlib.Path(args.output)
contents = app / "Contents"
macos = contents / "MacOS"
macos.mkdir(parents=True, exist_ok=True)
shutil.copy2(binary, macos / "openpatch-desktop")
(macos / "openpatch-desktop").chmod(0o755)
with (contents / "Info.plist").open("wb") as stream:
    plistlib.dump({
        "CFBundleName": "OpenPatch",
        "CFBundleDisplayName": "OpenPatch",
        "CFBundleIdentifier": "org.openpatch.desktop",
        "CFBundleVersion": "2",
        "CFBundleShortVersionString": "0.2.0",
        "CFBundleExecutable": "openpatch-desktop",
        "CFBundlePackageType": "APPL",
        "NSHighResolutionCapable": True,
        "LSMinimumSystemVersion": "12.0",
        "NSHumanReadableCopyright": "OpenPatch contributors. MIT license.",
    }, stream)
# Local ad-hoc signing satisfies Apple Silicon code-signing requirements. This
# is not a Developer ID signature or notarization for external distribution.
subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
print(app.resolve())
