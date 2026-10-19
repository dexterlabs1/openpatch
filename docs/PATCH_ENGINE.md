# Patch Engine

Three words (genre · mood · texture) plus a seed compose a complete, laid-out patch: key, progression,
voicing, drums, bass, chords, melody, song form and sound design. Source: `crates/openpatch/src/patch_engine.rs`
with instrument designs in `crates/openpatch/src/patch_engine/instruments.rs`. CLI:
`openpatch patch-engine GENRE MOOD TEXTURE [--seed N] [--budget PCT] [--info|--reveal|--showtime]`.

## Instrument designs

Each voice role draws from a registry of designs that spans the catalog instead of a fixed subtractive voice.

| Role | Designs |
|---|---|
| Chords | classic keys, piano, pluck, organ, saw pad, square pad, dub stab · FM keys and wavefold/metallic stabs (`macro_voice` with its low-pass gate) · swarm pad and harmonic drawbars (`macro_voice` + envelope on LEVEL) · evolving wavetable pad (`wavetable_vco`, position on the motion LFO) · struck glass (`modal_resonator`) · analog plucks (`rack264_vco` → `rack264_vcf`) |
| Pad layer | sustaining pad designs only, contrasting with the chord design |
| Bass | classic multi-VCO voices · Fundamental 2.6.4 saw/square through its ladder · capacitor-ramp saw through the circuit `ladder_vcf` · FM and wavefold `macro_voice` basses; every design keeps the sine sub |
| Lead | classic oscillator · FM bells, wavefold lead (percussive) · modal bells · swarm, harmonic, analog and wavetable leads (sustained) |

Designs carry character tags (warm, glassy, metallic, analog, evolving, …). Genre, mood and texture weight the
tags; the genre's classic voices get a bonus so a genre keeps its identity; each design's listening `rating`
(one-variable swap tests: one design changed inside a patch the listener liked) scales its weight, so favourites lead
and passed-over designs stay rare; the seed picks. Hooks and
counter-lines only take sustaining designs. The pad layer avoids repeating the chord design.

## Melody and bass lines

- Hooks are a 3–5 note motif over two chords, developed over the next two (carried onto the new harmony, inverted
  or thinned). Notes come from the mode bent to each chord: strong beats and phrase ends land on chord tones (3rd,
  5th, 7th or 9th more than the root), weak beats may pass through scale tones, never a semitone above a chord tone.
- Bass lines keep at most one octave jump per bar, put the odd fifth off the downbeats, slide less, and may step
  into the next chord's root from a scale tone.

## CPU tiers and plan size

`module_cost` holds measured offline costs ([evidence](benchmarks/patch-engine-module-costs-m1max.json),
`scripts/patch_engine_costs.py`). `macro_voice` is costed per engine (FM ×4 ≈ 8.6 % of an M1 Max core; Analog
×4 ≈ 1.3 %).

- Full builds (default on macOS) use any design.
- `--budget PCT` (Light mode in the app): when the full build is over budget, expensive designs swap to their
  classic stand-ins (FM keys → keys, swarm pad → saw pad, circuit ladder bass → four-pole saw bass, …), then
  optional layers drop in `DROP_ORDER`. Each attempt recomposes the same piece, so the result is deterministic.
- Every build stays inside one construction plan (≤ 256 edits); larger builds drop optional layers.

## Quality checks

`scripts/patch_engine_qa.py` renders offline WAVs with the release CLI (nothing plays through speakers):

- `stems N` then `trims FILE`: per-design levels relative to the kick, the source of `trim_db`.
- `gate N`: both drops of N builds; exits 1 on clipping, peaks over 0.98, silence or RMS outside −30…−8 dBFS.
- `variety K`: K word combinations × 6 seeds; exits 1 when two seeds of the same words render near-identical
  feature vectors (loudness, brightness, low weight, noisiness, onset rate, dynamics, width).

Tests (`cargo test -p openpatch --lib patch_engine`) check every kind/port/parameter against the catalog for
every design, construction-plan limits, light budgets, reproducibility and the full word matrix.
