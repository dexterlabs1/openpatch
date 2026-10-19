# Recall Random

Recall Random makes related rhythms and pitches that can repeat, change, and
shuffle. It is a native OpenPatch instrument based on the open-source Marbles
DSP. It appears in **Custom** and **Modulation**, with Clock, Random, Sequencers
and Quantizers tags. This development addition is not installed or in the
published 0.3.0 downloads.

## Make a repeating phrase

1. Add Recall Random, an oscillator, an envelope and a VCA.
2. Connect **X1** to oscillator pitch and **T1** to the envelope gate.
3. Open **RHYTHM** and enable **T repeat**. Open **X / Y** and enable **X repeat**.
4. Adjust **DEJA VU** and **LOOP LENGTH** to explore repeating and changing phrases.
5. Try the seven rhythm models in **RHYTHM** and the clock/range choices in **X / Y**.

The rhythm section has T1, T2 and T3 gate outputs. X1, X2, X3 and Y output
voltages. X spread/bias/steps shape the pitch distribution; Y has its own
controls and clock divider. Seed is an exact integer in the RHYTHM menu.

The panel and module-browser thumbnail use the same renderer. Large knobs keep
normal value entry, drag bubbles, undo grouping and visible MCP parameter motion.
The small display shows the saved scale weights, not an invented audio waveform.
Output lamps show actual callback voltages. The overflow warning reports a real
input-queue overrun.

Open the [quiet repeating-phrase example](../examples/recall-random-repeating-phrase.json)
to hear this routing with a filtered bass voice and Studio Mixer.

## Edit a scale

Open **SCALES / RECORD**. Choose an active scale and an edit scale, numbered
1–6 in the panel (0–5 in the API). Each has a positive repeat interval, up to
16 increasing degree voltages and individual weights from 0–255. Keep at least
one weight at 255. Negative degree voltages are supported.

**Apply scales** commits all changes as one undoable edit. If another edit
changes the patch while the menu is open, the draft stays visible but cannot
overwrite it; use **Reload saved scales** to start from the current document.
Scales and the active selection survive presets, undo/redo and save/reopen.

## Record notes into a scale

Patch pitch voltage into **REGISTER CV** and run audio. X CLOCK edges mark the
notes; without a cable the recorder follows internal T2. Choose an edit scale,
then **Record**, **Stop**, and **Apply capture**. At least two different degrees
are required. **Cancel** discards the recording, including a completed result.

Recording is temporary. Only Apply capture writes scale content and history.
An edit after Record makes that result stale, even if the edit is later undone.
The audio callback must acknowledge the latest command before a result can be
applied. A result can be applied only once, including when its scale matches the
saved scale. Stopped audio cannot service queued recording commands.

MCP clients use `capture_recall_random_scale` with `action` set to `begin`,
`stop`, `cancel`, `inspect` or `commit`. `begin` accepts `slot` 0–5 and captures
the starting revision. Weighted manual edits use `set_module_content` with the
catalog's typed `recall_random` content schema. These use the same document,
revision checks and undo history as the panel.

## Timing and source boundaries

The core runs at 32 kHz in five-frame blocks; the host adapter supports
8–192 kHz. CV uses one-source-frame interpolation, and T gates include the
firmware's two-source-frame delay. At a 32 kHz host, the first raw CV sample is
published at frame 6 and the first raw T sample at frame 7, counting from zero.
Native cables add their separate host-frame delay.

Clock edges use a fixed 64-snapshot chronological queue. Overflow reconciles
latest physical levels and lights the warning; it does not preserve arbitrary
above-bandwidth pulses. RESET is applied once per completed source block;
multiple reset rises within that block coalesce, as required by the source API.

Compatible graph replacement preserves RNG, loops, clock/interpolation state,
pending edges and recording, while keeping the receiver's saved controls/scales.
Initialize, seed changes, fresh reopen and sample-rate changes start fresh.
Bypass freezes runtime and outputs zero. Runtime loops and recordings are not
silently written into patch files.

This models the source algorithms, with ideal voltages and explicit native
corrections for undefined endpoints and empty Y scale masks. It does not model
hardware entropy, ADC/DAC calibration or board timing. See the detailed
[contract](RECALL_RANDOM_CONTRACT.md) and [source reference](../scripts/marbles_reference/README.md).

## Validation

The refreshed genuine C++/audited C++/Rust comparison passes 26 cases and
288,485 observations; three original undefined-source cases remain separate
from their native corrections. An additional Engine fixture compares 13,000
source frames across all seven rhythm models and six scales at the declared
latency. Focused tests cover lifecycle, bounded edge capture, weighted content,
recording authority, real callback publication, presets and save/reopen. The
callback allocation/deallocation checks cover five host sample rates.

The normal and dim panel reviews are [here](../design/reviews/recall-random-v1/README.md).
[Full macOS CI](https://github.com/dexterlabs1/openpatch/actions/runs/39191841453)
passes, and PR #37 is merged. No installed-build or hardware-equivalence claim
follows from these checks.
