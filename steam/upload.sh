#!/bin/sh
# Upload the prepared depots with SteamCMD. Run scripts/package_steam.py first.
# The build is never set live here; promote it to a branch in Steamworks.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
[ -f "$here/app_build.vdf" ] || { echo "Run scripts/package_steam.py first." >&2; exit 1; }
if grep -Eq '"(AppID|DepotID)" "0"|"0" "depot_' "$here"/app_build.vdf "$here"/depot_*.vdf; then
    echo "Steam app and depot IDs are still placeholders (0)." >&2
    echo "Re-run scripts/package_steam.py with --app-id and --depot-* from Steamworks." >&2
    exit 1
fi
: "${STEAM_USER:?Set STEAM_USER to the Steamworks build account}"
command -v steamcmd >/dev/null || { echo "steamcmd is not installed." >&2; exit 1; }
exec steamcmd +login "$STEAM_USER" +run_app_build "$here/app_build.vdf" +quit
