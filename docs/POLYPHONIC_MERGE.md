# Polyphonic Merge

Merge assembles the first channel of each of its sixteen inputs into one
polyphonic output. Input 1 becomes output channel 1, input 16 becomes channel
16, and empty slots remain zero. Extra channels within an input are ignored.
An occupied input carrying 0 V still contributes to the automatic width.

The **CHANNELS** dropdown offers Auto and explicit counts from 0 through 16.
Auto uses the highest input slot whose processed frame has at least one channel.
Explicit zero produces a zero-channel output even while inputs remain active.

## Saved configuration and MCP

OpenPatch preserves the existing `channels` field and its earlier meaning:

| Saved value | Selection |
|---|---|
| Omitted or `0` | Auto |
| `-1` | Explicit zero channels |
| `1` through `16` | Explicit channel count |

The negative sentinel extends the existing format without changing old patches
or making older positive-count MCP setters ineffective. This encoding differs
from Rack's saved data; a future Rack importer must translate it explicitly.
Fractional, nonfinite, and out-of-range values are rejected. Other modules'
channel parameters retain their existing domains.

Use `set_parameter` with `id`, `parameter: "channels"`, and the saved value.
The native dropdown uses the same revision-checked command. A changed selection
is one undo step. Presets, duplication, project files, and recovery carry this
configuration as an ordinary saved parameter. Initialize selects Auto;
Randomize leaves the channel selection alone.

## Measured count display

**OUTPUT CHANNELS** shows the processed output width, including a genuine zero.
An em dash means there is no valid current observation. The Auto choice can
show the measured automatic width even when an explicit output width is selected.
Neither count is inferred from cable topology or voltage magnitude.

With a live audio backend, `inspect_performance` exposes
`performance.modules.<id>.lights.output_channels` and `automatic_channels`.
Both are optional integers from 0 through 16. They describe the same processed
frame and publication revision. `lights.count` remains zero: these are count
readouts, not invented signal lamps. Headless inspection does not fabricate
live readings, and the browser preview has no live count.

The callback publishes counts through the existing coherent atomic transport.
Pending revisions and replaced instances remain unavailable until their actual
graph publishes. Backend close clears availability. Compatible same-ID/kind/rate
edits retain observations until processing supplies new ones; the receiving
configuration determines the next output. Fresh instances and rate changes
reset observations. OpenPatch's bypass freezes Merge processing and its counts.

## Reference and limits

The functional target is the [pinned Fundamental Merge class](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Merge.cpp).
OpenPatch uses original Rust code and original panel artwork. It clears inactive
output storage; the upstream class can retain written values outside its active
channel count. Active signal interpretation is the comparison target.

The [reference harness](../scripts/merge_reference/README.md) compares the actual
pinned class with production Engine output, with fixtures frozen before the
first comparison. The [recorded macOS ARM run](benchmarks/merge-reference.json) passed 16,416 rows at 44.1, 48 and 96 kHz: all output/automatic counts and 132,081 active voltage values matched exactly. All 769 retained evidence files and the 72-file production closure passed independent hash checks. Native capture qualification is pending.
Separate host tests cover cable delay, adoption, bypass, allocation-free callback
publication, persistence, and revision handling. These checks do not establish
hardware, native input/device, unrestricted headroom, or complete Rack import
compatibility.
