#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded source-version qualification against actual Core and genuine Rack."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DECLARATION = HERE / "acceptance.json"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def execute(command, **kwargs):
    result = subprocess.run(command, text=True, capture_output=True, **kwargs)
    if result.returncode:
        raise RuntimeError(f"{command[0]} failed ({result.returncode}):\n{result.stderr[-10000:]}")
    return result


def f32(value):
    return struct.unpack("f", struct.pack("f", value))[0]


def packets(case, rate, declaration):
    rows = []
    frames = math.ceil(rate / (261.6256 * 2 ** (-54 / 12))) + 65 if case == "low_cycle" else declaration["frames"]
    for frame in range(frames):
        controls = [0., .5, 0., 0., 0., 0.]
        ports = [[0] + [0.] * 16 for _ in range(4)]

        def port(index, values):
            ports[index] = [len(values)] + values + [0.] * (16 - len(values))

        mask, action = 15, 0
        if case != "unpatched":
            port(0, [0.])
        if case in ("poly16", "short_cv", "shrink_regrow"):
            port(0, [(lane - 8) / 12 for lane in range(16)])
        if case in ("low", "low_cycle"):
            controls[0] = -54.
        elif case == "high":
            controls[0] = 54.
            port(0, [4.])
        elif case in ("exp_fm", "linear_fm"):
            controls[2], controls[4] = -.7, float(case == "linear_fm")
            port(1, [3 * math.sin(frame * 2 * math.pi * 1073 / rate)])
        elif case == "pwm":
            controls[1], controls[3] = .2, 1.
            port(3, [8 * math.sin(frame * 2 * math.pi * 617 / rate)])
        elif case in ("hard_sync", "soft_sync"):
            controls[5] = float(case == "soft_sync")
            port(2, [5 * math.sin((frame + .37) * 2 * math.pi * 173 / rate)])
        elif case == "short_cv":
            controls[2:4] = [.4, -.5]
            for index, width in [(1, 3), (2, 5), (3, 7)]:
                port(index, [2 * math.sin(frame / 17 + lane) for lane in range(width)])
        elif case == "shrink_regrow":
            width = [16, 1, 5, 0, 4, 16][frame // 211 % 6]
            port(0, [(lane - 8) / 12 for lane in range(width)])
        elif case == "connectivity":
            mask = [15, 1, 0, 8, 3, 4, 15][frame // 199 % 7]
        elif case == "lifecycle":
            controls = [-12. + 12 * (frame // 251 % 4), .3, .2, -.3, float(frame // 313 % 2), float(frame // 419 % 2)]
            port(1, [math.sin(frame / 13)])
            port(2, [math.sin(frame / 23)])
            port(3, [math.sin(frame / 11)])
            action = 1 if frame in (255, 1023) else 2 if frame in (511, 1535) else 0
        row = [action, int(frame in declaration["observe_frames"]), mask] + controls
        for item in ports:
            row.extend(item)
        rows.append(" ".join(format(f32(value), ".9g") for value in row))
    return "\n".join(rows) + "\n"


def vcf_packets(case, rate, declaration):
    rows = []
    frames = declaration["self_oscillation"]["frames"] if case == "self_oscillation" else declaration["frames"]
    for frame in range(frames):
        controls = [.5, .2, 0., 0., 0., 0.]
        ports = [[0] + [0.] * 16 for _ in range(4)]

        def port(index, values):
            ports[index] = [len(values)] + values + [0.] * (16 - len(values))

        def audio(width=1):
            port(3, [3 * math.sin(2 * math.pi * (103 + lane * 31) * frame / rate) for lane in range(width)])

        mask, action = 3, 0
        audio()
        if case == "impulse":
            port(3, [5. if frame == 0 else 0.])
        elif case == "dc":
            port(3, [[-5., 0., 5., 20., -20.][frame // 311 % 5]])
        elif case == "poly16":
            audio(16)
        elif case == "drive":
            controls[4] = [-1., 0., 1.][frame // 199 % 3]
            port(3, [20. if frame % 37 < 19 else -20.] * 5)
        elif case in ("audio_rate_cv", "mono_cv", "short_cv"):
            controls = [.5, .2, .5, -.4, .1, .7]
            audio(16)
            for index, width in enumerate([3, 5, 7] if case == "short_cv" else [1, 1, 1]):
                port(index, [3 * math.sin(2 * math.pi * (1093 + index * 733) * frame / rate + lane) for lane in range(width)])
        elif case == "cutoff_limits":
            controls[2] = 1.
            port(0, [[-20., -10., 0., 10., 20.][frame // 137 % 5]])
            audio(5)
        elif case == "resonance_limits":
            controls[1], controls[3] = .3, [-1., 0., 1.][frame // 239 % 3]
            port(1, [[-20., -10., 0., 10., 20.][frame // 137 % 5]])
            audio(5)
        elif case == "shrink_regrow":
            audio([16, 1, 5, 0, 4, 16][frame // 211 % 6])
            controls[2] = .2
            port(0, [2.])
        elif case == "connectivity":
            mask = [3, 1, 0, 2, 3, 0, 1][frame // 199 % 7]
            audio(5)
        elif case == "lifecycle":
            controls = [.3 + .1 * (frame // 251 % 4), .2, .2, -.3, .1, .3]
            audio(5)
            for index in range(3):
                port(index, [math.sin(frame / (13 + index * 7))])
            action = 1 if frame in (255, 1023, 2047) else 2 if frame in (511, 1535, 3071) else 0
        elif case == "self_oscillation":
            controls = [.55, .95, 0., 0., 0., 0.]
            port(3, [0.])
        row = [action, int(frame in declaration["observe_frames"]), mask] + controls
        for item in ports:
            row.extend(item)
        rows.append(" ".join(format(f32(value), ".9g") for value in row))
    return "\n".join(rows) + "\n"


def oscillation_metrics(values, rate, contract):
    onset = next((frame / rate for frame, value in enumerate(values) if abs(value) >= contract["startup_threshold_volts"]), None)
    start = int(len(values) * contract["late_window_fraction"])
    tail = values[start:]
    crossings = [frame - 1 - values[frame - 1] / (values[frame] - values[frame - 1]) for frame in range(start + 1, len(values)) if values[frame - 1] <= 0 < values[frame]]
    return {"startup_seconds": onset, "rms": math.sqrt(sum(value * value for value in tail) / len(tail)), "peak": max(map(abs, tail)), "frequency": (len(crossings) - 1) * rate / (crossings[-1] - crossings[0]) if len(crossings) >= 2 else None, "crossings": len(crossings)}


def vco_category(index):
    if index == 0:
        return "exact"
    if index < 65:
        return "voltage"
    field = index - 65
    if field < 16:
        return "phase"
    if field < 32:
        return "frequency"
    if 48 <= field < 64 or 80 <= field < 96:
        return "exact"  # sync direction and square latch
    if 2176 <= field < 2180 or field == 2244:
        return "exact"  # ring indices and divider
    return "state"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--module", choices=["vco", "vcf"], default="vco")
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", action="append")
    parser.add_argument("--rate", type=int, action="append")
    args = parser.parse_args()
    module = args.module
    declaration_path = DECLARATION if module == "vco" else HERE / "vcf_acceptance.json"
    declaration = json.loads(declaration_path.read_text())
    available = declaration["vco_cases"] if module == "vco" else declaration["cases"]
    cases = args.case or available
    rates = args.rate or declaration["rates"]
    assert set(cases) <= set(available)
    assert set(rates) <= set(declaration["rates"])
    output, sdk = args.output.resolve(), args.sdk.resolve()
    output.mkdir(parents=True, exist_ok=False)
    # Freeze the contract before compiling or observing either numerical trace.
    (output / "acceptance-frozen.json").write_bytes(declaration_path.read_bytes())
    (output / "run-frozen.py").write_bytes(Path(__file__).read_bytes())
    source_path = ROOT / f"native-test-output/rack-264-voice/reference/{module.upper()}.cpp"
    assert digest(source_path) == declaration["source_sha256"][module]
    source = source_path.read_text()
    widget = f"struct {module.upper()}Widget"
    assert source.count(widget) == 1
    prefix = source.split(widget, 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    (output / "reference_module.hpp").write_text(prefix)
    mac_sdk = Path(execute(["xcrun", "--show-sdk-path"]).stdout.strip())
    cpp_flags = ["-std=c++17", "-O2", "-ffp-contract=off", "-isysroot", str(mac_sdk), "-isystem", str(mac_sdk / "usr/include/c++/v1")]
    cpp = output / f"sdk_{module}"
    execute(["c++", *cpp_flags, "-I" + str(sdk / "include"), "-I" + str(sdk / "dep/include"), "-I" + str(output), str(HERE / f"sdk_{module}.cpp"), "-L" + str(sdk), "-lRack", "-Wl,-rpath," + str(sdk), "-o", str(cpp)])
    load_audit = None
    if module == "vcf":
        loader = output / "sdk_vcf_load"
        compiled = execute(["c++", *cpp_flags, "-I" + str(sdk / "include"), "-I" + str(sdk / "dep/include"), "-I" + str(output), str(HERE / "sdk_vcf_load.cpp"), "-L" + str(sdk), "-lRack", "-Wl,-rpath," + str(sdk), "-o", str(loader)])
        (output / "loader-compile.log").write_text(compiled.stderr)
        loaded = execute([str(loader)], env={**os.environ, "DYLD_LIBRARY_PATH": str(sdk)})
        (output / "loader.json").write_text(loaded.stdout)
        (output / "loader.stderr").write_text(loaded.stderr)
        load_audit = json.loads(loaded.stdout)
        assert load_audit["all_passed"]
    build = execute(["cargo", "build", "--offline", "--locked", "-p", "openpatch-core", "--lib", "--message-format=json"], cwd=ROOT, env={**os.environ, "CARGO_TARGET_DIR": str(args.target.resolve())})
    (output / "cargo.log").write_text(build.stderr)
    libraries = {}
    for line in build.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact":
            for filename in item["filenames"]:
                if filename.endswith(".rlib"):
                    libraries[item["target"]["name"]] = Path(filename)
    rust = output / f"rust_{module}"
    rust_command = ["rustc", "--edition=2021", "-O", str(HERE / f"rust_{module}.rs"), "-L", "dependency=" + str(libraries["serde_json"].parent)]
    for name in ("openpatch_core", "serde_json"):
        rust_command += ["--extern", name + "=" + str(libraries[name])]
    execute(rust_command + ["-o", str(rust)])
    headers = {str(path.relative_to(sdk)): digest(path) for directory in (sdk / "include", sdk / "dep/include") for path in sorted(directory.rglob("*")) if path.is_file()}
    (output / "sdk-header-manifest.json").write_text(json.dumps(headers, indent=2) + "\n")
    production = {str(path.relative_to(ROOT)): digest(path) for path in sorted((ROOT / "crates/openpatch-core").rglob("*.rs"))}
    production["Cargo.lock"] = digest(ROOT / "Cargo.lock")
    for path in (ROOT / "Cargo.toml", ROOT / "crates/openpatch-core/Cargo.toml"):
        production[str(path.relative_to(ROOT))] = digest(path)
    (output / "production-manifest.json").write_text(json.dumps(production, indent=2) + "\n")
    report = {"module": module, "acceptance_sha256": digest(declaration_path), "source_sha256": digest(source_path), "cpp_prefix_sha256": digest(output / "reference_module.hpp"), "sdk_library_sha256": digest(sdk / "libRack.dylib"), "sdk_header_manifest_sha256": digest(output / "sdk-header-manifest.json"), "production_manifest_sha256": digest(output / "production-manifest.json"), "cpp_flags": cpp_flags, "rust_library_sha256": digest(libraries["openpatch_core"]), "rust_profile": "Cargo dev; rustc -O diagnostic driver", "rust_version": execute(["rustc", "--version"]).stdout.strip(), "cpp_version": execute(["c++", "--version"]).stdout.splitlines()[0], "machine": os.uname().machine, "adapter_sha256": {name: digest(HERE / name) for name in (f"sdk_{module}.cpp", f"rust_{module}.rs", "run.py")}, "binary_sha256": {"sdk": digest(cpp), "rust": digest(rust)}, "cases": []}
    if load_audit:
        report["load_audit"] = {"adapter_sha256": digest(HERE / "sdk_vcf_load.cpp"), "report_sha256": digest(output / "loader.json"), "binary_sha256": digest(loader), "cases": len(load_audit["cases"]), "passed": True}
    env = {**os.environ, "DYLD_LIBRARY_PATH": str(sdk)}
    for rate in rates:
        jobs = [(case, None, None) for case in cases] if module == "vco" else [(case, history, seed) for case in cases for history in ([0.] if case == "self_oscillation" else declaration["histories"]) for seed in (declaration["seeds"] if case == "self_oscillation" else declaration["seeds"][:1])]
        for case, history, seed in jobs:
            data = (packets if module == "vco" else vcf_packets)(case, rate, declaration)
            suffix = f"{case}-{rate}" + (f"-h{history}-s{seed[0]}" if module == "vcf" else "")
            (output / f"{suffix}-input.txt").write_text(data)
            traces = []
            trace_hashes = {}
            for label, executable in [("sdk", cpp), ("rust", rust)]:
                arguments = [str(rate)] + ([*seed, str(history)] if module == "vcf" else [])
                result = execute([str(executable), *arguments], input=data, env=env)
                (output / f"{suffix}-{label}.txt").write_text(result.stdout)
                (output / f"{suffix}-{label}.stderr").write_text(result.stderr)
                trace_hashes[label] = digest(output / f"{suffix}-{label}.txt")
                traces.append(result.stdout.splitlines())
            failures, maximum = [], {}
            frames = math.ceil(rate / (261.6256 * 2 ** (-54 / 12))) + 65 if module == "vco" and case == "low_cycle" else declaration["self_oscillation"]["frames"] if module == "vcf" and case == "self_oscillation" else declaration["frames"]
            assert all(len(trace) == frames for trace in traces)
            oscillation = [[], []]
            for frame, pair in enumerate(zip(*traces)):
                observed = frame in declaration["observe_frames"]
                expected_count = (2315 if observed else 65) if module == "vco" else (115 if observed else 35)
                reference, actual = [[int(token) if module == "vcf" and index in (33, 34) else f32(float(token)) for index, token in enumerate(row.split())] for row in pair]
                assert len(reference) == len(actual) == expected_count
                if module == "vcf" and case == "self_oscillation":
                    oscillation[0].append([reference[1], reference[17]])
                    oscillation[1].append([actual[1], actual[17]])
                for index, (left, right) in enumerate(zip(reference, actual)):
                    category = vco_category(index) if module == "vco" else "exact" if index in (0, 33, 34) else "voltage" if index < 33 else "state"
                    limit = declaration["vco_limits"][category] if module == "vco" else {"absolute": 0, "relative": 0} if category == "exact" else declaration["limits"][category]
                    error = abs(left - right)
                    maximum[category] = max(maximum.get(category, 0), error)
                    compared = not (module == "vcf" and case == "self_oscillation" and frame >= declaration["self_oscillation"]["pointwise_frames"] and category != "exact")
                    if not math.isfinite(left) or not math.isfinite(right) or compared and error > limit["absolute"] + limit["relative"] * abs(left):
                        if len(failures) < 8:
                            failures.append({"frame": frame, "index": index, "category": category, "sdk": left, "rust": right, "error": error})
            metrics = []
            if oscillation[0]:
                contract = declaration["self_oscillation"]
                for channel in range(2):
                    left, right = [oscillation_metrics([row[channel] for row in trace], rate, contract) for trace in oscillation]
                    metrics.append({"output": channel, "sdk": left, "rust": right})
                    valid = all(item["startup_seconds"] is not None and item["crossings"] >= contract["minimum_late_crossings"] for item in (left, right))
                    if valid:
                        valid = abs(left["startup_seconds"] - right["startup_seconds"]) <= max(contract["startup_absolute_seconds"], contract["startup_relative"] * left["startup_seconds"])
                        valid &= all(abs(left[key] - right[key]) <= contract[key + "_relative"] * abs(left[key]) for key in ("rms", "peak", "frequency"))
                    if not valid:
                        failures.append({"reason": "self-oscillation metrics outside declared bounds", "output": channel})
            record = {"name": case, "rate": rate, "initial_previous_input": history, "seed": seed, "input_sha256": hashlib.sha256(data.encode()).hexdigest(), "trace_sha256": trace_hashes, "passed": not failures, "maximum_errors": maximum, "first_failures": failures, "oscillation_metrics": metrics}
            report["cases"].append(record)
            (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps({key: record[key] for key in ("name", "rate", "passed", "maximum_errors")}), flush=True)
    report["passed"] = all(case["passed"] for case in report["cases"])
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
