# Reference source notices

Upstream Clouds and stmlib files are by Émilie Gillet, with copyright years (2012–2015) preserved in each original file. All forty-two files listed under `files` in `source_manifest.json`, and the one variant file, include the MIT permission/warranty notice. Fetching retains their complete notices and exact bytes, including the generated resource tables; do not strip notices when moving code into a production dependency.

Primary repositories: https://github.com/pichenettes/eurorack and https://github.com/pichenettes/stmlib. The stmlib commit `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec` is the gitlink recorded by the pinned eurorack tree `f4cfa7697095a28d292bff758beb51deb49b5a86`, not the current stmlib branch. The variant file `clouds/dsp/window.h` comes unmodified from eurorack `2a468734bdd53bea0f25165bf34d92a4c571db13`.

`reference_probe.cpp`, `fetch.py`, `run.py`, `compare.py`, `make_fixture.py`, `rust_probe.rs` and `stubs/clouds/drivers/debug_pin.h` are original OpenPatch files under the project's MIT license. The stub replaces an STM32 profiling header that `clouds/dsp/granular_processor.cc` includes without using; it declares nothing the DSP depends on. No upstream file is modified.

Not used: the Clouds firmware entry point (`clouds.cc`), CV scaling (`cv_scaler.cc`), UI, settings, meter, drivers, persistent sample storage and the desktop test harness are neither fetched nor compiled. The OpenPatch module documents its own host CV mapping, informed by `cv_scaler.cc`, in `docs/GRAIN_WEATHER.md`. Parasites and other alternate firmware are not used anywhere.
