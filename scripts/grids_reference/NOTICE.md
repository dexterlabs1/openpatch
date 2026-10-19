The Rhythm Terrain raw kernel and resource tables adapt Mutable Instruments Grids
by Emilie Gillet, copyright 2011–2012; the RNG and integer operations adapt
avrlib by Emilie Gillet, copyright 2009. These source files are licensed under
GNU GPL version 3 or, at your option, any later version. Original notices remain
in every retained source file. GPL text: https://www.gnu.org/licenses/gpl-3.0.html

Exact sources:
- https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/grids
- Its avrlib submodule: https://github.com/pichenettes/avril/tree/276b2887e4110ca913294fcbb313163dfb28a448

source_manifest.json pins every retained source/build-context dependency by
repository commit and SHA-256. The host overlay disables op.h's USE_OPTIMIZED_OP definition, selecting the
genuine portable arithmetic already present in its #else branch. It also
excludes exactly the unused portable U24Sub definition, which cannot compile
upstream because it declares `difference` then reads undeclared `sum`. An
asserted no-call audit covers every compiled source and the full ISR source;
future calls fail compilation. Used U8Mix/U8U8MulShift8/U8U8Mul arithmetic stays
unchanged. Each attempt retains original/overlay SHA-256, the unified diff and
the audit in host-overlay-audit.json. No handwritten expected-value sequencer replaces
the original C++ PatternGenerator, Clock, Random or tables.

The raw host harness uses local AVR program-memory and EEPROM stubs, skips
hardware_config.h's register types while preserving its LED/input bit constants,
and exposes private state for observation. The composite scan mechanically
extracts the exact HandleClockResetInputs function and ticks_granularity array
from the pinned grids.cc. Its input reader is active-low and its MIDI reader is
empty. It fixes the EEPROM power-cycle count at five (normal nonfactory life).
No original production source is overwritten in the retained upstream directory.
