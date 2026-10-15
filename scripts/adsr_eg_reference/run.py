#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare the production ADSR EG against the actual pinned Rack DSP class."""
import argparse
from array import array
import hashlib
import json
import math
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
PIN = "10dd0160c664770910e5584b7b00498cc48d9ddd"
SOURCE_URL = f"https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/ADSR.cpp"
CASES = {
    "default_voice": 65536,
    "minimum_times": 16384,
    "maximum_times": 131072,
    "sustain_zero": 32768,
    "sustain_one": 32768,
    "coefficient_and_light_phases": 32768,
    "gate_thresholds": 16384,
    "retrigger_thresholds": 16384,
    "retrigger_stages": 32768,
    "signed_cv_extrema": 32768,
    "poly_cv_broadcast": 32768,
    "raw_gate_no_broadcast": 16384,
    "group_shrink_regrow": 65536,
    "held_push": 32768,
    "held_push_with_poly_gate": 32768,
    "parameter_adoption": 32768,
    "deterministic_mixed": 32768,
}
ABS_TOLERANCE = 0.002
REL_TOLERANCE = 1e-5


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def float32(value):
    return struct.unpack("f", struct.pack("f", float(value)))[0]


def vectors(name, frames):
    # Stable vectors; all six ports carry poisoned inactive storage to detect
    # accidental host-padding reads. Oracle adapter clears SDK inactive lanes.
    rng = random.Random(831072)
    for frame in range(frames):
        params = [0.5] * 4 + [0.0] * 4
        ports = [[0, *[997.0 - lane * 3 for lane in range(16)]] for _ in range(6)]
        push = 0

        def port(index, value, width=1):
            values = value if isinstance(value, list) else [value] * width
            ports[index] = [width, *values[:width], *[997.0 - lane * 3 for lane in range(width, 16)]]

        port(4, 10.0 if frame < frames * 2 // 3 else 0.0)
        if name == "default_voice":
            pass
        elif name in ("minimum_times", "maximum_times"):
            t = 0.0 if name == "minimum_times" else 1.0
            params[:4] = [t, t, 0.3, t]
            if name == "minimum_times":
                port(4, 1.0 if frame % 1024 < 700 else 0.0)
        elif name.startswith("sustain_"):
            params[:4] = [0.1, 0.1, 0.0 if name == "sustain_zero" else 1.0, 0.1]
        elif name == "coefficient_and_light_phases":
            # 257 walks every 16/128 phase; each parameter/gate boundary moves.
            cycle = frame // 257
            params = [0.05 + (cycle % 7) * 0.04, 0.1, (cycle % 5) / 4, 0.05, 0.25, -0.5, 0.5, 1.0]
            port(4, 1.0 if frame % 257 < 137 else 0.0)
            for index in range(4):
                port(index, ((frame // 17 + index) % 5 - 2) * 5.0)
        elif name == "gate_thresholds":
            params[:4] = [0.0, 0.0, 0.3, 0.0]
            levels = [1.0, 0.99999994, 1.00000012, 0.5, 0.0, -0.1, 10.0]
            port(4, levels[(frame // 17) % len(levels)])
        elif name == "retrigger_thresholds":
            params[:4] = [0.01, 0.0, 0.25, 0.1]
            levels = [1.0, 0.5, 1.0, 0.1, 1.0, 0.000001, 1.0, 0.0, 1.0, -0.000001, 1.0]
            port(5, levels[(frame // 73) % len(levels)])
        elif name == "retrigger_stages":
            params[:4] = [0.2, 0.25, 0.25, 0.2]
            port(4, 10.0 if frame % 8192 < 6000 else 0.0)
            port(5, 10.0 if frame % 211 in [0, 1] else 0.0)
        elif name == "signed_cv_extrema":
            params = [0.3, 0.5, 0.7, 0.2, 1.0, -1.0, -0.5, 0.5]
            port(4, [10.0 if (frame + lane * 31) % 2048 < 1700 else 0.0 for lane in range(16)], 16)
            for index in range(4):
                port(index, [[-1000000.0, -20.0, -10.0, 0.0, 5.0, 10.0, 20.0, 1000000.0][(frame // 1001 + lane + index) % 8] for lane in range(16)], 16)
        elif name == "poly_cv_broadcast":
            params = [0.15, 0.2, 0.4, 0.1, -0.25, 0.5, 0.5, 1.0]
            port(4, [10.0 if (frame + lane * 79) % 4096 < 3000 else 0.0 for lane in range(13)], 13)
            for index, width in enumerate([1, 3, 7, 16]):
                port(index, [(lane - index) * 1.3 for lane in range(width)], width)
            port(5, 1.0 if frame % 919 == 500 else 0.0)
        elif name == "raw_gate_no_broadcast":
            params[:4] = [0.0, 0.1, 0.5, 0.1]
            port(4, 10.0, 1)
            port(5, [1.0 if frame % 500 == 100 else 0.0] * 16, 16)
        elif name == "group_shrink_regrow":
            params[:4] = [0.1, 0.15, 0.4, 0.2]
            width = [16, 1, 3, 4, 5, 8, 9, 0, 16, 4, 0, 16][(frame // 4093) % 12]
            port(4, [10.0 if (frame + lane * 123) % 2053 < 1500 else 0.0 for lane in range(width)], width)
            port(5, 1.0 if frame % 509 == 33 else 0.0)
        elif name in ("held_push", "held_push_with_poly_gate"):
            params[:4] = [0.05, 0.1, 0.4, 0.1]
            push = int(frame % 4096 < 3000)
            width = 9 if name.endswith("poly_gate") else 0
            port(4, [10.0 if (frame + lane * 81) % 4096 < 2500 else 0.0 for lane in range(width)], width)
        elif name == "parameter_adoption":
            step = frame // 263
            params = [(step % 9) / 8, (step % 7) / 6, (step % 5) / 4, (step % 11) / 10,
                      ((step % 5) - 2) / 2, 0.5, -0.5, 0.25]
            port(4, 10.0 if frame % 3079 < 2500 else 0.0, 5)
            port(0, 3.0)
        elif name == "deterministic_mixed":
            # Change controls every 197 samples, while widths and inputs have
            # independent cadences. Seed makes all cases reproducible.
            if frame % 197 == 0:
                mixed_params = [rng.choice([0.0, 0.03, 0.25, 0.5, 0.77, 1.0]) for _ in range(4)] + [rng.choice([-1.0, -0.25, 0.0, 0.5, 1.0]) for _ in range(4)]
            params = mixed_params
            width = [0, 1, 2, 4, 5, 12, 16][(frame // 311) % 7]
            port(4, [rng.choice([0.0, 0.5, 0.99999994, 1.0, 10.0]) for _ in range(width)], width)
            for index in range(4):
                w = [0, 1, 3, 8, 16][(frame // 157 + index) % 5]
                port(index, [rng.randint(-160, 160) / 8 for _ in range(w)], w)
            port(5, [rng.choice([0.0, 0.1, 0.5, 1.0]) for _ in range(3)], 3)
            push = int(frame % 1009 < 31)
        else:
            raise ValueError(name)
        yield " ".join(format(v, ".9g") for v in [push, *params, *[v for p in ports for v in p]])


def serde_library():
    built = subprocess.run(["cargo", "build", "--offline", "--locked", "-p", "openpatch-core", "--lib", "--message-format=json"], cwd=ROOT, check=True, capture_output=True, text=True)
    for line in built.stdout.splitlines():
        if not line.startswith("{"):
            continue
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "serde_json":
            return next(Path(p) for p in item["filenames"] if p.endswith(".rlib"))
    raise RuntimeError("Cargo did not report serde_json rlib")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/benchmarks/adsr-eg-reference.json")
    args = parser.parse_args()
    args.sdk = args.sdk.resolve()
    source = subprocess.check_output(["curl", "--fail", "--silent", "--show-error", "--location", SOURCE_URL])
    upstream = source.decode()
    assert upstream.count('#include "plugin.hpp"') == 1 and upstream.count("struct ADSRDisplay") == 1
    prefix = upstream.split("struct ADSRDisplay", 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    library = args.sdk / ("libRack.dylib" if sys.platform == "darwin" else "libRack.so")
    headers = ["rack.hpp", "dsp/digital.hpp", "simd/functions.hpp", "simd/sse_mathfun.h", "engine/Port.hpp", "engine/Module.hpp", "engine/Param.hpp", "engine/Light.hpp"]
    report = {
        "reference": {"fundamental_commit": PIN, "source_url": SOURCE_URL, "source_sha256": hashlib.sha256(source).hexdigest(), "compiled_prefix_sha256": hashlib.sha256(prefix.encode()).hexdigest(), "sdk_library_sha256": digest(library), "sdk_header_sha256": {h: digest(args.sdk / "include" / h) for h in headers}},
        "method": "Actual pinned ADSR class prefix, temporarily compiled with plugin.hpp replaced by rack.hpp and display/widget/registration omitted. Every sample compared against the actual production Rust module with minimal host types. No prewarming. Rust parameter changes rebuild and adopt the old module, retaining coefficients and divider phase.",
        "tolerances": {"absolute_volts": ABS_TOLERANCE, "relative": REL_TOLERANCE, "channel_count_and_lights": "exact", "alignment": "same sample; no stage-boundary exemption or trace alignment"},
        "limitations": ["No native app, GUI, plugin host, audio device or speakers opened", "Rust adapters bypass graph cable delay, public JSON validation, live queues, MCP and UI; those require separate host tests", "Finite input voltages inside host bounds; nonfinite policy and ±1000000 V host clamp tested separately", "Same-sample multi-action manual ordering, lifecycle/bypass/init/reset and performance are outside the source oracle", "Only this reported SDK/compiler/platform combination is numerically qualified; cross-platform workspace tests do not independently execute this oracle", "SDK inactive input lanes are explicitly zeroed, matching connected-port engine bookkeeping; nonexposed output padding is checked on Rust only"],
        "production_source_sha256": digest(ROOT / "crates/openpatch-core/src/adsr_eg.rs"),
        "driver_sha256": {p: digest(HERE / p) for p in ["sdk_probe.cpp", "rust_probe.rs", "run.py"]},
        "toolchain": {"cpp": subprocess.check_output(["c++", "--version"], text=True).splitlines()[0], "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(), "platform": sys.platform, "machine": os.uname().machine, "cpp_flags": ["-std=c++17", "-O2", "-ffp-contract=off"], "rust_flags": ["--edition=2021", "-O"], "fma": "C++ contraction disabled; Rust uses ordinary separate operations, no fast-math or mul_add"},
        "cases": [],
    }
    serde = serde_library()
    with tempfile.TemporaryDirectory(prefix="openpatch-adsr-reference-") as directory:
        tmp = Path(directory)
        (tmp / "reference_module.hpp").write_text(prefix)
        cpp, rust = tmp / "sdk_probe", tmp / "rust_probe"
        flags = []
        if sys.platform == "darwin":
            mac_sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
            flags = ["-isysroot", mac_sdk, "-isystem", str(Path(mac_sdk) / "usr/include/c++/v1")]
        subprocess.run(["c++", "-std=c++17", "-O2", "-ffp-contract=off", *flags, "-I" + str(args.sdk / "include"), "-I" + str(args.sdk / "dep/include"), "-I" + str(tmp), str(HERE / "sdk_probe.cpp"), "-L" + str(args.sdk), "-lRack", "-Wl,-rpath," + str(args.sdk), "-o", str(cpp)], check=True)
        (tmp / "probe.rs").write_text((HERE / "rust_probe.rs").read_text().replace("__ADSR_EG_SOURCE__", json.dumps(str(ROOT / "crates/openpatch-core/src/adsr_eg.rs")) ))
        subprocess.run(["rustc", "--edition=2021", "-O", str(tmp / "probe.rs"), "--extern", "serde_json=" + str(serde), "-L", "dependency=" + str(serde.parent), "-o", str(rust)], check=True)
        env = os.environ.copy()
        env["DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"] = str(args.sdk)
        for rate in [44100, 48000, 96000]:
            for name, frames in CASES.items():
                data = "\n".join(vectors(name, frames)) + "\n"
                traces = [subprocess.run([str(exe), str(rate)], input=data, text=True, capture_output=True, check=True, env=env).stdout for exe in [cpp, rust]]
                lines = [trace.splitlines() for trace in traces]
                assert len(lines[0]) == len(lines[1]) == frames
                worst = {"absolute_error_volts": -1.0}
                max_ratio = 0.0
                failures = []
                for frame, (ref_line, got_line) in enumerate(zip(*lines), 1):
                    ref, got = [list(array("f", map(float, line.split()))) for line in [ref_line, got_line]]
                    assert len(ref) == len(got) == 22 and all(math.isfinite(v) for v in ref + got)
                    metadata_ok = ref[0] == got[0] and ref[17:] == got[17:]
                    channels = int(ref[0])
                    valid = True
                    for lane in range(channels):
                        error = abs(ref[1+lane] - got[1+lane])
                        tolerance = ABS_TOLERANCE + REL_TOLERANCE * abs(ref[1+lane])
                        max_ratio = max(max_ratio, error / tolerance)
                        if error > worst["absolute_error_volts"]:
                            worst = {"frame_one_based": frame, "channel_one_based": lane+1, "reference_volts": ref[1+lane], "actual_volts": got[1+lane], "absolute_error_volts": error, "tolerance_volts": tolerance}
                        valid &= error <= tolerance
                    if (not valid or not metadata_ok) and len(failures) < 8:
                        failures.append({"frame_one_based": frame, "metadata_exact": metadata_ok, "reference": ref, "actual": got})
                case = {"name": name, "sample_rate": rate, "frames": frames, "input_sha256": hashlib.sha256(data.encode()).hexdigest(), "worst_sample": worst, "maximum_tolerance_ratio": max_ratio, "passed": not failures, "first_failures": failures}
                report["cases"].append(case)
                print(json.dumps({"case": name, "rate": rate, "passed": case["passed"], "maximum_tolerance_ratio": max_ratio}), flush=True)
    report["passed"] = all(case["passed"] for case in report["cases"])
    report["total_frames"] = sum(case["frames"] for case in report["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "frames": report["total_frames"], "report": str(args.output)}))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
