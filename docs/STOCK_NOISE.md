# Stock Noise

`stock_noise` is an original Rust implementation of the seven output roles in the
[pinned Fundamental Noise class](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Noise.cpp).
It is a separate module from [`colored_noise`](COLORED_NOISE.md): existing
`colored_noise` patches retain their level and seed controls, filters, random
streams, connection-independent advancement and saved behavior. Stock Noise
has seven mono outputs in source order—`white`, `pink`, `red`, `violet`, `blue`,
`gray`, `black`—and **no parameters, inputs or module lamps**. Select an output
or connect its jack to consume it. Its source-backed panel marks describe each
generation path; they are neither live signal meters nor extra controls.

## Generation and state

Generation follows consumer groups rather than advancing all histories every
sample. A white normal draw occurs when any of white, red, violet or gray is
consumed. That sample drives white directly, red's fixed low-pass history,
violet's first difference or gray's block input as applicable. Red, violet and
gray each advance their own history only while their own output is consumed.
Pink's eight-register Voss generator runs when pink or blue is consumed; blue takes
the first difference of pink and advances its prior-pink state only while blue
is consumed. Pink's register updates take uniform draws before black's one
uniform draw, so connecting pink can change black's subsequent samples.
Disconnected histories freeze and resume rather than silently catching up.

Gray collects **1,024 connected samples** before its first transformed block
appears, on connected sample 1,024. Its FFT weighting uses the source's
`sample_rate × k / (2 × 1024)` frequency for bin `k`, a half-bin interpretation
relative to the usual `sample_rate × k / 1024`. DC and Nyquist are zeroed.
The buffered gray input, output and frame cursor freeze while gray has no
consumer. The source loop touches an upper scratch half that the ordered real
inverse FFT does not consume. The reference runner initializes that otherwise
indeterminate scratch; this is a **declared erratum**, so the comparison is not
to completely unmodified source.

Black maps its uniform draw to `10U − 5` volts. The other outputs are source
scaled by `5 / √2` and have no generic peak clamp. A connected output is mono;
an unconsumed output has no active channel in the native host. Actual cables,
the selected output and temporary analysis taps are consumers. Version-2 cable
transport and raw port clearing follow the host's ordinary graph rules.

There is no source `onReset` override or saved control to restore. Initialize
preserves the random provider, color histories and gray block state. Compatible
same-incarnation, same-rate graph adoption retains those histories when the
draw-provider configuration also matches; a new incarnation or changed
controlled tables starts fresh. Bypass freezes processing and the host handles
connected output voltages. Ordinary OpenPatch playback uses a per-module
seeded stream. Its sequence is an OpenPatch policy, not a claim about Rack's
uncontrolled process-global random generator.

## Recorded reference and limits

The [independently reviewed declaration](benchmarks/stock-noise-host-reference.json)
froze the input matrix, pinned source and genuine Rack 2.6.6 SDK provenance
before the first reference or production execution. The actual class prefix
was compiled in a separate SDK probe with three controlled random-draw call
substitutions and the one-line unused-scratch initialization. The separate
release production probe ran the native Core graph. The
[SDK result](benchmarks/stock-noise-sdk-result.json.gz)
and [paired result](benchmarks/stock-noise-paired-result.json.gz)
both pass the declared gate: **424 cases, 17,689 events and 18,113 snapshots**
across 44.1, 48 and 96 kHz, including all 128 output-consumer masks at each
rate. No declared startup, boundary or no-sample snapshot was discarded.

The gate compares source-ordered draw logs, complete port and sink state,
consumer topology, freeze/adopt/recreate behavior and internal histories.
Each side passed **22 independent float64 gray DFT checks**. Six non-gray
outputs and non-FFT state use exact binary32 comparisons; FFT-derived gray
samples use the fixed finite bounds of `2.5e-4` raw units or `5e-4 V`, plus
`3e-5 × max(|actual|, |expected|)`. Three scratch variants—upper-half `+1`,
upper-half `−1` and packed-range-only weighting—each passed 1,025 paired
rows on the retained gray boundary subset. This supports the narrow erratum;
it does not establish arbitrary behavior for indeterminate scratch values.

The passing result establishes behavior for the pinned class, declared source
transforms, installed SDK and **controlled** binary32 draw tapes in this bounded
matrix. It does not establish identity to Rack's uncontrolled global PRNG,
an official distributed plugin binary, a hardware noise source or every
possible wiring and stream length. The companion
[host reference report](benchmarks/stock-noise-host-reference.json) records
the source/host evidence; the
[release qualification record](benchmarks/stock-noise-release-qualification.json)
records the combined-source release gates: all seven frozen reference
regressions pass, alongside 1,013 desktop tests, 113 release-executable MCP checks,
strict Clippy, macOS ARM/Linux/Windows CI and 145 validated native Linux captures.
The ARM bundle passes static signature, architecture and embedded-resource checks.
Local Mac GUI and physical audio-device playback were not exercised.
