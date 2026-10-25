# Stock Unity

Development addition, not installed. `stock_unity` recreates the hidden legacy
VCV Fundamental **Unity** (model slug `Unity`): two six-input unity-gain mixers
with sum/average switches, inverted outputs and level lights, so patches that
still use it import and sound as they did.

## Play it

Choose **Stock Unity** under **Mixing**, or open
`examples/legacy-vca2-unity-duet.json`, where it sums both channels of
[Stock VCA-2](STOCK_VCA2.md).

- Each section adds its six inputs at unity gain. **MIX** carries the sum and
  **INV** the same signal inverted.
- **AVERAGE** divides by the number of patched inputs, so the result stays at
  the level of one input. A patched input counts even at 0 V; unpatched inputs
  never count. With nothing patched, MIX is 0 V either way.
- **MERGE 1 + 2** joins the sections: both sets of outputs carry the
  twelve-input sum, and each section's AVERAGE still applies to its own outputs
  over all patched inputs. MERGE is saved with the patch.
- Five lights per section show the mix level against 10 V: −24, −18, −12 and
  −6 dB (green, green, green, yellow) fill across their 6 dB band, and the red
  0 light comes on at 10 V and above. They rise at once and fall back over a
  fraction of a second.
- There is no jack normaling: an unpatched input is simply 0 V. Unity is
  monophonic: it reads the first channel of each cable and every output is one
  channel. Initialize returns both sections to sum and turns MERGE off; the
  lights keep falling from where they were. Unity has no bypass route, so
  bypass silences its outputs.

## Source contract

Reference: [pinned `src/Unity.cpp`](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Unity.cpp)
(byte-identical at Fundamental 2.6.4, `e0ade9a`), listed `"hidden": true` in the
pinned `plugin.json`, with Rack SDK 2.6.6 `dsp::VuMeter2` and
`dsp::ClockDivider`.

Inputs `ch1_1`..`ch1_6`, `ch2_1`..`ch2_6` and outputs `mix1, inv1, mix2, inv2`
keep the source order. Parameters `average1`, `average2` are the source's
`AVG1/AVG2` switches (0 = sum, 1 = average). `merge` (0/1) is a durable alias of
the source's `merge` module-data Boolean, not a Rack parameter index.

Per frame, in f32 and source order: each section adds the first-channel
voltages of its six inputs from +0 and counts connected inputs (channels > 0);
with merge, `mix1 += mix2`, `mix2 = mix1` and the counts combine; a section
divides by its count when the count is positive and `round(switch) == 1`;
`MIX = mix`, `INV = -mix` (so an empty mix gives +0 and −0). Each section's
peak meter takes `|mix / 10|`, rises instantly and otherwise moves
`v += (x - v) * 30 * sampleTime`. Every 256th frame the lights take
`getBrightness(0, 0)` (clip) and `getBrightness(-6(j+1), -6j)` for j = 1..4 from
`20 * log10(v)`. The source resets only `merge` in `onReset`; parameters return
to defaults through Rack's reset; meters, divider and lights continue. All
arithmetic applies Rack's flush-to-zero contract, so a decaying meter flushes to
zero exactly as in Rack. Host policies: nonfinite input becomes 0 V, finite
input is bounded to ±1,000,000 V; meter state carries across edits and sample
rate changes, with the new sample time.

VCV import maps `Fundamental/Unity/2.6.4` to `stock_unity`: parameters 0/1 to
`average1`/`average2`, and module data `merge` to `merge` (only a JSON `true`
merges, as a fresh module's `dataFromJson` behaves). A switch saved between
positions leaves the module unresolved.

## Reference qualification

[Runner, sources and cases](../scripts/fundamental_legacy_reference/README.md),
attempt `attempt-003` (2026-10-24, macOS ARM): **24 Unity runs (11 declared
cases at 44.1, 48 and 96 kHz where timing matters), 534,444 frames, 9,620,031
compared values (all four outputs, ten lights, both meter states, the divider
clock, the merge flag, reset results and saved data), 0 mismatches, no
exclusions.** Cases cover unpatched defaults, each input alone, all 64
connection masks per section in sum and average with polyphonic cables, merge
with every switch combination, a four-second meter release into the
flush-to-zero range, slow ramps through every light band, reset, nine saved-data
shapes, special bit patterns and noise with changing modes. Counterfactual:
dropping flush-to-zero from the meter release mismatches 129,845 values of the
release case.

Bounds as for [VCA-2](STOCK_VCA2.md#reference-qualification). Reset in the
reference applies each genuine `ParamQuantity` default and then the genuine
`Unity::onReset()`, because `Module::onReset(ResetEvent)` needs a running app.

## Host validation

Core unit tests (`stock_unity.rs`: validation, sum/average/inverse, merge and
saved data, first-channel reads, light refresh and adoption), engine tests
(`tests/fundamental_legacy.rs`: sections, merge, lights, initialize, bypass,
catalog, example), the shared no-heap test in `engine_v2_contract`, MCP/history
and import tests, shared panel tests (print, toggles, lamps and sockets at five
scales, toggle edits against the current revision, lamps following measured
state) and [normal and dim captures](../design/reviews/fundamental-legacy-v1/README.md).

Not established: native window input, audio-device playback or complete Rack
host parity.
