#!/usr/bin/env python3
"""Headless SDK-primitives/production Scope comparison; no application or audio."""
import argparse
import hashlib
import json
from pathlib import Path
import random
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PIN = "10dd0160c664770910e5584b7b00498cc48d9ddd"
URL = f"https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/Scope.cpp"

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def vectors(name, frames=4500):
    rng = random.Random(53016)
    for i in range(frames):
        row = [int(i % 37 == 3)]  # Rebuild with display-only changes mid-bin.
        for axis in range(3):
            count = rng.randrange(17) if name == "random" else [16, 1, 0, 3][(i // 19 + axis) % 4]
            values = [rng.randrange(-300, 301) / 8 for _ in range(16)]
            if name in ("external", "internal", "xy"):
                count = 16
                if axis == 2 or (axis == 0 and name == "internal"):
                    values = [[2.002, 2.0, 2.0005, 2.001][(i // 13 + lane) % 4] for lane in range(16)]
                if axis == 2 and name == "internal": count = 0
            row += [count, *values]
        yield " ".join(format(v, ".9g") for v in row)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--output", type=Path, default=Path("/tmp/openpatch-scope-reference.json"))
    args = parser.parse_args()
    source = subprocess.check_output(["curl", "--fail", "--silent", "--show-error", "--location", URL])
    report = {"reference": {"commit": PIN, "url": URL, "sha256": hashlib.sha256(source).hexdigest(),
        "sdk_path": str(args.sdk), "headers": {name: digest(args.sdk / "include" / name) for name in ["dsp/digital.hpp", "engine/Port.hpp"]}},
        "production_sha256": digest(ROOT / "crates/openpatch-core/src/scope.rs"),
        "method": "Independent C++ bin driver with genuine SDK Input and SchmittTrigger versus production scope.rs with minimal Rust Node/parameter/PortFrame host. Exact bin widths use binary-exact literal seconds at 65536 Hz. Per-frame comparison includes indices, output widths, X/Y/external-trigger input channel metadata and FNV-1a-style u32-word hash of every min/max bin plus all passthrough voltages.",
        "limitations": ["Does not execute upstream Scope module or GUI; no upstream source vendored", "Does not compare exponential TIME mapping/approximation, graph latency, physical SDK output bookkeeping, bypass, route-edit reset or nonfinite samples", "Hash comparison has theoretical collisions; regression tests independently inspect exact bins", "Display-edit adoption compared to uninterrupted SDK driver; reset differences deliberately excluded"],
        "cases": []}
    with tempfile.TemporaryDirectory(prefix="openpatch-scope-reference-") as directory:
        tmp = Path(directory)
        cpp = tmp / "sdk_probe"
        flags = []
        if sys.platform == "darwin":
            sdk = Path(subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip())
            flags = ["-isysroot", str(sdk), "-isystem", str(sdk / "usr/include/c++/v1")]
        subprocess.run(["c++", "-std=c++17", "-O2", "-ffp-contract=off", *flags, "-I" + str(args.sdk / "include"), "-I" + str(args.sdk / "dep/include"), str(HERE / "sdk_probe.cpp"), "-o", str(cpp)], check=True)
        (tmp / "src").mkdir()
        (tmp / "src/main.rs").write_text((HERE / "rust_probe.rs").read_text().replace("__SCOPE_SOURCE__", json.dumps(str(ROOT / "crates/openpatch-core/src/scope.rs"))))
        (tmp / "Cargo.toml").write_text('[package]\nname="scope-reference"\nversion="0.0.0"\nedition="2021"\n[dependencies]\nserde_json="1"\n')
        subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(tmp / "Cargo.toml")], check=True)
        rust = tmp / "target/release/scope-reference"
        for width in [2, 4, 16]:
            for name in ["retirement", "random", "external", "internal", "xy"]:
                data = "\n".join(vectors(name)) + "\n"
                argv = [str(width), str(int(name in ("external", "internal", "xy"))), str(int(name == "xy"))]
                outputs = [subprocess.check_output([str(exe), *argv], input=data, text=True).splitlines() for exe in [cpp, rust]]
                assert len(outputs[0]) == len(outputs[1]) == 4500
                failures = [{"frame": i + 1, "sdk": a, "production": b} for i, (a, b) in enumerate(zip(*outputs)) if a != b]
                report["cases"].append({"name": name, "samples_per_bin": width, "frames": 4500, "input_sha256": hashlib.sha256(data.encode()).hexdigest(), "passed": not failures, "failure_count": len(failures), "first_failures": failures[:3]})
    report["passed"] = all(case["passed"] for case in report["cases"])
    report["total_frames"] = sum(case["frames"] for case in report["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "frames": report["total_frames"], "report": str(args.output)}))
    raise SystemExit(0 if report["passed"] else 1)

if __name__ == "__main__": main()
