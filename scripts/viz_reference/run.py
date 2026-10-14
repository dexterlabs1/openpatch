#!/usr/bin/env python3
"""Compare production Viz DSP with genuine Rack SDK primitives, headlessly.

Usage: python3 scripts/viz_reference/run.py --sdk /path/to/Rack-SDK
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
SOURCE_URL = f"https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/Viz.cpp"

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def float32(text):
    return struct.unpack("f", struct.pack("f", float(text)))[0]

def vectors(name, frames=8192):
    rng = random.Random(16032)
    for i in range(frames):
        adopt, count, values = 0, 1, [0.0] * 16
        if name == "count_and_polarity":
            count = [0, 1, 2, 8, 16, 3, 0, 16][i // 1024]
            values = [(-1 if j % 3 == 0 else 1) * (j + 1) for j in range(16)]
        elif name == "signed_release":
            count = 16
            values = [([10, -10, 5, 0, -5, 0, 0, 0][i // 1024]) * (-1 if j % 2 else 1) for j in range(16)]
        elif name == "sampled_impulses":
            count = 16 if i % 32 in [14, 15, 16] else 1
            values = [12.0 if i % 1024 in [14, 15, 16, 30, 31, 32] else 0.0] * 16
        elif name == "retirement_and_reactivation":
            count = [16, 1, 0, 0, 16, 8, 1, 0][i // 1024]
            values = [(-10 if j % 2 else 10) if i < 1024 or i >= 4096 else 0 for j in range(16)]
        elif name == "random_polyphony":
            count = rng.randrange(17)
            values = [rng.uniform(-20, 20) for _ in range(16)]
        elif name == "high_voltage_adoption":
            count = 16
            values = [([10000, -10000, 0, 250, -250, 0, 10, 0][i // 1024]) * (-1 if j % 2 else 1) for j in range(16)]
            adopt = int(i % 37 == 0 or i % 16 in [0, 15])
        yield " ".join(format(x, ".9g") for x in [adopt, count, *values])

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/benchmarks/viz-reference.json")
    args = parser.parse_args()
    source = subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", SOURCE_URL], check=True, capture_output=True).stdout
    # Fetch for provenance only. Upstream copyrighted source is not vendored.
    sdk_headers = ["engine/Light.hpp", "dsp/digital.hpp", "dsp/common.hpp", "engine/Port.hpp", "math.hpp"]
    report = {
        "reference": {"fundamental_commit": PIN, "source_url": SOURCE_URL,
                      "source_sha256": hashlib.sha256(source).hexdigest(),
                      "sdk_path": str(args.sdk),
                      "sdk_header_sha256": {p: digest(args.sdk / "include" / p) for p in sdk_headers}},
        "method": "Production viz.rs compiled directly with minimal host types versus actual Rack SDK Input, Light, ClockDivider; independent C++ driver follows pinned Viz scheduling. Upstream Viz.cpp is fetched and hashed, not compiled or vendored.",
        "coverage": ["0..16 active lanes with independent signed polarity", "16-frame sampling and held count/lights", "raw signed SDK Light state (including negative values and values above one)", "retired/disconnected lanes retain positive release tails", "release at 44100/48000/96000 Hz", "rebuild adoption retains light state and divider phase", "input-only production process returns unit"],
        "limitations": ["Not full upstream module/plugin or GUI execution", "Minimal Rust host adapters bypass graph/cable timing, safety bounding, telemetry publication, UI and MCP", "Inactive SDK voltages are zeroed by host to model engine bookkeeping; Rust host channel accessor likewise returns zero", "Finite test voltages remain within OpenPatch safety bound; no nonfinite or unrestricted voltage parity claim", "No bypass or sample-rate-change lifecycle validation; graph zero-output metadata requires separate integration tests"],
        "tolerances": {"brightness_absolute": 0.000003, "brightness_relative": 0.000001, "channel_count": "exact"},
        "production_source_sha256": digest(ROOT / "crates/openpatch-core/src/viz.rs"),
        "toolchain": {"cpp": subprocess.check_output(["c++", "--version"], text=True).splitlines()[0], "rust": subprocess.check_output(["rustc", "--version"], text=True).strip()},
        "cases": [],
    }
    with tempfile.TemporaryDirectory(prefix="openpatch-viz-reference-") as tmp:
        tmp = Path(tmp)
        cpp = tmp / "sdk_probe"
        rust = tmp / "rust_probe"
        platform_flags = []
        if sys.platform == "darwin":
            mac_sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
            platform_flags = ["-isysroot", mac_sdk, "-isystem", str(Path(mac_sdk) / "usr/include/c++/v1")]
        subprocess.run(["c++", "-std=c++17", "-O2", "-ffp-contract=off", *platform_flags, "-I" + str(args.sdk / "include"), "-I" + str(args.sdk / "dep/include"), str(HERE / "sdk_probe.cpp"), "-o", str(cpp)], check=True)
        rust_src = (HERE / "rust_probe.rs").read_text().replace("__VIZ_SOURCE__", json.dumps(str(ROOT / "crates/openpatch-core/src/viz.rs")))
        (tmp / "probe.rs").write_text(rust_src)
        subprocess.run(["rustc", "--edition=2021", "-O", str(tmp / "probe.rs"), "-o", str(rust)], check=True)
        for rate in [44100, 48000, 96000]:
            for name in ["count_and_polarity", "signed_release", "sampled_impulses", "retirement_and_reactivation", "random_polyphony", "high_voltage_adoption"]:
                data = "\n".join(vectors(name)) + "\n"
                outputs = [subprocess.run([str(exe), str(rate)], input=data, text=True, capture_output=True, check=True).stdout for exe in [cpp, rust]]
                reference, actual = [[list(map(float32, row.split())) for row in text.splitlines()] for text in outputs]
                assert len(reference) == len(actual) == 8192
                maximum_light_error = 0.0
                failures = []
                for frame, (ref, got) in enumerate(zip(reference, actual), 1):
                    assert len(ref) == len(got) == 33, (name, rate, frame)
                    assert all(math.isfinite(v) for v in ref + got), (name, rate, frame)
                    light_error = max(abs(a - b) for a, b in zip(ref[1:], got[1:]))
                    maximum_light_error = max(maximum_light_error, light_error)
                    if ref[0] != got[0] or any(abs(a - b) > 3e-6 + 1e-6 * abs(a) for a, b in zip(ref[1:], got[1:])):
                        if len(failures) < 5: failures.append({"frame": frame, "reference": ref, "actual": got})
                report["cases"].append({"name": name, "sample_rate": rate, "frames": len(actual), "input_sha256": hashlib.sha256(data.encode()).hexdigest(), "max_brightness_error": maximum_light_error, "passed": not failures, "first_failures": failures})
    report["passed"] = all(case["passed"] for case in report["cases"])
    report["total_frames"] = sum(case["frames"] for case in report["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "frames": report["total_frames"], "report": str(args.output)}))
    raise SystemExit(0 if report["passed"] else 1)

if __name__ == "__main__":
    main()
