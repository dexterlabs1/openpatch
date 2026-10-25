# Reference source notices

Upstream Rings and stmlib files are by Émilie Gillet, with copyright years (2012–2015) preserved in each original file. All thirty-one files listed under `files` in `source_manifest.json`, and the four listed under `reference_only`, include the MIT permission/warranty notice. Fetching retains their complete notices and exact bytes, including the generated resource tables; do not strip notices when moving code into a production dependency.

Primary repositories: https://github.com/pichenettes/eurorack and https://github.com/pichenettes/stmlib. The stmlib commit `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec` is the gitlink recorded by the pinned eurorack tree `f4cfa7697095a28d292bff758beb51deb49b5a86`, not the current stmlib branch.

`reference_probe.cpp`, `fetch.py`, `run.py`, `compare.py`, `make_fixture.py` and `rust_probe.rs` are original OpenPatch files under the project's MIT license. `reference_probe.cpp` reproduces, with attribution, the few DSP lines of `FillBuffer` in `rings/rings.cc` (input clearing while the exciter input is normalized, the input noise gate, the Strummer and Part calls and the `Clip16` output conversion), because `rings.cc` itself depends on the STM32 drivers. No upstream file is modified.

Reference only (fetched and hash-checked, never compiled): `rings/rings.cc`, `rings/cv_scaler.cc`, `rings/cv_scaler.h` and `rings/ui.cc`. They document the firmware call order and inform the OpenPatch host mapping in `docs/STRING_FIELD.md`. Not used: the easter-egg string synthesizer (`string_synth_*`, chorus and ensemble effects), the firmware entry point, settings, calibration, drivers, LEDs and the desktop test harness.
