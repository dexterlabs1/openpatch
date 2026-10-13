# Circuit optimization: Apple M1 Max

Observed 2026-10-13. This compares OpenPatch `98edb73` with `a9144a0`, not
VCV Rack. The [raw JSON](circuit-optimization-m1max.json) includes complete
patches, binary hashes, all paired timings and channel measurements.

## Method and results

Both release binaries used Rust 1.99.0 on macOS 26.7.1, Apple M1 Max (10 logical
CPUs). Each workload has five randomized/interleaved baseline/candidate pairs,
48 kHz sampling, 128-frame blocks and two audio seconds per timed call. Each
benchmark warms 2,048 frames before timing. Compilation and RPC are excluded;
DSP processing and checksum reduction are included. All active voices run even
though the benchmark output adapter selects channel zero.

| Workload | Before µs/frame | After µs/frame | Time reduction |
|---|---:|---:|---:|
| Ladder, 1 voice | 2.2668 | 1.5820 | 30.21% |
| Ladder, 16 voices | 35.8308 | 24.8707 | 30.59% |
| VCA, 1 voice | 1.0825 | 0.7010 | 35.24% |
| VCA, 16 voices | 16.9185 | 10.8331 | 35.97% |
| Full voice, 1 voice | 3.3390 | 2.2598 | 32.32% |
| Full voice, 16 voices | 53.3542 | 35.7752 | 32.95% |
| Ladder with audio-rate CV, 1 voice | 2.3491 | 1.6662 | 29.07% |
| Ladder with audio-rate CV, 16 voices | 37.2840 | 26.2224 | 29.67% |
| VCA with audio-rate CV, 1 voice | 1.1401 | 0.7451 | 34.64% |
| VCA with audio-rate CV, 16 voices | 17.1480 | 11.0735 | 35.42% |

Times are medians. The full reference voice reaches 9.22× offline realtime with
one voice and 0.582× with sixteen. Sixteen full circuit voices remain slower than
realtime. Paired median speedups are 1.481× and 1.491× respectively. These are
medians of paired ratios, whereas time reduction compares the separate medians.
Min/max ranges and unrounded values are retained in the JSON.

## Fidelity and implementation

All 50 paired checksums match exactly. All ten all-channel analyses match
exactly, excluding the control revision, with zero observed solver failures
during their 0.5-second windows. Four frozen scalar tests additionally compare
exact callback/output bits, solver state and diagnostics across ring wraps,
control-history draining, sample rates, nonlinear feedback and forced failure
recovery. Independent physical-response, state-transfer and allocation tests
remain in place. Exact-bit checks apply within a build/platform, not across
different math libraries.

The implementation removes repeated kernel copies, mirrors FIR histories,
accumulates independent interpolation phases together, and reuses already
evaluated Newton currents. It retains all 513 coefficients, 8× oversampling,
64-sample latency per circuit, circuit equations, convergence tolerance and
bounded failure behavior. ARM64 disassembly of the measured candidate contains
independent two-lane f64 multiply/add instructions in both circuit functions,
with no memcpy/memmove calls in those functions. Representative instructions are
in the JSON; this describes this binary, not every compiler or target.

Mirrored storage adds 5,144 bytes per oversampler. Version 2 preallocates sixteen
voices plus a template per circuit, so a graph containing one ladder and one
VCA adds 174,896 bytes (about 171 KiB), even with one active voice. State copies
therefore transfer more history. Process RSS and callback-tail cost during
crossfades were not measured.

## Reproduce and limits

Build separate release CLI binaries from the two revisions, then run:

```sh
python3 scripts/compare_engines.py \
  --baseline /path/to/baseline/openpatch \
  --candidate target/release/openpatch \
  --baseline-source 98edb73 --candidate-source a9144a0 \
  --runs 5 --seconds 2 --output comparison.json
```

This is a single-host offline observation with background load, no core affinity,
and no controlled power or thermal state. Initial load averages were about
7.0/9.5/9.0. Observed ranges are not confidence intervals. No audio-device
callbacks, underruns, UI/GPU load, crossfade tails, hardware latency or VCV
workload were measured. Passing these finite signal checks does not prove all
patches equivalent or establish measured-hardware accuracy.
