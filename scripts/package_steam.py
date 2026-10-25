#!/usr/bin/env python3
"""Lay out Steam depot content from the three CI release archives.

Steam installs and updates the app itself, so the depots leave out the portable
Linux installer, the GitHub updater and the "extract this folder" guides. The
Linux archive must come from the Steam Linux Runtime build (steam-linux CI job).

    python3 scripts/package_steam.py \
        --linux OpenPatch-linux-x86_64.tar.gz \
        --windows OpenPatch-windows-x86_64.zip \
        --macos OpenPatch-macos-arm64.zip \
        --app-id 0 --depot-linux 0 --depot-windows 0 --depot-macos 0

IDs default to 0 until the Steamworks app exists; steam/upload.sh refuses to
upload with placeholder IDs.
"""
import argparse
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tomllib
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
STEAM = ROOT / "steam"
PORTABLE_ONLY = {"install.sh", "openpatch-update.sh", "STEAM-DECK.txt", "START-HERE.txt"}


def extract(archive, destination):
    destination.mkdir(parents=True)
    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive) as tar:
            tar.extractall(destination, filter="tar")
    else:
        # zipfile drops Unix modes; ditto keeps the macOS bundle's executable bits and signature.
        if sys.platform == "darwin":
            subprocess.run(["ditto", "-x", "-k", str(archive), str(destination)], check=True)
        else:
            with zipfile.ZipFile(archive) as zf:
                zf.extractall(destination)
    (folder,) = [p for p in destination.iterdir() if p.is_dir()]
    for item in folder.iterdir():
        if item.name in PORTABLE_ONLY:
            item.unlink()
        else:
            item.rename(destination / item.name)
    folder.rmdir()


def render(template, values):
    text = template.read_text(encoding="utf-8")
    for key, value in values.items():
        text = text.replace(f"@{key}@", str(value))
    (STEAM / template.name.removesuffix(".in")).write_text(text, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for platform in ("linux", "windows", "macos"):
        parser.add_argument(f"--{platform}", required=True, type=pathlib.Path, help=f"{platform} release archive")
        parser.add_argument(f"--depot-{platform}", default="0")
    parser.add_argument("--app-id", default="0")
    args = parser.parse_args()

    content = STEAM / "content"
    shutil.rmtree(content, ignore_errors=True)
    for platform in ("linux", "windows", "macos"):
        extract(getattr(args, platform).resolve(), content / platform)

    version = tomllib.loads((ROOT / "crates/openpatch-desktop/Cargo.toml").read_text())["package"]["version"]
    values = {
        "APP_ID": args.app_id,
        "VERSION": version,
        "DEPOT_LINUX": args.depot_linux,
        "DEPOT_WINDOWS": args.depot_windows,
        "DEPOT_MACOS": args.depot_macos,
    }
    for template in sorted(STEAM.glob("*.vdf.in")):
        render(template, values)
    (STEAM / "steam_appid.txt").write_text(f"{args.app_id}\n", encoding="utf-8")
    for platform in ("linux", "windows", "macos"):
        files = sorted(p.relative_to(content / platform) for p in (content / platform).iterdir())
        print(f"{platform}: {', '.join(map(str, files))}")


if __name__ == "__main__":
    main()
