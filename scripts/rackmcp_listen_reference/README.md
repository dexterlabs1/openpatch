# RackMCP Listen offline reference

This original probe links the **untouched** RackMCP
`src/core/listen_analysis.cpp` / `.hpp` frozen at commit
`285e84c5ff2d3ecef25890da07f578af6a0c5ca6`. That implementation is by
Zachary Scheffler, GPL-3.0-or-later. The production Rust adaptation carries
its own source attribution and uses the existing core third-party notice.
The probe drivers are original MIT code.

Run only under the root agent's serial build/reference ownership, after
registering `pub mod rackmcp_listen_analysis` in `openpatch-core`:

```sh
python3 scripts/rackmcp_listen_reference/analysis.py \
  --output native-test-output/rack-control-import/listen-analysis-001
```

The output must be new. Inputs, source copies, Cargo manifests/lockfile,
toolchain versions/flags and fixture hashes are frozen before compilation.
The C++ executable compiles the frozen untouched source, while the Rust
probe links the actual production core library and calls its public API.
Source hashes are checked after building, and both executable hashes and
the linked core-library hash are written before either probe analyzes a
sample. The supplied pin is the audited reference identity; actual source
hashes identify the files used. No claim of independent Git pin verification
is made by this script.

Fixtures cover silence, DC, no inputs, right-only, absent right, equal and
opposed stereo, a host poly-sum-equivalent bus, clipping at/beyond 10 V,
pulse edges, a four-second 120 BPM rhythm, and 440 Hz tones at 44.1/48/96 kHz
for 0.1 and 1 second. Each input is generated **once** as interleaved little
endian float32 bytes shared by both binaries. This is an offline analysis
proof, not a test of the capture callback or polyphonic passthrough.

Every source result field, validity flag, dBFS conversion, nearest-note
name/cents, band and float/double bit pattern is compared exactly. Both
stereo WAV bytes and a separate absent-right WAV export are compared
exactly. The raw codec probe passes the actual R buffer even when R is
marked absent for analysis. `encode_wav_float_stereo(left, None, rate)`
explicitly requests the source writer's L-to-R duplication policy; the
host must choose that policy for export only. Analysis never duplicates L.

Exit 0 means exact agreement; 1 means raw mismatches are preserved in
`exact-differences.json`; 2 means execution/build/setup failed. No tolerance,
patching, normalization or oracle replacement is applied. Root must inspect
any libm differences before deciding the next action. Probe subprocesses
have 90-second deadlines (builds have separate bounded deadlines); full
outputs stay in the evidence directory.

The Rust API rejects unequal/empty/nonfinite inputs, captures longer than
30 seconds, rates outside Core's 8000–192000 Hz patch range, and nonfinite
mono sums or resulting metrics. These native boundary guards are stricter
than the standalone source function. Analysis allocates and must run on a
worker thread. Invalid source metrics retain their source flags and zero
values; conversion of zero volts to dBFS intentionally returns negative
infinity, so hosts must choose explicit JSON handling when using it.
