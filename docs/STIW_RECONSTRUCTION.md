# A closer native reconstruction

`scripts/rebuild_stiw.py` reconstructs one inspected “So There I Was” Rack patch
as an editable OpenPatch project. It uses saved notes and automation to drive
native synthesis through the headless MCP server. No recorded backing track or
VCV plugin binary is used.

This is a bounded compatibility tool for one known patch, not a general `.vcv`
importer. The repository includes a source identity/route map and a synthetic
test fixture, not the user's song score or note text. Supply the inspected JSON
separately.

## What maps directly

| Source role | Count | Native implementation |
|---|---:|---|
| Fundamental VCA-1 | 14 | `stock_vca`: saved level and linear/fourth-power response; raw CV normaling and clamping |
| Fundamental ADSR | 5 | `adsr_eg`: normalized A/D/S/R, four signed CV depths, gate and retrigger |
| Fundamental VCF | 4 | `stock_vcf`: saved cutoff, resonance, drive, signed CV depths, lowpass and highpass outputs |
| Core Notes | 4 | `notes`: exact text, bounded to 8,192 UTF-8 bytes |

These digital models have separate reference and qualification contracts:
[VCA](STOCK_VCA.md), [ADSR](ADSR_EG.md), [VCF](STOCK_VCF.md).
They do not establish measured hardware calibration or whole-patch audio
equivalence. The stock VCF applies only to `bass_filter`, `stab_filter`,
`pad_filter` and `hats_highpass`. The lead and riser filters are Surge roles.

## Run the reconstruction

Build the CLI, then choose a new output directory:

```sh
cargo build --release -p openpatch
python3 scripts/rebuild_stiw.py \
  --source /path/to/inspected-source.json \
  --output /path/to/new-rendition
```

The helper starts a private headless `openpatch mcp` process, compiles the graph
through `replace_patch`, saves `stiw-native.json`, then writes the editable
`stiw-native.openpatch` project and `substitution-manifest.json`. It does not
open the desktop app or play audio. Use `--binary` to select a CLI build.
Existing output files are never overwritten.

The identity map checks all 55 module IDs, plugin/model/version identities and
all 83 cable IDs and routes. Reordering module or cable arrays is supported;
changing the topology requires updating and reviewing the compatibility map.
The manifest records the actual input JSON hash separately from the inspected
reference's hash, each source role and each cable translation.

Optional `--score-a` and `--score-b` must be supplied together. They use native
score structure but retain the source parameter value domains for automation;
filter conversion happens once inside the helper. They are not a way to supply
an already converted saved output score.

## Differences that remain audible

Surge oscillators and filters, drums, waveshaping, dynamics, delay and reverb
remain independently implemented approximations. The default Surge lowpasses
use cascaded native SVFs; `--circuit-filters` changes those lowpasses to the
nonlinear ladder approximation. It does not replace the matching Fundamental
stock filters. Stereo lead and riser bandpass roles remain approximations.

Automated VCA level uses a downstream multiplier while preserving the stock
module's CV response. Its gain display excludes that multiplier. Automated
Fundamental cutoff is converted affinely into a 1 V/oct CV lane, with any
independent source cutoff CV scaled separately. Helper stages and one-sample
native cable delays differ from direct source parameter automation. Sequencer
timing/interpolation, mixer behavior and stock VCF noise sequencing also have
declared differences.

The source's zero audio-device gain is replaced by an explicit audition gain;
arrangement mutes and the source audio DC-filter setting remain represented.
The manifest documents these audition changes. Changing the filter, envelope
and VCA implementations can change the mix and headroom, so keep older
renditions separately and inspect offline rendering statistics before playback.

## Verification

```sh
python3 -m unittest discover -s scripts -p test_rebuild_stiw.py
# Enable the additional headless MCP integration test (macOS/Linux):
OPENPATCH_BIN=target/release/openpatch python3 -m unittest discover -s scripts -p test_rebuild_stiw.py
```

The public tests use synthetic score/text content with the known source
identities and routes. They check identity rejection, order independence,
parameter and CV mapping, Notes preservation and helper graph structure. Set
`OPENPATCH_BIN` to the release CLI path to enable the MCP integration test;
on Windows select `target/release/openpatch.exe`. That test also compiles,
saves and reloads the synthetic rendition. This is compatibility validation, not a
comparison against recorded Rack output or a native-device audio test.

The [2026-10-15 qualification report](benchmarks/stiw-stock-reconstruction.json)
records 78 passing Python/MCP tests and a separate reconstruction of the actual
source: 140 native modules, 210 cables and all 55 roles / 83 source connections
accounted for. Two 120.012-second stereo renders at 48 kHz produced identical
PCM16 WAVs from reset, with zero clipped samples (peak 0.46294, RMS 0.09674).
The source and existing rendition hashes stayed unchanged. This establishes
repeatable native output; it does not establish audio equivalence with Rack.
