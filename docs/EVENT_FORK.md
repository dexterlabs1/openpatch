# Event Fork

Development candidate, not installed. Two mono Bernoulli gate routers derived
from the [Branches firmware](https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/branches/branches.cc),
with an independent OpenPatch panel and explicit native host policies. This is a
distinct router, not another random-voltage generator. The [hardware manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/branches/manual/)
describes the original normaling, toggle and latch interactions.

## Patching and controls

Each section has GATE and probability CV inputs and A/B outputs. Unpatched
GATE 2 reads GATE 1; a physical cable breaks that normal even if its source has
zero channels. All inputs read channel zero, and all outputs are mono 0/5 V.
The native Schmitt detector starts low, goes high at 1 V and low at 0.1 V.
The first high input fires a decision. Nonfinite input volts read as zero.

P(B) sets the probability of choosing B. Native CV adds voltage/5, clamps to
0..1, then uses `floor((1-pB)*255)` as the original 8-bit ADC-table index.
The actual source endpoint plateaus remain. TOGGLE changes the interpretation:
an A decision stays on the previous output, and B switches it. The shared RNG's
low/high words feed sections 1/2; both histories remain independent. SEED is an
integer from 1 through 16,777,215 and is excluded from knob randomization.

GATED outputs preserve detected input duration. LATCH keeps the chosen output
after the gate falls. The original firmware quirk is retained: turning latch
off while the input is already low leaves the held output in place until a
later rising decision and unlatched falling edge. Toggle/latch are saved native
parameters; physical switch debounce/long-press and EEPROM timing are not modeled.

## Timing, state and telemetry

One native host frame executes one complete source scan, including unconditional
LFSR advancement on idle frames. This is not an AVR CPU/ADC timing simulation;
seeded sequences depend on sample rate and elapsed frames. Ordinary native
cables add one host frame of delay. A raw scan has no invented additional delay.
Input detector, RNG, previous outcome and held outputs survive compatible
same-rate/same-seed graph adoption. Receiving controls apply immediately without
creating an edge. A changed seed/rate, Initialize or fresh reopen restarts state.
Bypass emits mono zero and freezes runtime. The callback paths allocate and free
no memory and use no locks or filesystem operations.

Four native lamps report actual routed A/B output volts divided by 5 in the
first brightness component. They are dark without telemetry. The original
bi-color decision LED/tail remains part of the raw oracle, separate from these
native output indicators. Rack and browser share the same panel renderer,
controls, font, sockets and seated black screws. The module belongs to Custom
and Modulation, with Random/Gates/Logic search tags.

## Source qualification and example

The [reference harness](../scripts/branches_reference/README.md) executes the
unchanged pinned `branches.cc` loop and GPIO functions, using host AVR stubs and
a finite scan scheduler. Seventeen raw fixtures cover 11,677 scans, every ADC
entry, simultaneous/staggered edges, shared RNG, zero/high seeds, threshold
equality, held gates, mode changes and LED tails. A separate 513-scan fixture
feeds the actual native Engine golden test. Original, UBSan/bounds-audited and
Rust records match exactly. ADC scheduling, buttons, circuitry, electrical
levels and physical hardware accuracy are outside that bounded source claim.

[Routed duet example](../examples/event-fork-routed-duet.json): A/B select two
quiet, filtered voices a perfect fifth apart; section 2's latched decisions
change the second voice's filter through an explicitly attenuated Frame Scene.
The notes are readable at the supported preview scales.

GPL-3.0-or-later source attribution, corresponding-source retrieval and bundled
notices are retained. Original OpenPatch source keeps its MIT notices; combined
binaries containing this firmware adaptation are GPL-3.0-or-later.

[Actual shared-panel and thumbnail review](../design/reviews/event-fork-v1/README.md).
The 14-second 48 kHz stereo offline example peaks at 0.050995, RMS 0.008657,
with zero clipped samples. This is an offline numerical check, not a listening
or audio-device test.

Done checks: genuine raw source/audit equality; native graph golden and
normaling/lifecycle/callback memory checks; shared UI/history/seed/telemetry and
default-renderer checks; actual stdio MCP capture/history/atomic rejection/preset/
save/reopen; normal/dim shared panel review and offline example render. Family-level macOS CI remains the release
gate. No new installation, native-window or audio-device qualification is claimed.
