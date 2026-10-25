#!/usr/bin/env python3
"""Extract pinned Clouds/stmlib initializers; do not synthesize or resample data.

Usage: python3 generate_tables.py PATH/TO/upstream OUTPUT/resources.rs
PATH/TO/upstream is the verified cache from scripts/clouds_reference/fetch.py.
C++ unsuffixed floating constants are parsed as binary64, then narrowed to
binary32, matching the source initialization. Store the resulting exact bits.
"""
import hashlib
import json
import pathlib
import re
import struct
import sys

SOURCES = ("clouds/resources.cc", "clouds/dsp/mu_law.cc", "stmlib/dsp/units.cc", "stmlib/dsp/atan.cc")
upstream = pathlib.Path(sys.argv[1])
manifest = json.loads(
    (pathlib.Path(__file__).resolve().parents[5] / "scripts/clouds_reference/source_manifest.json").read_text())
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


def emit_f32(out, name, values, per_line=4):
    out.append(f"pub(super) static {name}: [f32; {len(values)}] = [\n")
    rendered = [f"f32::from_bits(0x{bits(v):08x})" for v in values]
    for i in range(0, len(rendered), per_line):
        out.append("    " + ", ".join(rendered[i:i + per_line]) + ",\n")
    out.append("];\n\n")


def emit_int(out, name, kind, values, per_line=12):
    out.append(f"pub(super) static {name}: [{kind}; {len(values)}] = [\n")
    for i in range(0, len(values), per_line):
        out.append("    " + ", ".join(str(int(v)) for v in values[i:i + per_line]) + ",\n")
    out.append("];\n\n")


def table(text, declaration):
    return numbers(re.search(re.escape(declaration) + r"\s*=\s*\{(.*?)\};", text, re.S).group(1))


resources, resources_hash = texts["clouds/resources.cc"]
license_text = resources[:resources.index("// -----------------------------------------------------------------------------")]
license_text = "\n".join(line.rstrip() for line in license_text.splitlines()) + "\n"
out = [license_text,
       "// Generated from pinned clouds/resources.cc (SHA-256 " + resources_hash + "),\n",
       "// clouds/dsp/mu_law.cc (SHA-256 " + texts["clouds/dsp/mu_law.cc"][1] + "),\n",
       "// stmlib/dsp/units.cc (SHA-256 " + texts["stmlib/dsp/units.cc"][1] + ")\n",
       "// and stmlib/dsp/atan.cc (SHA-256 " + texts["stmlib/dsp/atan.cc"][1] + "),\n",
       "// all Copyright Emilie Gillet under the MIT notice above.\n",
       "// Reproduce with kernel/generate_tables.py UPSTREAM resources.rs.\n",
       "#![allow(dead_code)]\n\n"]

expected = {"src_filter_1x_2_45": 45, "lut_sin": 1281, "lut_window": 4097, "lut_xfade_in": 17,
            "lut_xfade_out": 17, "lut_sine_window_4096": 4096, "lut_grain_size": 257}
for name, length in expected.items():
    values = table(resources, f"const float {name}[]")
    assert len(values) == length, (name, len(values))
    emit_f32(out, name.upper(), values)

units = texts["stmlib/dsp/units.cc"][0]
for name in ["lut_pitch_ratio_high", "lut_pitch_ratio_low"]:
    values = table(units, f"const float {name}[]")
    assert len(values) == 256
    emit_f32(out, name.upper(), values)

ulaw = table(texts["clouds/dsp/mu_law.cc"][0], "int16_t lut_ulaw[256]")
assert len(ulaw) == 256
emit_int(out, "LUT_ULAW", "i16", ulaw)
atan = table(texts["stmlib/dsp/atan.cc"][0], "const uint16_t atan_lut[513]")
assert len(atan) == 513
emit_int(out, "ATAN_LUT", "u16", atan)

pathlib.Path(sys.argv[2]).write_text("".join(out))
