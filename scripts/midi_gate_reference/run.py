#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare the genuine pinned Rack MIDI_Gate class with production Rust."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import struct
import subprocess
import sys
import tempfile
import time
import zlib

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
PIN = "8ea470f343058ade6f75f16d503a1eb330e1700c"
SOURCE_URL = f"https://raw.githubusercontent.com/VCVRack/Rack/{PIN}/src/core/MIDI_Gate.cpp"
SOURCE_SHA = "c83b822aaacea3902d8f9cc85eeff9346ac8e95767e3f2632b8cb0556947a347"
DEFAULTS = [48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39]
MANIFEST = json.loads((HERE / "fixtures.json").read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def fixture(name, frames):
    for frame in range(frames):
        options = [0, 0, 0]
        maps = DEFAULTS.copy()
        operation = learn = 0
        events = []

        def on(channel, note, velocity=127):
            events.append([9, channel, note, velocity])

        def off(channel, note):
            events.append([8, channel, note, 0])

        if name == "startup_defaults":
            pass
        elif name == "all_cells_all_wire_channels":
            options = [1, 1, 0]
            if frame % 257 == 0:
                for lane in range(16):
                    for cell, note in enumerate(maps):
                        on(lane, note, 1 if (cell + lane) % 2 else 127)
            if frame % 257 == 97:
                for lane in range(16):
                    for cell, note in enumerate(maps):
                        if (cell + lane) % 3:
                            off(lane, note)
        elif name == "all_128_notes_mapping_windows":
            options = [1, 1, 0]
            bank = frame // 512
            maps = list(range(bank * 16, bank * 16 + 16))
            if frame % 512 in (0, 2):
                for lane in range(16):
                    for cell, note in enumerate(maps):
                        if frame % 512 == 0:
                            on(lane, note, 1 + (cell * 7 + lane * 11) % 127)
                        else:
                            off(lane, note)
        elif name == "minimum_pulse_velocity_endpoints":
            options = [1, 1, 0]
            if frame % 131 == 0:
                for lane in range(16):
                    for cell, note in enumerate(maps):
                        on(lane, note, 1 if (cell + lane + frame // 131) % 2 else 127)
                        off(lane, note)
        elif name == "short_and_long_notes":
            options = [1, 1, 0]
            cycle = frame % 1021
            for lane in range(16):
                for cell, note in enumerate(maps):
                    if cycle == 0:
                        on(lane, note, 1 + (lane * 7 + cell * 11) % 127)
                    if cycle == [0, 1, 17, 700][cell % 4]:
                        off(lane, note)
        elif name == "repeated_note_timer_velocity":
            options = [1, 1, 0]
            for lane in range(16):
                if frame % 31 == 0:
                    on(lane, 48, 1 if (frame // 31 + lane) % 2 else 127)
                if frame % 31 == 17:
                    off(lane, 48)
        elif name == "zero_velocity_note_on_release":
            options = [1, 1, 0]
            for lane in range(16):
                if frame % 257 == 0:
                    on(lane, 48, 1)
                    on(lane, 49, 127)
                if frame % 257 == 1:
                    on(lane, 48, 0)
                if frame % 257 == 100:
                    on(lane, 49, 0)
        elif name == "same_sample_event_order":
            options = [1, 1, 0]
            if frame % 149 == 0:
                for lane in range(16):
                    for cell, note in enumerate(maps):
                        if cell % 3 == 0:
                            on(lane, note, 1)
                            off(lane, note)
                            on(lane, note, 127)
                        elif cell % 3 == 1:
                            on(lane, note, 127)
                            on(lane, note, 0)
                            off(lane, note)
                        else:
                            off(lane, note)
                            on(lane, note, 1)
                            off(lane, note)
        elif name == "mono_channel_overlap_release":
            options = [1, 0, 0]
            cycle = frame % 509
            if cycle == 0:
                on(0, 48, 1)
            if cycle == 1:
                on(15, 48, 127)
            if cycle == 100:
                off(7, 48)
            if cycle == 300:
                on(2, 49, 64)
                off(3, 49)
        elif name == "channel_filter_and_unrelated_messages":
            options = [1, 1, 1 + (frame // 257) % 16]
            if frame % 31 == 0:
                for lane in range(16):
                    on(lane, 48, 1 + (lane * 11 + frame // 31) % 127)
                    events.append([11, lane, 49, 127])
                    off(lane, 49)
            if frame % 31 == 17:
                for lane in range(16):
                    off(lane, 48)
        elif name == "mpe_hidden_lane_timer_freeze":
            options = [1, int(frame < 64 or frame >= 2048), 0]
            if frame == 63:
                for note in maps:
                    on(15, note, 1)
                    off(15, note)
        elif name == "velocity_mode_toggle_held_cells":
            options = [(frame // 257) % 2, 1, 0]
            if frame == 0:
                for lane in range(16):
                    for note in maps:
                        on(lane, note, 1 + lane * 8)
            if frame == 3072:
                for lane in range(16):
                    for note in maps:
                        off(lane, note)
        elif name == "unmapped_held_cell_remap":
            options = [1, 1, 0]
            if frame >= 257:
                maps[0] = -1
            if frame >= 1024:
                maps[0] = 90
            if frame == 0:
                for lane in range(16):
                    on(lane, 48, 1 + lane * 8)
            if frame in (300, 1500, 2048):
                for lane in range(16):
                    off(lane, 90 if frame == 2048 else 48)
        elif name == "collision_arrays_keep_cell_signal":
            options = [1, 1, 0]
            if frame == 0:
                for lane in range(16):
                    for note in maps:
                        on(lane, note, 1 + lane * 8)
            if frame >= 257:
                maps = [-1 if cell % 3 == 0 else 60 for cell in range(16)]
            if frame in (300, 700):
                for lane in range(16):
                    on(lane, 60, 127)
                    off(lane, 60)
            if frame == 2000:
                operation = 1
        elif name == "learn_positive_repeated_collision":
            options = [1, 1, 2]
            cycle = frame % 257
            if frame == 0:
                on(1, 49, 64)
            if cycle == 1:
                learn = 1 + (frame // 257) % 16
                on(0, 60, 127)
                off(1, 49)
                on(1, 49, 0)
            if cycle == 2:
                events.append([11, 1, 49, 127])
            if cycle == 3:
                on(1, 49, 127)
        elif name == "learn_same_sample_first_message":
            options = [1, 1, 2]
            if frame % 257 == 0:
                learn = 1 + (frame // 257) % 16
                on(0, 70, 127)
                on(1, 60, 0)
                off(1, 59)
                on(1, 60, 1)
                on(1, 61, 127)
        elif name == "source_panic_preserves_learn":
            options = [1, 1, 0]
            cycle = frame % 257
            if cycle == 0:
                for lane in range(16):
                    on(lane, 48, 1 + lane * 8)
            if cycle == 1:
                learn = 1 + (frame // 257) % 16
            if cycle == 2:
                operation = 1
            if cycle == 3:
                events.append([11, 0, 60, 127])
            if cycle == 4:
                on(15, 60, 127)
        elif name == "json_mapping_options_snapshot_restore":
            options = [1, 1, 0]
            if 1024 <= frame <= 2048:
                options = [0, 0, 0]
                maps = list(range(60, 76))
            if frame == 0:
                for lane in range(16):
                    for note in maps:
                        on(lane, note, 1 + lane * 8)
            if frame == 512:
                operation = 2
            if frame == 2048:
                operation = 3
            if frame == 3072:
                for lane in range(16):
                    for note in DEFAULTS:
                        off(lane, note)
        elif name == "parameter_adoption_preserves_cells":
            options = [(frame // 257) % 2, (frame // 509) % 2, (frame // 1021) % 3]
            maps = [(cell + (frame // 383) * 17) % 128 for cell in range(16)]
            if frame % 43 == 0:
                for lane in range(16):
                    for note in maps:
                        on(lane, note, 1 + (note * 7 + lane * 11 + frame // 43) % 127)
            if frame % 43 == 21:
                for lane in range(16):
                    for note in maps:
                        off(lane, note)
        elif name == "unmapped_notes_ignored":
            options = [1, 1, 0]
            maps = [-1] * 16
            if frame % 17 == 0:
                start = ((frame // 17) % 16) * 8
                for lane in range(16):
                    for note in range(start, start + 8):
                        on(lane, note, 127)
        elif name == "source_panic_with_held_notes":
            options = [1, 1, 0]
            if frame % 1024 == 0:
                for lane in range(16):
                    for note in maps:
                        on(lane, note, 1 + lane * 8)
            if frame % 1024 == 512:
                operation = 1
        else:
            raise ValueError(name)
        row = [*options, operation, learn, len(events), *maps]
        row += [number for event in events for number in event]
        yield " ".join(map(str, row))


def serde_library():
    cargo = str(Path.home() / ".cargo/bin/cargo")
    built = subprocess.run([cargo, "build", "--offline", "--locked", "-p", "openpatch-core", "--lib", "--message-format=json"], cwd=ROOT, check=True, capture_output=True, text=True)
    for line in built.stdout.splitlines():
        if line.startswith("{"):
            item = json.loads(line)
            if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "serde_json":
                return next(Path(path) for path in item["filenames"] if path.endswith(".rlib"))
    raise RuntimeError("serde_json build artifact not reported")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/benchmarks/midi-gate-reference.json")
    arguments = parser.parse_args()
    sdk = arguments.sdk.resolve()
    assert sys.byteorder == "little", "C++ trace ABI requires little-endian host"
    started = time.time()
    source = subprocess.check_output(["curl", "--fail", "--silent", "--show-error", "--location", SOURCE_URL])
    assert hashlib.sha256(source).hexdigest() == SOURCE_SHA, "pinned source hash mismatch"
    upstream = source.decode()
    assert upstream.count("struct MIDI_GateWidget") == 1 and upstream.count('#include "plugin.hpp"') == 1
    prefix = upstream.split("struct MIDI_GateWidget", 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>') + "\n} // namespace core\n} // namespace rack\n"
    routing_source = subprocess.check_output(["curl", "--fail", "--silent", "--show-error", "--location", f"https://raw.githubusercontent.com/VCVRack/Rack/{PIN}/src/midi.cpp"])
    cc_report = json.loads((ROOT / "docs/benchmarks/midi-cc-reference.json").read_text())
    assert digest(ROOT / "crates/openpatch-core/src/midi_cc.rs") == cc_report["production_source_sha256"], "frozen CC production source changed"
    for filename, expected in cc_report["driver_sha256"].items():
        assert digest(ROOT / "scripts/midi_cc_reference" / filename) == expected, "frozen CC driver changed"
    cpp_flags = ["-std=c++17", "-O2", "-ffp-contract=off"]
    rust_flags = ["--edition=2021", "-O", "-C", "target-feature=-fma"]
    headers = ["rack.hpp", "midi.hpp", "engine/Port.hpp", "engine/Module.hpp"]
    report = {
        "reference": {"rack_commit": PIN, "source_url": SOURCE_URL, "source_sha256": SOURCE_SHA,
            "compiled_prefix_sha256": hashlib.sha256(prefix.encode()).hexdigest(),
            "midi_routing_source_sha256": hashlib.sha256(routing_source).hexdigest(),
            "sdk_library_sha256": digest(sdk / ("libRack.dylib" if sys.platform == "darwin" else "libRack.so")),
            "sdk_header_sha256": {header: digest(sdk / "include" / header) for header in headers}},
        "method": "Actual pinned MIDI_Gate class and genuine SDK InputQueue, temporarily compiled with widget/registration omitted, rack.hpp substituted for plugin.hpp and both namespaces closed. Production midi_gate.rs compiled unchanged with minimal host data types. Raw original sample indices, all sixteen output probes and every active lane, beginning with the first process call.",
        "predeclared_manifest": MANIFEST,
        "manifest_sha256": digest(HERE / "fixtures.json"),
        "tolerances": {"absolute_volts": MANIFEST["absolute_volts"], "relative": MANIFEST["relative"],
            "maps_learning_completion_frame_output_width_gate_and_pulse_edges": "exact", "alignment": "same sample index; no warm-up, shifts, exclusions or threshold exemptions"},
        "limitations": [
            "No application, native window, MIDI driver/device/context, hardware endpoint or audio device is opened",
            "Exact pinned InputDevice channel-selection rule is applied in the driver before timestamped messages enter the genuine SDK InputQueue; hardware routing is outside this comparison",
            "All sixteen source outputs are marked connected as probes to observe every lane. SDK disconnected port channel metadata is zero, while the source gate DSP still advances; OpenPatch publishes logical width 1/16 even unconsumed, verified separately by Engine tests",
            "Source panic() is exercised directly and deliberately preserves armed Learn; OpenPatch host Panic cancellation, bypass, initialization/reset, endpoint/transport behavior and emergency fallback are separate host tests",
            "Reference constructor velocity bytes are not initialized by the driver and are never read before the actual DSP's valid positive NoteOn; no hidden velocity initialization or prewarming",
            "JSON snapshots call actual dataToJson/dataFromJson; only hardware MIDI endpoint JSON is removed before restore, while note mappings and velocity/MPE options are restored by the genuine class",
            "Configuration is applied through source public options/JSON maps and Rust build/adopt. Configuration-triggered Learn cancellation is explicit host policy, separate from the source panic primitive",
            "Session capture/save/history, durable mailbox/backpressure/stale kind/instance/epoch/backend handling, UI, native hardware qualification and performance are separate gates",
            "Only this SDK/compiler/platform combination is numerically qualified; another platform must execute its own actual-class comparison"],
        "production_source_sha256": digest(ROOT / "crates/openpatch-core/src/midi_gate.rs"),
        "driver_sha256": {name: digest(HERE / name) for name in ["run.py", "sdk_probe.cpp", "rust_probe.rs", "fixtures.json"]},
        "frozen_cc_unchanged": {"production_source_sha256": cc_report["production_source_sha256"], "drivers_match_previous_report": True},
        "toolchain": {"cpp": subprocess.check_output(["c++", "--version"], text=True).splitlines()[0],
            "rust": subprocess.check_output([str(Path.home() / ".cargo/bin/rustc"), "--version"], text=True).strip(),
            "platform": platform.platform(), "machine": platform.machine(), "cpp_flags": cpp_flags, "rust_flags": rust_flags,
            "floating_point": "C++ contraction disabled; Rust FMA target feature disabled; no fast-math or fused operations"},
        "cases": [],
    }
    serde = serde_library()
    with tempfile.TemporaryDirectory(prefix="openpatch-midi-gate-reference-") as directory:
        temporary = Path(directory)
        (temporary / "reference_module.hpp").write_text(prefix)
        cpp, rust = temporary / "sdk_probe", temporary / "rust_probe"
        extra_flags = []
        if sys.platform == "darwin":
            mac_sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
            extra_flags = ["-isysroot", mac_sdk, "-isystem", str(Path(mac_sdk) / "usr/include/c++/v1")]
        subprocess.run(["c++", *cpp_flags, *extra_flags, "-I" + str(sdk / "include"), "-I" + str(sdk / "dep/include"), "-I" + str(temporary), str(HERE / "sdk_probe.cpp"), "-L" + str(sdk), "-lRack", "-Wl,-rpath," + str(sdk), "-o", str(cpp)], check=True)
        (temporary / "probe.rs").write_text((HERE / "rust_probe.rs").read_text().replace("__MIDI_GATE_SOURCE__", json.dumps(str(ROOT / "crates/openpatch-core/src/midi_gate.rs"))))
        subprocess.run([str(Path.home() / ".cargo/bin/rustc"), *rust_flags, str(temporary / "probe.rs"), "--extern", "serde_json=" + str(serde), "-L", "dependency=" + str(serde.parent), "-o", str(rust)], check=True)
        report["executable_sha256"] = {"reference": digest(cpp), "production": digest(rust)}
        environment = os.environ.copy()
        environment["DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"] = str(sdk)
        for rate in MANIFEST["rates"]:
            for name in MANIFEST["cases"]:
                frames = MANIFEST["frames_per_case"]
                data = ("\n".join(fixture(name, frames)) + "\n").encode()
                run_start = time.monotonic()
                traces = [subprocess.run([str(executable), str(rate)], input=data, capture_output=True, check=True, env=environment).stdout for executable in [cpp, rust]]
                execution_seconds = time.monotonic() - run_start
                row_size = (34 + 256) * 4
                assert all(len(trace) == frames * row_size for trace in traces), (name, [len(trace) for trace in traces])
                failures = []
                worst = {"absolute_error_volts": -1.0}
                maximum_ratio = 0.0
                metadata_mismatches = edge_mismatches = active_lane_samples = 0
                for frame in range(frames):
                    offset = frame * row_size
                    metadata = [struct.unpack_from("<34i", trace, offset) for trace in traces]
                    voltages = [struct.unpack_from("<256f", trace, offset + 136) for trace in traces]
                    if metadata[0] != metadata[1]:
                        metadata_mismatches += 1
                        if len(failures) < 8:
                            failures.append({"frame_zero_based": frame, "metadata_differences": [{"field": index, "reference": reference, "actual": actual} for index, (reference, actual) in enumerate(zip(*metadata)) if reference != actual]})
                    for cell, width in enumerate(metadata[0][18:]):
                        for lane in range(width):
                            index = cell * 16 + lane
                            reference, actual = voltages[0][index], voltages[1][index]
                            assert math.isfinite(reference) and math.isfinite(actual)
                            edge_exact = (reference != 0.0) == (actual != 0.0)
                            edge_mismatches += not edge_exact
                            error = abs(reference - actual)
                            tolerance = MANIFEST["absolute_volts"] + MANIFEST["relative"] * abs(reference)
                            ratio = error / tolerance
                            maximum_ratio = max(maximum_ratio, ratio)
                            active_lane_samples += 1
                            if error > worst["absolute_error_volts"]:
                                worst = {"frame_zero_based": frame, "cell_one_based": cell + 1, "lane_one_based": lane + 1, "reference_volts": reference, "actual_volts": actual, "absolute_error_volts": error, "tolerance_volts": tolerance}
                            if (ratio > 1 or not edge_exact) and len(failures) < 8:
                                failures.append({"frame_zero_based": frame, "cell_one_based": cell + 1, "lane_one_based": lane + 1, "reference": reference, "actual": actual, "absolute_error_volts": error, "tolerance_volts": tolerance, "gate_or_pulse_edge_exact": edge_exact})
                case = {"name": name, "sample_rate": rate, "frames": frames, "active_lane_samples": active_lane_samples,
                    "input_sha256": hashlib.sha256(data).hexdigest(), "input_crc32": f"{zlib.crc32(data):08x}",
                    "trace_sha256": {"reference": hashlib.sha256(traces[0]).hexdigest(), "production": hashlib.sha256(traces[1]).hexdigest()},
                    "trace_crc32": {"reference": f"{zlib.crc32(traces[0]):08x}", "production": f"{zlib.crc32(traces[1]):08x}"},
                    "worst_sample": worst, "maximum_tolerance_ratio": maximum_ratio, "metadata_mismatches": metadata_mismatches,
                    "gate_or_pulse_edge_mismatches": edge_mismatches, "driver_execution_seconds": execution_seconds,
                    "passed": not failures, "first_failures": failures}
                report["cases"].append(case)
                print(json.dumps({"case": name, "rate": rate, "passed": case["passed"], "maximum_tolerance_ratio": maximum_ratio}), flush=True)
    report["passed"] = all(case["passed"] for case in report["cases"])
    report["total_frames"] = sum(case["frames"] for case in report["cases"])
    report["total_active_lane_samples"] = sum(case["active_lane_samples"] for case in report["cases"])
    report["elapsed_seconds"] = time.time() - started
    report["run_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "frames": report["total_frames"], "report": str(arguments.output)}))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
