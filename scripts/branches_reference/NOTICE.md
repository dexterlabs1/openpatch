# Branches source and Rust adaptation

Original `branches/branches.cc`: Copyright 2012 Emilie Gillet.
Author: Emilie Gillet (emilie.o.gillet@gmail.com).
Source: pichenettes/eurorack, commit
`f4cfa7697095a28d292bff758beb51deb49b5a86`.

The production `event_fork/kernel.rs` input/decision/output/LED/PRNG scan and ADC
table are adaptations of that source, distributed under GPL-3.0-or-later.
The fetched original retains its complete copyright and license notice.
The harness wrapper/stubs are OpenPatch test infrastructure, not AVR libraries
or analog hardware emulation. Other original OpenPatch code retains its existing
license; this notice does not relicense it.

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later version.
This program is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for more details.

The complete license is [LICENSE-GPL-3.0.txt](../../crates/openpatch-core/LICENSE-GPL-3.0.txt).
Source retrieval with hash verification is provided by `fetch.py` and the pinned
manifest. The Rust adaptation and probe source ship in this repository.
