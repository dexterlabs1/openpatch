#!/usr/bin/env python3
"""Bundle an already-built ARM64 desktop executable as a native macOS app."""
import argparse
import pathlib
import plistlib
import shutil
import subprocess
import tomllib

from package_release import copy_notices

root = pathlib.Path(__file__).resolve().parents[1]
version = tomllib.loads((root / "crates/openpatch-desktop/Cargo.toml").read_text())["package"]["version"]

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/openpatch-desktop")
parser.add_argument("--cli-binary", help="MCP attachment CLI (default: openpatch beside desktop binary)")
parser.add_argument("--output", default="dist/OpenPatch.app")
parser.add_argument("--bundle-id", default="org.openpatch.desktop", help="CFBundleIdentifier (use a distinct id for side-by-side previews)")
parser.add_argument("--name", default="OpenPatch", help="Bundle and display name")
parser.add_argument("--build-number", default="16", help="CFBundleVersion (default: 16)")
parser.add_argument("--source", help="Corresponding source URL for this exact build")
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
copy_notices(licenses, args.source or f"https://github.com/dexterlabs1/openpatch/tree/v{version}")
with (contents / "Info.plist").open("wb") as stream:
    plistlib.dump({
        "CFBundleName": args.name,
        "CFBundleDisplayName": args.name,
        "CFBundleIdentifier": args.bundle_id,
        "CFBundleVersion": args.build_number,
        "CFBundleShortVersionString": version,
        "CFBundleExecutable": "openpatch-desktop",
        "CFBundleIconFile": "OpenPatch.icns",
        "CFBundlePackageType": "APPL",
        "NSHighResolutionCapable": True,
        "LSMinimumSystemVersion": "12.0",
        "NSHumanReadableCopyright": "OpenPatch contributors; VCV © 2016-2023. Combined binaries: GPL-3.0-or-later; original OpenPatch code: MIT.",
    }, stream)
# Local ad-hoc signing satisfies Apple Silicon code-signing requirements. This
# is not a Developer ID signature or notarization for external distribution.
subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
print(app.resolve())
