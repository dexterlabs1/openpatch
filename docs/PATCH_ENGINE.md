# Patch Engine

Three words (genre · mood · texture) plus a seed compose a complete, laid-out patch: key, progression,
voicing, drums, bass, chords, melody, song form and sound design. Source: `crates/openpatch/src/patch_engine.rs`
with instrument designs in `crates/openpatch/src/patch_engine/instruments.rs`. CLI:
`openpatch patch-engine GENRE MOOD TEXTURE [--seed N] [--budget PCT] [--energy|--complexity|--weirdness|--space 0..1]
[--length 32|64|128] [--info|--reveal|--showtime]`.

## Genres

Deep House, Dub Techno, Techno, Ambient, Lo-fi Hip Hop, Synthwave, Downtempo, UK Garage, plus five newer genres with
their own tempo, grooves, instruments, melodies and song forms (drum kit, chord colour and mix details come from a
parent genre): **Progressive House** (Pryda-style rolling bass, plucks, long filter builds; parent Deep House),
**Hypnagogic** (Oneohtrix Point Never-style drones, generative arps, collage cuts; Ambient), **Berlin School**
(sequenced ostinato bass, generative arps, no drums; Synthwave), **IDM** (broken beats, generative and riff leads,
collage; Downtempo), **Melodic Techno** (rolling bass, big pads, arps, progressive builds; Techno). **Ambient Piano**
(soft piano figures, slow drifting harmony, no drums) stays after the genre words; genomes saved with its
earlier name, Minecraft, still load and rebuild the same song.

## Controls

Energy (tempo ±5 BPM, percussion density, pad layer), Complexity (ghost bass notes, counter-lines, arp breathing, hat
rolls), Weirdness (collage forms, generative arps, rarer designs: the rating weight flattens), Space (reverb/echo
depth) and Length (the 64-bar form stretched to 32 or 128 bars). The centre (0.5, 64 bars) reproduces every seed.

## Genome, mutate and evolve

A genome is everything that defines a piece: `Genre|Mood|Texture|song.sound.detail[|energy.complexity.weirdness.space.length]`.
The song seed drives tempo, key, harmony, melody, rhythm and form; the sound seed the instrument designs; the detail
seed envelopes, mix and effect settings. A plain seed uses it for all three. Mutations: **sound** (same song, new
instruments and details — drums, bass line and melody stay note for note), **song** (new song on the same
instruments), **evolve** (new details, controls drift by up to ±0.12); **breed** takes the words and song of one
genome and the sounds of another. The panel shows the genome under the build buttons with New sounds / New song /
Evolve, and Recent replays any earlier genome. CLI: `patch-engine --genome CODE [--mutate KIND] [--entropy N]
[--with CODE]`. The genome is also written into the patch's About note.

## Song forms

Each seed picks a form its genre suits: classic (intro · build · drop · breakdown · drop · outro), long layering
(16-bar sections), progressive (30-bar filter build into the drop at 32, breakdown at 48, last drop at 56) or collage
(abrupt 3–8 bar scene cuts, sometimes changing key). Inside the 4-bar loops, per-pass step probability adds ghost
bass notes, breathing arps and hat rolls. The "drift" lead is a recall_random generative arp quantized to the key's
pentatonic. Effects add a mono tape echo with wobbling time (Dusty and some lo-fi/dub/collage builds) and mid/side
widening (Spacious, Liquid, Ambient).

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
- `gate N`: both drops (from `--info` `drops`) of N builds; exits 1 on clipping, peaks over 0.98, silence or RMS outside −30…−8 dBFS.
- `variety K`: K word combinations × 6 seeds; exits 1 when two seeds of the same words render near-identical
  feature vectors (loudness, brightness, low weight, noisiness, onset rate, dynamics, width).

Tests (`cargo test -p openpatch --lib patch_engine`) check every kind/port/parameter against the catalog for
every design, construction-plan limits, light budgets, reproducibility and the full word matrix.
