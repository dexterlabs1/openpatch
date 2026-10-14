# Scope primitive reference

Run `python3 scripts/scope_reference/run.py --sdk /path/to/Rack-SDK`.
The default report goes to `/tmp/openpatch-scope-reference.json`.

This headless harness compares an original acquisition driver using genuine SDK
Input and SchmittTrigger primitives with the production Rust scope source using
minimal host adapters. It fetches and hashes the pinned Fundamental source for
provenance, without compiling, copying or vendoring that source or artwork.

The two drivers receive identical finite streams with 0–16 independent channels,
mid-bin retirement, external and normalled triggers, threshold hysteresis, XY,
and display-only adoption. Every sample compares bin/write positions and raw
passthrough widths, plus a hash of every completed min/max bin and output sample.
Production rebuilds with display edits every 37 samples must match uninterrupted
reference acquisition. Bin widths are 2, 4 and 16 samples using exact binary time
values at 65536 Hz; this deliberately avoids asserting identical TIME parameter
mapping or exponential approximation.

This is primitive/scheduling evidence, not execution of the full upstream module,
physical output-port bookkeeping, graph timing, lifecycle, GUI or MCP. Empty bins
are represented by their infinity bit patterns. Hash comparison has theoretical
collisions; production unit tests inspect exact representative bin values.
