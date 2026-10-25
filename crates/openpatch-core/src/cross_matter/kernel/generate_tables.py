#!/usr/bin/env python3
"""Extract pinned Warps/stmlib initializers; do not synthesize or resample data.

Usage: python3 generate_tables.py PATH/TO/upstream OUTPUT/resources.rs
PATH/TO/upstream is the verified cache from scripts/warps_reference/fetch.py.
C++ unsuffixed floating constants are parsed as binary64, then narrowed to
binary32, matching the source initialization. Store the resulting exact bits.
"""
import hashlib
import pathlib
import re
import struct
import sys

SOURCES = ("warps/resources.cc", "warps/dsp/sample_rate_conversion_filters.h", "stmlib/dsp/units.cc")
upstream = pathlib.Path(sys.argv[1])
manifest = __import__("json").loads(
    (pathlib.Path(__file__).resolve().parents[5] / "scripts/warps_reference/source_manifest.json").read_text())
texts = {}
for name in SOURCES:
    data = (upstream / name).read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    assert digest == manifest["files"][name]["sha256"], "Unpinned source: " + name
    texts[name] = (data.decode(), digest)


def bits(value):
    return struct.unpack("<I", struct.pack("<f", float(value)))[0]


def numbers(body):
    return [s.strip() for s in body.split(",") if s.strip()]


def emit(out, name, values, per_line=4):
    out.append(f"pub(super) static {name}: [f32; {len(values)}] = [\n")
    rendered = [f"f32::from_bits(0x{bits(v):08x})" for v in values]
    for i in range(0, len(rendered), per_line):
        out.append("    " + ", ".join(rendered[i:i + per_line]) + ",\n")
    out.append("];\n\n")


resources, resources_hash = texts["warps/resources.cc"]
license_text = resources[:resources.index("// -----------------------------------------------------------------------------")]
out = [license_text,
       "// Generated from pinned warps/resources.cc (SHA-256 " + resources_hash + "),\n",
       "// warps/dsp/sample_rate_conversion_filters.h (SHA-256 " + texts["warps/dsp/sample_rate_conversion_filters.h"][1] + ")\n",
       "// and stmlib/dsp/units.cc (SHA-256 " + texts["stmlib/dsp/units.cc"][1] + "),\n",
       "// all Copyright Emilie Gillet under the MIT notice above.\n",
       "// Reproduce with kernel/generate_tables.py UPSTREAM resources.rs.\n",
       "#![allow(dead_code)]\n\n"]

bank = []
for match in re.finditer(r"const float (fb_\w+)\[\] = \{(.*?)\};", resources, re.S):
    bank.append((match.group(1), numbers(match.group(2))))
assert len(bank) == 20 and all(len(v) == 7 for _, v in bank)
out.append("/// Filter-bank band records in source order: decimation, delay, post gain,\n")
out.append("/// then f/fq for the two cascaded crossover SVF passes.\n")
out.append(f"pub(super) static FILTER_BANK: [[f32; 7]; {len(bank)}] = [\n")
for name, values in bank:
    out.append(f"    // {name}\n    [" + ", ".join(f"f32::from_bits(0x{bits(v):08x})" for v in values) + "],\n")
out.append("];\n\n")

for name in ["lut_sin", "lut_xfade_in", "lut_xfade_out", "lut_bipolar_fold", "lut_midi_to_f_high",
             "lut_midi_to_f_low", "lut_pot_curve", "lut_ap_poles", "wav_sine_i", "wav_sine_q",
             "wav_harmonics_i", "wav_harmonics_q", "wav_buzzy_i", "wav_buzzy_q"]:
    body = re.search(r"const float " + name + r"\[\] = \{(.*?)\};", resources, re.S).group(1)
    emit(out, name.upper(), numbers(body))

units = texts["stmlib/dsp/units.cc"][0]
for name in ["lut_pitch_ratio_high", "lut_pitch_ratio_low"]:
    body = re.search(r"const float " + name + r"\[\] = \{(.*?)\};", units, re.S).group(1)
    emit(out, name.upper(), numbers(body))

filters = texts["warps/dsp/sample_rate_conversion_filters.h"][0]
for match in re.finditer(r"struct SRC_FIR<SRC_(UP|DOWN), (\d+), (\d+)> \{.*?const float h\[\] = \{(.*?)\};", filters, re.S):
    direction, ratio, length, body = match.groups()
    values = numbers(body)
    assert len(values) == int(length) // 2
    emit(out, f"SRC_{direction}_{ratio}_{length}", values)

pathlib.Path(sys.argv[2]).write_text("".join(out))
