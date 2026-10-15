#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare production sh_asr against actual pinned Fundamental SHASR, headlessly."""
import argparse
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
SOURCE_URL = f"https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/SHASR.cpp"
NAMES = ["shift_old_holds", "patched_input_overrides", "independent_clocks", "split_clock_inheritance", "startup_high_and_intermediate", "disconnect_reconnect_history", "push_clear_collisions", "mono_channel_zero", "deterministic_mixed"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def float32(value):
    return struct.unpack("f", struct.pack("f", float(value)))[0]


def vectors(name, frames=1024):
    rng = random.Random(652991)
    for frame in range(frames):
        push = clear = 0
        ports = [[0, *([0.0] * 16)] for _ in range(16)]
        def port(index, voltage, channels=1):
            ports[index] = [channels, voltage, *[997.0 - lane * 3 for lane in range(1, 16)]]
        value = ((frame * 17) % 257 - 128) / 8
        if name == "shift_old_holds":
            port(0, value)
            push = int(frame % 4 == 1)
        elif name == "patched_input_overrides":
            for row in range(8):
                if (frame // 32 + row) % 3 != 0:
                    port(row, value + row / 4)
            push = int(frame % 4 == 1)
        elif name == "independent_clocks":
            for row in range(8):
                port(row, value + row)
                port(8 + row, 10.0 if frame % (4 + 2 * row) == 1 else 0.0)
        elif name == "split_clock_inheritance":
            port(0, value)
            port(10, 5.0 if frame % 14 == 3 else 0.0)
            port(13, 5.0 if frame % 22 == 7 else 0.0)
            push = int(frame % 6 == 1)
        elif name == "startup_high_and_intermediate":
            levels = [10.0, 0.5, 1.0, 0.5, 1.0, 0.1, 0.5, 1.0, 0.5, 0.1, 1.0, 0.0]
            for row in range(8):
                port(row, value + row / 2)
                # Every detector starts high or intermediate, with no prior low.
                level = levels[(frame + (row % 2)) % len(levels)]
                port(8 + row, level)
        elif name == "disconnect_reconnect_history":
            port(0, value)
            # Connected low, disconnect, reconnect high => edge; disconnect
            # high and reconnect high => no edge. Unpatched interval still normals.
            state = frame % 12
            if state not in [1, 4, 5, 8]:
                port(8, 0.0 if state in [0, 7, 11] else 1.0)
            if frame % 48 < 24:
                port(11, 1.0 if frame % 10 == 2 else 0.0)
        elif name == "push_clear_collisions":
            port(0, value)
            port(8, 10.0 if frame % 6 in [1, 2] else 0.0)
            push = int(frame % 8 in [1, 2, 3])
            clear = int(frame % 8 in [1, 2])
            port(12, 10.0 if frame % 8 == 3 else 0.0)
        elif name == "mono_channel_zero":
            for row in range(8):
                port(row, value + row / 4, [0, 1, 2, 8, 16][(frame // 16 + row) % 5])
                port(8 + row, 1.0 if frame % (4 + row * 2) == 1 else 0.0,
                     [1, 2, 16][row % 3])
            push = int(frame % 10 == 1)
        elif name == "deterministic_mixed":
            for row in range(8):
                if rng.random() < 0.65:
                    port(row, rng.randint(-4096, 4096) / 16, rng.choice([0, 1, 2, 8, 16]))
                if rng.random() < 0.8:
                    port(8 + row, rng.choice([0.0, 0.1, 0.5, 1.0, 10.0]), rng.choice([1, 2, 16]))
            push = int(rng.random() < 0.35)
            clear = int(rng.random() < 0.12)
        else:
            raise ValueError(name)
        yield " ".join(format(value, ".9g") for value in [push, clear, *[item for port_values in ports for item in port_values]])


def serde_library():
    # Reuse the workspace's locked, already-required dependency. No probe deps.
    built = subprocess.run(["cargo", "build", "--offline", "--locked", "-p", "openpatch-core", "--lib", "--message-format=json"], cwd=ROOT, check=True, capture_output=True, text=True)
    artifacts = [json.loads(line) for line in built.stdout.splitlines() if line.startswith("{")]
    for artifact in artifacts:
        if artifact.get("reason") == "compiler-artifact" and artifact.get("target", {}).get("name") == "serde_json":
            for filename in artifact["filenames"]:
                if filename.endswith(".rlib"):
                    return Path(filename)
    raise RuntimeError("Cargo did not report the existing serde_json rlib")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/benchmarks/shift-register-reference.json")
    args = parser.parse_args()
    args.sdk = args.sdk.resolve()
    source = subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", SOURCE_URL], check=True, capture_output=True).stdout
    # GPL upstream source remains temporary and is never committed or installed.
    text = source.decode()
    assert text.count('#include "plugin.hpp"') == 1
    assert text.count("struct SHASRWidget") == 1
    prefix = text.split("struct SHASRWidget", 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    headers = ["rack.hpp", "dsp/digital.hpp", "engine/Port.hpp", "engine/Module.hpp", "engine/Param.hpp", "engine/Light.hpp"]
    library = args.sdk / ("libRack.dylib" if sys.platform == "darwin" else "libRack.so")
    report = {
        "reference": {"fundamental_commit": PIN, "source_url": SOURCE_URL, "source_sha256": hashlib.sha256(source).hexdigest(), "compiled_prefix_sha256": hashlib.sha256(prefix.encode()).hexdigest(), "sdk_path": str(args.sdk), "sdk_library_sha256": digest(library), "sdk_header_sha256": {h: digest(args.sdk / "include" / h) for h in headers}},
        "method": "Actual pinned Fundamental SHASR class DSP/persistence prefix compiled in a temporary directory, changing only plugin.hpp to rack.hpp and omitting SHASRWidget/model registration; linked to separately installed libRack. Compared every output sample against production shift_register.rs compiled directly with minimal host types and the workspace's existing serde_json dependency.",
        "scope": "Deterministic RND-off electrical sample/hold/shift behavior; no random-stream equivalence claim.",
        "coverage": ["old held-value snapshot normaling across eight rows", "patched sample inputs override normals", "independent and split clocks break inherited events", "startup high/intermediate suppression and exact 0.1/1 V thresholds", "disconnect/reconnect detector history", "held PUSH/CLEAR and simultaneous CV/PUSH/CLEAR", "channel zero only with zero, mono and polyphonic connected inputs", "all eight values compared on every frame at 44100/48000/96000 Hz"],
        "limitations": ["No Rack application, plugin host, GUI or audio device is opened; upstream widget/model registration is not compiled", "Minimal Rust adapters bypass public Engine graph/cable delay, host queue publication, saved JSON validation, MCP and UI", "RND off only; OpenPatch's deterministic PRNG/ranges/seed, multi-edge queues within one sample, bypass, reset/init, adoption and persistence are outside this differential check", "Inputs are finite and inside OpenPatch's ±1000000 V safety bound; nonfinite and unrestricted Rack headroom are not parity claims", "Driver assigns SDK input channel bookkeeping and output mono channel count normally owned by Rack Engine; arming frame processes low manual buttons with CV disconnected"],
        "tolerances": {"values_absolute": 0.0, "comparison": "decimal outputs decoded to float32; exact numeric equality for all eight outputs"},
        "production_source_sha256": digest(ROOT / "crates/openpatch-core/src/shift_register.rs"),
        "driver_sha256": {p: digest(HERE / p) for p in ["sdk_probe.cpp", "rust_probe.rs", "run.py"]},
        "toolchain": {"cpp": subprocess.check_output(["c++", "--version"], text=True).splitlines()[0], "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(), "platform": sys.platform},
        "cases": [],
    }
    serde = serde_library()
    with tempfile.TemporaryDirectory(prefix="openpatch-shasr-reference-") as directory:
        tmp = Path(directory)
        (tmp / "reference_module.hpp").write_text(prefix)
        cpp, rust = tmp / "sdk_probe", tmp / "rust_probe"
        platform_flags = []
        if sys.platform == "darwin":
            mac_sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
            platform_flags = ["-isysroot", mac_sdk, "-isystem", str(Path(mac_sdk) / "usr/include/c++/v1")]
        subprocess.run(["c++", "-std=c++17", "-O2", "-ffp-contract=off", *platform_flags, "-I" + str(args.sdk / "include"), "-I" + str(args.sdk / "dep/include"), "-I" + str(tmp), str(HERE / "sdk_probe.cpp"), "-L" + str(args.sdk), "-lRack", "-Wl,-rpath," + str(args.sdk), "-o", str(cpp)], check=True)
        rust_source = (HERE / "rust_probe.rs").read_text().replace("__SHIFT_REGISTER_SOURCE__", json.dumps(str(ROOT / "crates/openpatch-core/src/shift_register.rs")))
        (tmp / "probe.rs").write_text(rust_source)
        subprocess.run(["rustc", "--edition=2021", "-O", str(tmp / "probe.rs"), "--extern", "serde_json=" + str(serde), "-L", "dependency=" + str(serde.parent), "-o", str(rust)], check=True)
        env = os.environ.copy()
        env["DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"] = str(args.sdk)
        for rate in [44100, 48000, 96000]:
            for name in NAMES:
                data = "\n".join(vectors(name)) + "\n"
                traces = [subprocess.run([str(exe), str(rate)], input=data, text=True, capture_output=True, check=True, env=env).stdout for exe in [cpp, rust]]
                reference, actual = [[list(map(float32, line.split())) for line in trace.splitlines()] for trace in traces]
                assert len(reference) == len(actual) == 1024
                maximum_error = 0.0
                failures = []
                for frame, (ref, got) in enumerate(zip(reference, actual), 1):
                    assert len(ref) == len(got) == 8
                    assert all(math.isfinite(v) for v in ref + got)
                    error = max(abs(a - b) for a, b in zip(ref, got))
                    maximum_error = max(maximum_error, error)
                    if ref != got and len(failures) < 5:
                        failures.append({"frame": frame, "reference": ref, "actual": got})
                report["cases"].append({"name": name, "sample_rate": rate, "frames": len(actual), "input_sha256": hashlib.sha256(data.encode()).hexdigest(), "max_absolute_error": maximum_error, "passed": not failures, "first_failures": failures})
    report["passed"] = all(case["passed"] for case in report["cases"])
    report["total_frames"] = sum(case["frames"] for case in report["cases"])
    report["total_values"] = report["total_frames"] * 8
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "frames": report["total_frames"], "values": report["total_values"], "report": str(args.output)}))
    raise SystemExit(0 if report["passed"] else 1)

if __name__ == "__main__":
    main()
