# Producer output lifecycle reference

This declaration qualifies the bounded effect of cable and bypass events on
producer output storage. It does **not** compare Mid/Side DSP. The actual pinned
MidSide class supplies four outputs and no bypass routes or custom lifecycle
callbacks. The oracle uses genuine Rack Engine 2.6.6 `addCable`, `removeCable`,
`bypassModule` and `hasCable`; no engine step or module DSP call is permitted.
The configured 48 kHz rate is metadata, not a numerical test axis.

The primary sources are Rack
[`Engine.cpp`](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Engine.cpp),
[`Module.cpp`](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Module.cpp),
genuine SDK `include/engine/Port.hpp`, and Fundamental
[`MidSide.cpp`](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/MidSide.cpp).
Their full hashes and line references are in `acceptance.json`. The independent
fixture author did not inspect the new Core implementation.

## Contract

A first physical consumer sets output count to one; a last removal sets it to
zero. Neither clears producer raw voltages. Additional consumers and nonlast
removals preserve the prior count. Both bypass and unbypass edges call
`setChannels(0)`: disconnected outputs retain all storage, while connected
outputs clear only the previously active prefix to **positive zero**, retain
higher raw lanes and become mono. Reassigning the existing bypass state returns
without changing storage.

These operations do not commute. Disconnect then bypass retains every raw
lane; bypass then disconnect clears the former active prefix. Disconnect then
reconnect changes the count to one, so a later bypass clears only lane zero.
Moving consumers while one remains connected preserves the old count and hence
the larger clear extent. Connecting an output while already bypassed preserves
raw storage until a real subsequent bypass-state edge occurs.

All initial connections are actual cables. Connected counts are set using the
SDK API before seeding raw values. The explicit finite raw seeds include
inactive values and signed zero. Public/unstable SDK storage initialization is
intentional; it does not claim every seeded tuple is reachable by running
MidSide's signal equations. Input storage, cable propagation and public host
zero padding are different contracts.

## Wire and coverage

Each input line has `70 + N` fields: initial eight-bit cable mask, four counts,
64 output-major raw voltages, operation count N and N opcodes. Each output line
has exactly 70 fields: four actual counts, all 64 raw voltages, actual bypass
state and actual eight-bit cable mask. Bit `port*2+slot` identifies one of two
consumers per port. Opcodes 0–7 ensure that cable present; 8–15 ensure cable
`opcode−8` absent; 16 toggles bypass; 17/18 ensure bypass false/true. Initial
bypass is always false. Repeated cable requests use `hasCable` guards.

There are **224,988 independent cases**, all observed: 223,040 exhaustive
single-target words (four targets × seventeen counts × all words of lengths
zero through seven over C/D/B), 1,920 fixed multi-consumer/cross-port cases
(120 scripts × all sixteen initial presence masks), and 28 hand anchors
(fourteen histories × two raw seed profiles). Heterogeneous connected counts
are 1, 3, 7 and 16. Exact dyadic seeds alternate with a signed-zero profile,
avoiding a redundant Cartesian expansion. Every exhaustive word prefix already
appears as a separate case; arbitrary cross-port histories are not exhausted.

All **15,749,160 final fields** must match exactly, including inactive and
disconnected raw slots and signed-zero bits. There are no discarded rows,
tolerances or excluded producer lanes. Independent invariants check counts,
consumer/bypass assignments, finite storage, possible clearing and untouched
upper lanes. Literal hand anchors prescribe specific clear prefixes. The
validator does not implement a generic lifecycle transform or inspect Core.

## Reproduction interface

The parent runner owns compilation and actual Engine execution. This directory
contains only the independent declaration, generator and validator:

```sh
python3 scripts/output_lifecycle_reference/verify.py --self-test
python3 scripts/output_lifecycle_reference/fixtures.py --freeze
python3 scripts/output_lifecycle_reference/fixtures.py --check-fixtures
```

`--freeze` exclusively creates `fixtures.json`; it cannot overwrite an existing
manifest. The manifest records all input hashes and counts, compact group row
offsets, the complete supplemental script inventory, hand anchors, and hashes
of the other four declaration files. The runner records the manifest's own hash
externally. All five files become immutable before first reference execution.
Any precomparison correction must preserve its unused predecessor; observations
must never be used to fit a gate or remove a case.

`fixtures.cases(a)` streams dictionaries with `name`, `group`, `profile`,
`fields` and optional `anchor`. `rows(a)` yields `(name, 0, fields)`;
`fixture_text(a)` returns one complete input string; `write_fixture(path,a)`
streams directly to disk and returns the case count/input hash.
`check_declaration()` recomputes and validates the complete declaration.

`verify.validate(a,input_path,sdk_trace_path,rust_trace_path)` streams uncompressed
text and returns a JSON-serializable report with `passed`, `cases`,
`failed_cases`, protocol failures, the first sixteen detailed failures,
compared/mismatched field counts, independent check totals and trace hashes.
There is no frame or safety-phase axis. The runner must separately retain the
adapter's assertion that the actual Engine frame remained zero, full source and
build closure, SDK library/header hashes and both actual executables/traces.
Failed acceptance must retain its report and exit nonzero.

A passing report would establish this declared producer-lifecycle law only.
It would not establish MidSide signal equivalence, realtime or performance
claims, arbitrary module callback behavior, native UI, physical devices,
concurrent host ordering, public tap behavior or complete Rack compatibility.
