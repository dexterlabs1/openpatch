#!/usr/bin/env python3
"""Extract pinned Rings/stmlib initializers; do not synthesize or resample data.

Usage: python3 generate_tables.py PATH/TO/upstream OUTPUT/resources.rs
PATH/TO/upstream is the verified cache from scripts/rings_reference/fetch.py.
C++ unsuffixed floating constants are parsed as binary64, then narrowed to
binary32, matching the source initialization. Store the resulting exact bits.
"""
import hashlib
import json
import pathlib
import re
import struct
import sys

SOURCES = ("rings/resources.cc", "stmlib/dsp/units.cc")
upstream = pathlib.Path(sys.argv[1])
manifest = json.loads(
    (pathlib.Path(__file__).resolve().parents[5] / "scripts/rings_reference/source_manifest.json").read_text())
texts = {}
for name in SOURCES:
    data = (upstream / name).read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    assert digest == manifest["files"][name]["sha256"], "Unpinned source: " + name
    texts[name] = (data.decode(), digest)


def bits(value):
    return struct.unpack("<I", struct.pack("<f", float(value)))[0]


def numbers(body):
    body = re.sub(r"//[^\n]*", "", body)
    return [s.strip() for s in body.split(",") if s.strip()]


def emit_f32(out, name, values, per_line=1):
    out.append(f"pub(super) static {name}: [f32; {len(values)}] = [\n")
    rendered = [f"f32::from_bits(0x{bits(v):08x})" for v in values]
    for i in range(0, len(rendered), per_line):
        out.append("    " + ", ".join(rendered[i:i + per_line]) + ",\n")
    out.append("];\n\n")


def table(text, declaration):
    return numbers(re.search(re.escape(declaration) + r"\s*=\s*\{(.*?)\};", text, re.S).group(1))


resources, resources_hash = texts["rings/resources.cc"]
license_text = resources[:resources.index("// -----------------------------------------------------------------------------")]
license_text = "\n".join(line.rstrip() for line in license_text.splitlines()) + "\n"
out = [license_text,
       "// Generated from pinned rings/resources.cc (SHA-256 " + resources_hash + ")\n",
       "// and stmlib/dsp/units.cc (SHA-256 " + texts["stmlib/dsp/units.cc"][1] + "),\n",
       "// both Copyright Emilie Gillet under the MIT notice above.\n",
       "// Reproduce with kernel/generate_tables.py UPSTREAM resources.rs.\n\n"]

expected = {"lut_sine": 5121, "lut_4_decades": 257, "lut_svf_shift": 257, "lut_stiffness": 257,
            "lut_fm_frequency_quantizer": 129}
for name, length in expected.items():
    values = table(resources, f"const float {name}[]")
    assert len(values) == length, (name, len(values))
    emit_f32(out, name.upper(), values)

units = texts["stmlib/dsp/units.cc"][0]
for name in ["lut_pitch_ratio_high", "lut_pitch_ratio_low"]:
    values = table(units, f"const float {name}[]")
    assert len(values) == 256
    emit_f32(out, name.upper(), values)

pathlib.Path(sys.argv[2]).write_text("".join(out).rstrip("\n") + "\n")
