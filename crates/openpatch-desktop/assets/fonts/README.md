# Panel typography

Rajdhani SemiBold by Indian Type Foundry, copyright 2014.
The unmodified static weight-600 face is distributed under the SIL Open Font
License 1.1; see [OFL-Rajdhani.txt](OFL-Rajdhani.txt).

Pinned source: [Google Fonts at `1d0e46c7933245c8180fe83e47917961c913bbdb`](https://github.com/google/fonts/blob/1d0e46c7933245c8180fe83e47917961c913bbdb/ofl/rajdhani/Rajdhani-SemiBold.ttf).

SHA-256 (`Rajdhani-SemiBold.ttf`, 390,340 bytes): `94bbd25a18ca665999feb05a537de9fd2b860dcfb78bbe9ca00270825bf235da`.

SHA-256 (`OFL-Rajdhani.txt`): `f62ef357d3a1c3d27edd35a6e1ba350e8a8d13499797964eeadefbf0b3b15d1f`.

This is candidate 09 selected from the browser font study. Both established
`Panel Print` and `Panel Heading` families use this face for panel titles,
labels, readouts and numeric entry. Digits have proportional advances; numeric
wells retain their existing bounds and fit their text to the available width.
General application text retains the existing UI faces first, with the bundled
fallback faces below for missing glyphs. Calibrated scope screen typography
retains its measurement font.

The prior Barlow font binaries are no longer bundled. [OFL-Barlow.txt](OFL-Barlow.txt)
is retained as a historical source notice; it is not part of the current app's
font-license output or package.

## Unicode fallback faces

The following unmodified static Regular faces are appended, in the order shown,
after every existing face in the Proportional, Monospace, Panel Print and Panel
Heading families. Existing Latin typography retains priority. Noto Sans CJK JP
uses Japanese CJK glyph forms; Noto Sans Symbols 2 supplies miscellaneous symbols
including the control-knob character `🎛`; Noto Music supplies musical notation
including `𝄞`. Emoji fallback is monochrome. These faces improve coverage; they
do not promise every Unicode character, emoji sequence, or complex-script shaping.

Noto Sans CJK JP is the static Regular OTF, with OS/2 weight 400. The Google Fonts
`NotoSansJP[wght].ttf` variable face was deliberately not bundled because its
default is Thin (100), and the egui font installation API does not select axes.
The actual egui/ab_glyph parser and required per-face glyph coverage are tested.

| Font | Bytes | SHA-256 |
| --- | ---: | --- |
| `NotoSansCJKjp-Regular.otf` | 16,467,736 | `68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5` |
| `NotoSansSymbols2-Regular.ttf` | 1,233,128 | `7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21` |
| `NotoMusic-Regular.ttf` | 124,336 | `aeb2998c380f5807cda77e1d94e4ac596429cb74d4fffb8da279d4f3144c0eae` |

The added font data totals 17,825,200 bytes (about 17.0 MiB), embedded in the
executable. No fonts are loaded from the host system or downloaded at runtime.

Pinned primary sources:

- CJK JP: [notofonts/noto-cjk at `523d033d6cb47f4a80c58a35753646f5c3608a78`](https://github.com/notofonts/noto-cjk/blob/523d033d6cb47f4a80c58a35753646f5c3608a78/Sans/OTF/Japanese/NotoSansCJKjp-Regular.otf).
- Symbols 2: [google/fonts at `b7c8331cf620346575487ef8db0e2c729f014675`](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf).
  Its pinned Google Fonts metadata records upstream notofonts/symbols revision
  `25b00f0d3f40873a005287514ba2a48558655314`, release `NotoSansSymbols2-v2.008`.
- Music: [google/fonts at the same revision](https://github.com/google/fonts/blob/b7c8331cf620346575487ef8db0e2c729f014675/ofl/notomusic/NotoMusic-Regular.ttf).
  Its pinned metadata records upstream notofonts/music revision
  `4a20607c896d77aebe4d6983a949adad705e05b7`, release `NotoMusic-v2.004`.

All three use the SIL Open Font License 1.1. Original copyright notices are
reproduced in [NOTO-FONTS.txt](../licenses/NOTO-FONTS.txt), and the following
license files are exact copies from the pinned source revisions:

| Local license | Upstream path | SHA-256 |
| --- | --- | --- |
| [OFL-NotoSansCJK.txt](../licenses/OFL-NotoSansCJK.txt) | `notofonts/noto-cjk/LICENSE` | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |
| [OFL-NotoSansSymbols2.txt](../licenses/OFL-NotoSansSymbols2.txt) | `google/fonts/ofl/notosanssymbols2/OFL.txt` | `b118dd41337806a5d4797052c77caf3bd096aed783e5eb21b4d11154351e1ac0` |
| [OFL-NotoMusic.txt](../licenses/OFL-NotoMusic.txt) | `google/fonts/ofl/notomusic/OFL.txt` | `1679cb60544990b4c2445320c6d5298e4bb0c08ff00dbb5a526f3793fdaa1e6f` |

The macOS packager already copies every file in `assets/licenses` into
`Contents/Resources/Licenses`; `--font-license` prints the Rajdhani license plus all
three Noto license texts and the Noto notice above.
