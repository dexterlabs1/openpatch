#!/usr/bin/env python3
"""Purely parse pinned Emilie Gillet Marbles/stmlib decimal resources to f32 bits."""
import pathlib
import re
import struct

ROOT = pathlib.Path(__file__).resolve().parents[2]
MARBLES = ROOT / 'native-test-output/marbles-reference/upstream/marbles/resources.cc'
UNITS = ROOT / 'native-test-output/marbles-reference/upstream/stmlib/dsp/units.cc'
DEST = ROOT / 'crates/openpatch-core/src/recall_random/kernel/resources.rs'
HEADER = '''// Copyright 2015 Emilie Gillet. MIT license; see crate LICENSE-THIRD-PARTY.
// Source-derived Marbles resources and stmlib pitch tables. Do not regenerate
// numerically: generate_resources.py parses the pinned C++ decimal literals.
// Decimal unsuffixed C++ constants round through double, then binary32.
#![allow(clippy::unreadable_literal)]
'''

def generate():
    output = [HEADER]
    for source in [MARBLES, UNITS]:
        content = source.read_text()
        for name, body in re.findall(r'const float (\w+)\[\]\s*=\s*\{(.*?)\};', content, re.S):
            values = [x.strip() for x in body.split(',') if x.strip()]
            bits = [struct.unpack('<I', struct.pack('<f', float(x.rstrip('f'))))[0] for x in values]
            output.append(f'pub static {name.upper()}: [f32; {len(bits)}] = [\n')
            for i in range(0, len(bits), 6):
                output.append('    ' + ', '.join(f'f32::from_bits(0x{b:08x})' for b in bits[i:i+6]) + ',\n')
            output.append('];\n')
        if source == MARBLES:
            body = re.search(r'const float\* distributions_table\[\]\s*=\s*\{(.*?)\};', content, re.S)[1]
            names = [x.strip() for x in body.split(',') if x.strip()]
            output.append(f'pub static DISTRIBUTIONS_TABLE: [&[f32]; {len(names)}] = [\n')
            output.extend(f'    &{name.upper()},\n' for name in names)
            output.append('];\n')
    DEST.write_text(''.join(output))

if __name__ == '__main__':
    generate()
