#!/usr/bin/env python3
"""Extract pinned Tides initializers; do not synthesize or resample wave data.

Usage: python3 generate_tables.py PATH/TO/upstream OUTPUT/resources.rs
C++ unsuffixed floating constants are parsed as binary64, then narrowed to
binary32, matching resources.cc initialization. Store the resulting exact bits.
"""
import hashlib
import pathlib
import re
import struct
import sys

source = pathlib.Path(sys.argv[1]) / "tides2/resources.cc"
expected = "18ef4483a12b28072700862aa5bece0496573987dc2f21cf69c5f46ecf0a494e"
data = source.read_bytes()
assert hashlib.sha256(data).hexdigest() == expected, "Unpinned resource source"
text = data.decode()
license_text = text[:text.index("// -----------------------------------------------------------------------------")]
result = [license_text, "// Generated from pinned tides2/resources.cc; SHA-256: " + expected + "\n",
          "// Reproduce with kernel/generate_tables.py UPSTREAM resources.rs.\n"]
for name, kind in [("lut_bipolar_fold", "f32"), ("lut_unipolar_fold", "f32"),
                   ("lut_wavetable", "i16")]:
    body = re.search(r"const (?:float|int16_t) " + name + r"\[\] = \{(.*?)\};", text, re.S).group(1)
    values = [s.strip() for s in body.split(",") if s.strip()]
    result.append(f"pub(super) static {name.upper()}: [{kind}; {len(values)}] = [\n")
    if kind == "f32":
        values = [f"f32::from_bits(0x{struct.unpack('<I', struct.pack('<f', float(v)))[0]:08x})" for v in values]
        per_line = 4
    else:
        per_line = 12
    for i in range(0, len(values), per_line):
        result.append("    " + ", ".join(values[i:i + per_line]) + ",\n")
    result.append("];\n\n")
pathlib.Path(sys.argv[2]).write_text("".join(result))
