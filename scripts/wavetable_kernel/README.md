# Wavetable hard-sync step profile

OpenPatch's Rust hard-sync implementation uses a fixed 513-point numerical step
profile. The first 512 values were **computed offline** by Rack SDK 2.6.6
`rack::dsp::minBlepImpulse(16,16)` on macOS ARM; OpenPatch appends the unit endpoint.
The SDK is not linked into OpenPatch, invoked by Cargo, or shipped with the app.
No Rack/Fundamental source, artwork, or embedded wavetable is included in this
array. The profile is a compatibility choice, not an independently generated
kernel. Its exact provenance is in
[`docs/dsp/wavetable-kernel-provenance.json`](../../docs/dsp/wavetable-kernel-provenance.json).

Rack's SDK is GPLv3 with additional permissions: see its
[license](https://github.com/VCVRack/Rack/blob/v2/LICENSE.md).
The separately installed numerical design tool and any linked binaries retain
their own licensing requirements; we do not redistribute them here. GPL tools do
not generally impose the GPL on their output merely by processing it; the FSF
[distinguishes output containing portions of the program](https://www.gnu.org/licenses/gpl-faq.en.html#WhatCaseIsOutputGPL).
The distributed data here is computed numerical output, not program text or an
extracted embedded table. This does not label Rack as permissively licensed.

Reproduce with a locally supplied SDK (the recipe builds and runs only an offline
numerical adapter, never a GUI, audio device, plugin host, or installed app):

```sh
python3 scripts/wavetable_kernel/generate_sdk.py \
  --sdk /absolute/path/to/Rack-SDK \
  --metadata /tmp/openpatch-kernel-provenance.json
```

Expected SHA256 of the first 512 little-endian f32 values:
`63feb405c75d062feb5759f5a4f27e1ab9e5da5ecfb62d00c6370fc0b6db1d32`.
Numerical design can vary with SDK build/compiler/architecture; compare the
recorded output hash before changing the production array.

`generate.py` and `generate_pffft.cpp` are independent experiments implementing
Brandt's [Hard Sync Without Aliasing](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf),
sections 6.1–6.3. The minimum-phase reconstruction is sensitive to floating-point
stopband noise; those valid alternative designs differ from the SDK profile and
are not the production compatibility data. Do not replace the production table
by running them without a deliberate profile change and renewed DSP acceptance.
The Python experiment defaults to readout into a separately supplied `--output`
when comparing profiles; use `--json` for a read-only diagnostic.

The standalone PFFFT experiment requires the separately licensed PFFFT source
(BSD-style license in its header), revision
`74d7261be17cf659d5930d4830609406bd7553e3` from
https://bitbucket.org/jpommier/pffft. No PFFFT source or binary is bundled here.
