#!/usr/bin/env python3
"""Bundle an already-built ARM64 desktop executable as a native macOS app."""
import argparse
import pathlib
import plistlib
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/openpatch-desktop")
parser.add_argument("--cli-binary", help="MCP attachment CLI (default: openpatch beside desktop binary)")
parser.add_argument("--output", default="dist/OpenPatch.app")
args = parser.parse_args()
binary = pathlib.Path(args.binary)
if not binary.is_file():
    parser.error(f"Build the desktop executable first: {binary}")
cli = pathlib.Path(args.cli_binary) if args.cli_binary else binary.with_name("openpatch")
if not cli.is_file():
    parser.error(f"Build the workspace CLI first: {cli}")
app = pathlib.Path(args.output)
contents = app / "Contents"
macos = contents / "MacOS"
macos.mkdir(parents=True, exist_ok=True)
shutil.copy2(binary, macos / "openpatch-desktop")
(macos / "openpatch-desktop").chmod(0o755)
shutil.copy2(cli, macos / "openpatch")
(macos / "openpatch").chmod(0o755)
resources = contents / "Resources"
resources.mkdir(parents=True, exist_ok=True)
icon = pathlib.Path(__file__).resolve().parents[1] / "crates/openpatch-desktop/assets/OpenPatch.icns"
shutil.copy2(icon, resources / "OpenPatch.icns")
licenses = resources / "Licenses"
licenses.mkdir(parents=True, exist_ok=True)
shutil.copy2(icon.parent / "fonts/OFL-Barlow.txt", licenses / "Barlow.txt")
for notice in (icon.parent / "licenses").iterdir():
    if notice.is_file():
        shutil.copy2(notice, licenses / notice.name)
with (contents / "Info.plist").open("wb") as stream:
    plistlib.dump({
        "CFBundleName": "OpenPatch",
        "CFBundleDisplayName": "OpenPatch",
        "CFBundleIdentifier": "org.openpatch.desktop",
        "CFBundleVersion": "2",
        "CFBundleShortVersionString": "0.2.0",
        "CFBundleExecutable": "openpatch-desktop",
        "CFBundleIconFile": "OpenPatch.icns",
        "CFBundlePackageType": "APPL",
        "NSHighResolutionCapable": True,
        "LSMinimumSystemVersion": "12.0",
        "NSHumanReadableCopyright": "OpenPatch contributors. MIT license.",
    }, stream)
# Local ad-hoc signing satisfies Apple Silicon code-signing requirements. This
# is not a Developer ID signature or notarization for external distribution.
subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
print(app.resolve())
