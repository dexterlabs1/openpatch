# Listen raw module reference

This original MIT adapter invokes the **verbatim constructor and process** from
RackMCP 2.0.0 `ListenModule.cpp`, revision
`285e84c5ff2d3ecef25890da07f578af6a0c5ca6`, by Zachary Scheffler,
GPL-3.0-or-later. Its original header and SPSC ring remain untouched. The C++
probe runs the actual class in the genuine Rack 2.6.6 Engine, including its
configured bypass routes, FPU environment, source modules, cables and sinks.
The Rust probe runs the actual production native Core model, canonical host
ports and graph adoption. For a synchronous SDK bypass command it applies the
production ordered host-lifecycle API after adoption, matching desktop delivery;
direct Core adoption alone deliberately defers net host context to next sample. Neither probe contains the Listen DSP equation.

Root alone runs builds and samples serially after reviewing this declaration:

```sh
python3 scripts/rackmcp_listen_reference/module.py \
  --output native-test-output/rack-control-import/listen-module-001
```

The output must be new. The runner copies and hashes every production Core
source, dependency manifest and lockfile, driver, upstream source, SDK header,
library, generated input stream, compiler flag and toolchain before compiling.
The source prefix extraction removes only plugin/UI integration after
`findListenModule` and its unused plugin include. It checks the SDK against the
existing genuine 2.6.6 attestation. Both executables and the linked Core library
are hashed before either probe processes a sample; a changed production source
aborts the run. Source pin identity comes from the existing audited reference;
the runner records actual source hashes and does not independently authenticate
Git provenance.

The shared integer text input covers all 0–16 source widths at 44.1/48/96 kHz,
each of four physical input and four output consumer masks, armed changes
around sample 255/256, raw signed zero, tiny voltages, source-order cancellation,
16-to-1 retirement/regrowth, absent right, silent frames and bypass transitions.
A 192 kHz case fills the actual original `2^18` ring, rejects eleven new
frames, drains part of the ring and resumes capture. The Rust probe uses an
independent fixed-capacity sink to observe the production capture hook.

Every command emits complete raw input/output storage, channel counts, source
counters, latched connection hints, actual lamp bits and queue occupancy. Drain
commands emit every captured frame's exact float32 bits. Constructor and
no-sample command observations are retained. All trace lines compare exactly;
there is no tolerance, warmup discard or expected output generated from Rust.
The full traces and first 50 differing lines are retained, including failures.

This is a bounded source-model/host contract check. The real desktop SPSC
transport, asynchronous worker, UI/MCP job lifecycle and audio device remain
separate host checks. Cross-rate state adoption and generation-fenced capture
selection are native lifetime policies covered by the Core integration tests.
