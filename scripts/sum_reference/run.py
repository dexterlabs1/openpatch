#!/usr/bin/env python3
"""Compare production Sum DSP with genuine Rack SDK primitives, headlessly.

Usage: python3 scripts/sum_reference/run.py --sdk /path/to/Rack-SDK
No application, audio device, external Rust dependency, or GUI is used.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import subprocess
import struct
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
PIN = "10dd0160c664770910e5584b7b00498cc48d9ddd"
SOURCE_URL = f"https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/Sum.cpp"

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def float32(text):
    return struct.unpack("f", struct.pack("f", float(text)))[0]

def vectors(name, frames=8192):
    rng = random.Random(80316)
    for i in range(frames):
        level, count, values = 1.0, 1, [0.0] * 16
        if name == "count_and_polarity":
            count = [0, 1, 2, 8, 16, 3, 0, 16][i // 1024]
            values = [(-1 if j % 3 == 0 else 1) * (j + 1) / 8 for j in range(16)]
        elif name == "post_level_release":
            values[0] = -10.0
            level = [1, 0.5, 0.25, 0, 0, 0, 0, 0][i // 1024]
        elif name == "sampled_impulses":
            # Pulses just before/on/after sampling and publication boundaries.
            values[0] = 12.0 if i % 1024 in [14, 16, 510, 512, 1023] else 0.0
        elif name == "meter_thresholds":
            values[0] = [0, 0.158489319, 0.630957344, 2.51188643, 5.01187234,
                         7.07945784, 10, 10.00001][i // 1024]
        elif name == "random_polyphony":
            count = rng.randrange(17)
            level = [0, 0.125, 0.5, 1][i // 256 % 4]
            values = [rng.uniform(-20, 20) for _ in range(16)]
        elif name == "cancellation_headroom":
            count = 16
            values = [10000, 0.003, -10000, 0.001] * 4
            level = [1, 0.5, 0.125, 0][i // 2048]
        yield " ".join(format(x, ".9g") for x in [level, count, *values])

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/benchmarks/sum-reference.json")
    args = parser.parse_args()
    source = subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", SOURCE_URL], check=True, capture_output=True).stdout
    # Fetch for provenance only. Upstream copyrighted source is not vendored.
    sdk_headers = ["dsp/vumeter.hpp", "dsp/digital.hpp", "dsp/common.hpp", "engine/Port.hpp", "math.hpp"]
    report = {
        "reference": {"fundamental_commit": PIN, "source_url": SOURCE_URL,
                      "source_sha256": hashlib.sha256(source).hexdigest(),
                      "sdk_path": str(args.sdk),
                      "sdk_header_sha256": {p: digest(args.sdk / "include" / p) for p in sdk_headers}},
        "method": "Production sum.rs compiled directly with minimal host types versus actual Rack SDK Input, VuMeter2, ClockDivider; independent C++ driver follows pinned Sum scheduling. Upstream Sum.cpp is fetched and hashed, not compiled or vendored.",
        "coverage": ["finite signed sums over 0..16 active lanes", "post-level gain and cancellation", "16-frame sampling and 512-frame held indicators", "six VU bands and sampled impulse behavior", "release at 44100/48000/96000 Hz", "level edits retain meter phase and state via production adopt"],
        "limitations": ["Not a full Rack module/plugin or GUI execution", "Minimal Rust host adapters bypass parameter validation, graph/cable delay, telemetry publication, UI and MCP", "Inputs remain below OpenPatch safety bound; unrestricted headroom, nonfinite values and clipping differences are not parity claims", "No bypass or sample-rate-change lifecycle validation"],
        "tolerances": {"output_absolute": 0.000001, "output_relative": 0.000001, "brightness_absolute": 0.000003, "channel_count": "exact"},
        "production_source_sha256": digest(ROOT / "crates/openpatch-core/src/sum.rs"),
        "toolchain": {"cpp": subprocess.check_output(["c++", "--version"], text=True).splitlines()[0], "rust": subprocess.check_output(["rustc", "--version"], text=True).strip()},
        "cases": [],
    }
    with tempfile.TemporaryDirectory(prefix="openpatch-sum-reference-") as tmp:
        tmp = Path(tmp)
        cpp = tmp / "sdk_probe"
        rust = tmp / "rust_probe"
        platform_flags = []
        if sys.platform == "darwin":
            mac_sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
            platform_flags = ["-isysroot", mac_sdk, "-isystem", str(Path(mac_sdk) / "usr/include/c++/v1")]
        subprocess.run(["c++", "-std=c++17", "-O2", "-ffp-contract=off", *platform_flags, "-I" + str(args.sdk / "include"), "-I" + str(args.sdk / "dep/include"), str(HERE / "sdk_probe.cpp"), "-o", str(cpp)], check=True)
        rust_src = (HERE / "rust_probe.rs").read_text().replace("__SUM_SOURCE__", json.dumps(str(ROOT / "crates/openpatch-core/src/sum.rs")))
        (tmp / "probe.rs").write_text(rust_src)
        subprocess.run(["rustc", "--edition=2021", "-O", str(tmp / "probe.rs"), "-o", str(rust)], check=True)
        for rate in [44100, 48000, 96000]:
            for name in ["count_and_polarity", "post_level_release", "sampled_impulses", "meter_thresholds", "random_polyphony", "cancellation_headroom"]:
                data = "\n".join(vectors(name)) + "\n"
                outputs = [subprocess.run([str(exe), str(rate)], input=data, text=True, capture_output=True, check=True).stdout for exe in [cpp, rust]]
                reference, actual = [[list(map(float32, row.split())) for row in text.splitlines()] for text in outputs]
                assert len(reference) == len(actual) == 8192
                maximum_output_error = maximum_light_error = 0.0
                failures = []
                for frame, (ref, got) in enumerate(zip(reference, actual), 1):
                    assert len(ref) == len(got) == 8, (name, rate, frame)
                    assert all(math.isfinite(v) for v in ref + got), (name, rate, frame)
                    error = abs(ref[0] - got[0])
                    light_error = max(abs(a - b) for a, b in zip(ref[2:], got[2:]))
                    maximum_output_error = max(maximum_output_error, error)
                    maximum_light_error = max(maximum_light_error, light_error)
                    if error > 1e-6 + 1e-6 * abs(ref[0]) or ref[1] != got[1] or light_error > 3e-6:
                        if len(failures) < 5: failures.append({"frame": frame, "reference": ref, "actual": got})
                report["cases"].append({"name": name, "sample_rate": rate, "frames": len(actual), "input_sha256": hashlib.sha256(data.encode()).hexdigest(), "max_output_error": maximum_output_error, "max_brightness_error": maximum_light_error, "passed": not failures, "first_failures": failures})
    report["passed"] = all(case["passed"] for case in report["cases"])
    report["total_frames"] = sum(case["frames"] for case in report["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "frames": report["total_frames"], "report": str(args.output)}))
    raise SystemExit(0 if report["passed"] else 1)

if __name__ == "__main__":
    main()
