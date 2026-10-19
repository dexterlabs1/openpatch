#!/usr/bin/env python3
"""Package prebuilt OpenPatch desktop and CLI binaries for one release target."""
import argparse
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/openpatch-desktop/assets"
PLATFORMS = ("macos-arm64", "windows-x86_64", "linux-x86_64")


def copy_notices(destination, source):
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "LICENSE", destination / "OpenPatch-MIT.txt")
    for name in ("LICENSE-THIRD-PARTY", "LICENSE-GPL-3.0.txt"):
        shutil.copy2(ROOT / "crates/openpatch-core" / name, destination / name)
    for notice in sorted((ASSETS / "fonts").glob("OFL-*.txt")):
        shutil.copy2(notice, destination / notice.name)
    for notice in sorted((ASSETS / "licenses").iterdir()):
        if notice.is_file():
            shutil.copy2(notice, destination / notice.name)
    (destination / "SOURCE.txt").write_text(
        "OpenPatch combined binaries are distributed under GPL-3.0-or-later.\n"
        "Original OpenPatch code is MIT licensed; see the included notices.\n"
        f"Corresponding source for this release: {source}\n", encoding="utf-8")


INSTALLER = r'''#!/bin/sh
# Install only managed program files. User patches and settings are never removed.
set -eu
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
dest="$HOME/.local/share/openpatch"
bin="$HOME/.local/bin"
applications="$HOME/.local/share/applications"
mkdir -p "$dest" "$bin" "$applications"
# Rename complete new executables into place, including when an old process runs.
for program in openpatch-desktop openpatch; do
    install -m 755 "$source_dir/$program" "$dest/$program.new"
    mv -f "$dest/$program.new" "$dest/$program"
done
cp "$source_dir/VERSION" "$dest/VERSION"
cp "$source_dir/openpatch.png" "$dest/openpatch.png"
mkdir -p "$dest/Licenses"
cp "$source_dir/Licenses/"* "$dest/Licenses/"
install -m 755 "$source_dir/openpatch-update.sh" "$bin/openpatch-update"
ln -sfn "$dest/openpatch" "$bin/openpatch"
# Escape paths for the Desktop Entry specification, including spaces and %.
desktop_path() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/`/\\`/g' -e 's/\$/\\$/g' -e 's/%/%%/g'; }
executable=$(desktop_path "$dest/openpatch-desktop")
updater=$(desktop_path "$bin/openpatch-update")
cat > "$applications/openpatch.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=OpenPatch
Comment=Modular synthesizer
Exec="$executable"
Icon=$dest/openpatch.png
Terminal=false
Categories=AudioVideo;Audio;Music;
EOF
cat > "$applications/openpatch-update.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Update OpenPatch
Exec="$updater"
Terminal=true
Categories=AudioVideo;Audio;
EOF
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications" >/dev/null 2>&1 || true
fi
printf 'Installed OpenPatch %s in %s\nLaunch OpenPatch from your app menu.\n' "$(cat "$dest/VERSION")" "$dest"
printf 'Examples remain in %s/examples; existing patches and settings are preserved.\n' "$source_dir"
'''

LINUX_GUIDE = """OpenPatch for Steam Deck / Linux x86_64

Extract this entire archive. In Steam Deck Desktop Mode, open Konsole in the
extracted OpenPatch-linux-x86_64 folder and run:

    ./install.sh

No root access is needed. OpenPatch appears in the application menu. To add it
to Steam, choose Games > Add a Non-Steam Game and select OpenPatch (or browse to
~/.local/share/openpatch/openpatch-desktop). Native Linux x86_64; do not force
Proton. Audio device and game-mode behavior require testing on your own device.

The installer replaces managed executables and launchers in ~/.local, preserving
user patches and settings. Examples stay in this extracted folder. Repeat the
installer to update, or run ~/.local/bin/openpatch-update. Restart OpenPatch
after an update. The updater needs curl and python3 and downloads GitHub releases.

You can also run ./openpatch-desktop directly without installing. The bundled
./openpatch CLI provides command-line and MCP attachment workflows. Linux needs
a working desktop session and the standard ALSA, X11/Wayland and OpenGL runtime
libraries; no system packages or SteamOS files are changed by this installer.

Patch Engine controls: F1 toggles the overlay; Up/Down chooses the row and
Left/Right chooses an option; Enter starts; Space or / chooses a surprise; Esc
closes the overlay. Touch the visible controls to do the same. Steam Input can
map the D-pad to arrow keys, A to Enter, X to Space, B to Esc and a back button
to F1. Configure this keyboard mapping explicitly; native gamepad button input
is not assumed. R selects a mood and is not the surprise shortcut.
"""


def package(args):
    version = tomllib.loads((ROOT / "crates/openpatch-desktop/Cargo.toml").read_text())["package"]["version"]
    source = args.source or f"https://github.com/dexterlabs1/openpatch/tree/v{version}"
    binary_dir = pathlib.Path(args.binary_dir).resolve()
    suffix = ".exe" if args.platform == "windows-x86_64" else ""
    desktop = binary_dir / ("openpatch-desktop" + suffix)
    cli = binary_dir / ("openpatch" + suffix)
    # CLI and GUI must be distinct executables; never silently ship the CLI as GUI.
    for binary in (desktop, cli):
        if not binary.is_file():
            raise ValueError(f"Missing prebuilt executable: {binary}")
    extension = ".tar.gz" if args.platform == "linux-x86_64" else ".zip"
    name = f"OpenPatch-{args.platform}"
    output = pathlib.Path(args.output)
    if not str(output).endswith(extension):
        output = output / (name + extension)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="openpatch-package-") as temporary:
        staging = pathlib.Path(temporary)
        folder = staging / name
        folder.mkdir()
        if args.platform == "macos-arm64":
            subprocess.run([
                sys.executable, str(ROOT / "scripts/package_macos.py"),
                "--binary", str(desktop), "--cli-binary", str(cli),
                "--output", str(folder / "OpenPatch.app"), "--build-number", args.build_number,
                "--source", source,
            ], check=True)
        else:
            for binary in (desktop, cli):
                shutil.copy2(binary, folder / binary.name)
                (folder / binary.name).chmod(0o755)
        copy_notices(folder / "Licenses", source)
        shutil.copytree(ROOT / "examples", folder / "examples")
        (folder / "VERSION").write_text(f"v{version}\n", encoding="utf-8")
        if args.platform == "linux-x86_64":
            (folder / "install.sh").write_text(INSTALLER, encoding="utf-8")
            (folder / "install.sh").chmod(0o755)
            shutil.copy2(ROOT / "scripts/openpatch-update.sh", folder / "openpatch-update.sh")
            (folder / "openpatch-update.sh").chmod(0o755)
            shutil.copy2(ASSETS / "openpatch.png", folder / "openpatch.png")
            (folder / "STEAM-DECK.txt").write_text(LINUX_GUIDE, encoding="utf-8")
            with tarfile.open(output, "w:gz") as archive:
                archive.add(folder, arcname=name)
        else:
            instructions = (
                "Extract the entire folder, then run openpatch-desktop.exe.\n"
                "openpatch.exe is the command-line tool. For a live MCP window, use\n"
                "openpatch-desktop.exe --mcp; attachment to an existing Windows window\n"
                "is not supported yet.\n"
                if args.platform == "windows-x86_64" else
                "Move OpenPatch.app to Applications.\n"
                "This app is ad-hoc signed, not Developer ID signed or notarized.\n"
            )
            (folder / "START-HERE.txt").write_text(instructions, encoding="utf-8")
            with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
                for path in sorted(folder.rglob("*")):
                    archive.write(path, path.relative_to(staging))
    return output.resolve()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True, choices=PLATFORMS)
    parser.add_argument("--binary-dir", required=True, help="Directory containing prebuilt desktop and CLI executables")
    parser.add_argument("--output", default="dist", help="Output archive path, or directory (default: dist)")
    parser.add_argument("--build-number", default="11")
    parser.add_argument("--source", help="Corresponding source URL for the exact release revision")
    args = parser.parse_args()
    try:
        print(package(args))
    except ValueError as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
