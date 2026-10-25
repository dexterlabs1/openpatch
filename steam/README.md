# Steam release

OpenPatch ships on Steam as one app with three depots: Linux x86_64 (Steam Deck), Windows x64 and macOS ARM.
Nothing here uploads automatically. Builds are never set live by script; promote them in Steamworks.

## Build the depots

1. Run CI for the release commit (a `work/release-*` PR runs all platforms). Download:
   - `openpatch-linux-x86_64-steamrt` — the Linux package built inside the Steam Linux Runtime 3.0 SDK.
     Use this one, not the Ubuntu `openpatch-linux-x86_64` package: `scripts/check_glibc.py` gates it at glibc 2.31.
   - `openpatch-windows-x86_64` and `openpatch-macos-arm64`.
2. `python3 scripts/package_steam.py --linux … --windows … --macos … --app-id … --depot-linux … --depot-windows … --depot-macos …`
   writes `steam/content/<platform>/` (portable installer, updater and extract-me guides removed) and the `.vdf` files.
3. `STEAM_USER=… sh steam/upload.sh` (needs `steamcmd`; refuses placeholder IDs).

## Steamworks settings

| Setting | Value |
|---|---|
| Linux launch | `openpatch-desktop`, OS Linux, **Steam Linux Runtime 3.0 (sniper)** compatibility tool |
| Windows launch | `openpatch-desktop.exe`, OS Windows |
| macOS launch | `OpenPatch.app`, OS macOS, ARM64 |
| Steam Input | Default *Gamepad* layout; OpenPatch reads controllers natively ([gamepad.rs](../crates/openpatch-desktop/src/gamepad.rs)). Trackpad-as-mouse on the right pad. |
| Steam Cloud | Off for the RC; patches live in `~/OpenPatch` |
| DRM | None (no Steamworks SDK dependency) |

## Release-candidate gates

| Gate | Status |
|---|---|
| Linux binary needs glibc ≤ 2.31 | CI job `steam-linux` |
| Linux runtime build launches on a stock desktop distribution | CI job `steam-linux-smoke` |
| Depot layout from CI archives; mac bundle signature intact after unpack | Checked locally against v0.3.3 |
| Steam Deck Game Mode: launch, audio, controls, 1280×800 fit, text entry | **Needs the Deck** |
| Windows audio device on real hardware | Open |
| macOS: Developer ID signing + notarization | Open — current builds are ad-hoc signed; Steam does not quarantine downloads, but notarize before launch |

## Store assets still needed

Header capsule 920×430, small capsule 462×174, main capsule 1232×706, vertical capsule 748×896,
library capsule 600×900, library hero 3840×1240, library logo 1280×720 (transparent), page background 1438×810 (optional),
at least five 1920×1080 screenshots, and a short trailer. Screenshots can be captured silently with
`OPENPATCH_WINDOW=1920x1080 OPENPATCH_CAPTURE=shot.png@6 openpatch-desktop --workspace <scratch>` and a
`.host-settings.json` that names a nonexistent output device.
