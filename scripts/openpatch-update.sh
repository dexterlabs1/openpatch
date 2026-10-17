#!/bin/sh
# OpenPatch updater for Steam Deck / Linux x86_64: installs the newest GitHub release build for this user.
# Usage: openpatch-update [--check]
set -e
REPO="dexterlabs1/openpatch"
DEST="$HOME/.local/share/openpatch"
ASSET="linux-x86_64.tar.gz"
current="$(cat "$DEST/VERSION" 2>/dev/null || echo none)"

api() { curl -fsSL -H "Accept: application/vnd.github+json" "https://api.github.com/repos/$REPO/releases?per_page=20"; }
json="$(api)" || { echo "Could not reach GitHub. Check the network and try again."; exit 1; }
# First (newest) published release that carries a Linux x86_64 build.
tag="$(printf '%s' "$json" | python3 -c '
import json, sys
for r in json.load(sys.stdin):
    if r.get("draft"): continue
    for a in r.get("assets", []):
        if a["name"].endswith(sys.argv[1]):
            print(r["tag_name"], a["browser_download_url"]); sys.exit()
' "$ASSET")"
[ -n "$tag" ] || { echo "No Linux build found in the latest releases."; exit 1; }
latest="${tag%% *}"; url="${tag#* }"

if [ "$latest" = "$current" ]; then echo "OpenPatch is up to date ($current)."; exit 0; fi
echo "Installed: $current  ·  Latest: $latest"
[ "$1" = "--check" ] && exit 0

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
echo "Downloading $latest…"
curl -fL --progress-bar -o "$tmp/openpatch.tar.gz" "$url"
tar -xzf "$tmp/openpatch.tar.gz" -C "$tmp"
dir="$(find "$tmp" -maxdepth 1 -type d -name 'OpenPatch*' | head -1)"
[ -x "$dir/install.sh" ] || { echo "Download did not contain install.sh"; exit 1; }
if pgrep -f "$DEST/openpatch-desktop" >/dev/null 2>&1; then echo "Note: OpenPatch is running; restart it after the update."; fi
"$dir/install.sh"
echo "$latest" > "$DEST/VERSION"
echo "Updated to $latest."
