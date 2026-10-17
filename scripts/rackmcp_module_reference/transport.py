#!/usr/bin/env python3
"""Exact transport/status comparison using unchanged genuine source and public Core.
No target lanes, SDK binding correction, copied status implementation or tolerances.
"""
import argparse
import copy
import itertools
import json
import os
from pathlib import Path
import shutil
import subprocess

import run as common

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CASES = {"internal_selection": 14000, "stopped_seek_reset_coalescing": 1536,
         "external_transport": 2048}
FILES = ("transport.py", "transport.cpp", "transport.rs", "run.py", "README.md")
STATUS_KEYS = {"moduleId", "running", "bpm_bits", "swing_bits", "clockSource",
               "selectedPattern", "activePattern", "pendingPattern", "step", "loop",
               "bar_bits", "beat_bits", "patternCount", "laneCount"}


def data():
    """Original no-target data exercises polyphony and different pattern divisions."""
    tracks = []
    for track in range(4):
        tracks.append(dict(voices=track + 1, glideMs=37.5, steps=[
            dict(step=step, notes=[-12 + track * 12 + step * 3, 12 + track * 12],
                 vel=(track + 1) / 4, gate=.65, accent=step % 2 == 0,
                 slide=step % 3 == 0, ratchet=1 + step % 3)
            for step in range(4)]))
    return dict(version=1, seed=4721, selectedPattern="A", lanes=[], patterns=[
        dict(name="A", length=4, stepsPerBeat=8, tracks=tracks),
        dict(name="B", length=3, stepsPerBeat=4, tracks=[dict(voices=2, steps=[
            dict(step=step, notes=[24 + step * 12], vel=.75, accent=True,
                 gate=.8, ratchet=2) for step in range(3)])])])


def action(name, **fields):
    return dict(action=name, **fields)


def packets(case):
    events = {
        "internal_selection": {
            0: [action("play")],
            65: [action("select", pattern=1, now=True)],
            1700: [action("select", pattern=0, now=False)],
            9500: [action("select", pattern=1, now=True)],
            9800: [action("reset")],
            10000: [action("stop")],
            10010: [action("toggle")],
            11000: [action("stop")],
            11002: [action("play")],
        },
        "stopped_seek_reset_coalescing": {
            0: [action("select", pattern=1, now=False), action("seek", bar=.390625), action("reset")],
            63: [action("seek", bar=99), action("seek", bar=.875)],
            127: [action("select", pattern=0, now=True), action("select", pattern=1, now=False)],
            129: [action("play")],
            200: [action("stop")],
            201: [action("seek", bar=.1)],
            255: [action("reset"), action("reset")],
            300: [action("select", pattern=0, now=False), action("seek", bar=.875), action("reset")],
            301: [action("seek", bar=.375), action("play")],
            700: [action("toggle")],
            701: [action("select", pattern=1, now=True), action("reset"), action("seek", bar=.875)],
            768: [action("play")],
        },
        "external_transport": {
            0: [action("play")],
            65: [action("select", pattern=1, now=True)],
            100: [action("seek", bar=.875)],
            105: [action("stop")],
            110: [action("seek", bar=.390625)],
            111: [action("reset"), action("select", pattern=0, now=False)],
            130: [action("play")],
            333: [action("reset")],
            1200: [action("select", pattern=1, now=False)],
            1297: [action("reset"), action("seek", bar=.875), action("select", pattern=0, now=True)],
            1600: [action("stop")],
            1601: [action("seek", bar=.390625)],
            1650: [action("toggle")],
        },
    }[case]
    for frame in range(CASES[case]):
        packet = dict(bpm=300, swing=63.25, button=0, external=False,
                      clock=0, reset=0, commands=copy.deepcopy(events.get(frame, [])))
        if case == "stopped_seek_reset_coalescing":
            # Parameter changes update status immediately without source data loads.
            if 400 <= frame < 600:
                packet.update(bpm=173.5, swing=71.25)
            packet["button"] = int(500 <= frame < 502)
            packet["reset"] = 10 if 900 <= frame < 903 else 0
        elif case == "external_transport":
            packet["external"] = not 700 <= frame < 740
            # Period changes, stalls and reconnect-high retain real detector history.
            period = 31 if frame < 900 else 47
            packet["clock"] = 10 if frame % period >= 8 else 0
            if 400 <= frame < 520:
                packet["clock"] = 10
            if frame in (601, 602, 603):
                packet["clock"] = [.100001, .1, 1][frame - 601]
            packet["reset"] = 10 if 1000 <= frame < 1003 else 0
            packet["button"] = int(1500 <= frame < 1502)
        yield packet


def differences(left, right, path=""):
    if type(left) is not type(right):
        return [dict(field=path, source=left, native=right)]
    if isinstance(left, dict):
        if left.keys() != right.keys():
            return [dict(field=path, source_keys=sorted(left), native_keys=sorted(right))]
        result = []
        for key in sorted(left):
            result.extend(differences(left[key], right[key], f"{path}.{key}".lstrip(".")))
            if len(result) >= 12:
                break
        return result[:12]
    if isinstance(left, list):
        if len(left) != len(right):
            return [dict(field=path, source_length=len(left), native_length=len(right))]
        result = []
        for index, (one, two) in enumerate(zip(left, right)):
            if one != two:
                result.extend(differences(one, two, f"{path}[{index}]"))
                if len(result) >= 12:
                    break
        return result[:12]
    return [] if left == right else [dict(field=path, source=left, native=right)]


def compare(source, native, expected):
    count, differing = 0, 0
    first = []
    with source.open() as left, native.open() as right:
        for count, (one, two) in enumerate(itertools.zip_longest(left, right), 1):
            if one is None or two is None:
                raise RuntimeError(f"trace length mismatch at frame {count - 1}")
            one, two = json.loads(one), json.loads(two)
            for row in (one, two):
                if set(row) != {"before", "after", "outputs"} or len(row["outputs"]) != 306:
                    raise RuntimeError(f"malformed trace frame {count - 1}")
                if any(set(row[key]) != STATUS_KEYS for key in ("before", "after")):
                    raise RuntimeError(f"malformed status frame {count - 1}")
            if one != two:
                differing += 1
                if len(first) < 8:
                    first.append(dict(frame=count - 1, fields=differences(one, two)))
    if count != expected:
        raise RuntimeError(f"expected {expected} frames, got {count}")
    return dict(frames=count, passed=differing == 0, differing_frames=differing, first_mismatches=first)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "sdk", "output", "target-dir"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--case", choices=list(CASES), action="append")
    args = parser.parse_args()
    source, sdk, output, target = (getattr(args, key).resolve() for key in ("source", "sdk", "output", "target_dir"))
    output.mkdir(parents=True, exist_ok=False)
    frozen = json.loads((source / "manifest.json").read_text())
    assert frozen["repository_commit"] == common.PIN
    for name, expected in frozen["files"].items():
        assert common.digest(source / name) == expected, name
    for name in FILES:
        shutil.copy2(HERE / name, output / name)
    common.save(output / "source-manifest.json", frozen)
    production = {str(path.relative_to(ROOT)): common.digest(path)
                  for path in sorted((ROOT / "crates/openpatch-core/src").rglob("*.rs"))}
    for name in ("Cargo.toml", "Cargo.lock", "crates/openpatch-core/Cargo.toml"):
        production[name] = common.digest(ROOT / name)
    common.save(output / "production-manifest.json", production)
    headers = {str(path.relative_to(sdk)): common.digest(path)
               for folder in (sdk / "include", sdk / "dep/include")
               for path in sorted(folder.rglob("*")) if path.is_file()}
    common.save(output / "sdk-headers.json", headers)
    chosen = args.case or list(CASES)
    common.save(output / "declaration.json", dict(
        source_pin=common.PIN, sdk="2.6.6", rate=48000,
        cases={name: CASES[name] for name in chosen},
        comparison="exact every frame from frame zero; every output/status field; no tolerance or ignored columns",
        outputs="18*(channel count + 16 canonical f32 IEEE words)",
        status="all genuine statusJson fields before and after process; reals encoded as IEEE words; actual dataToJson desired name; published beat derived exactly from source bar",
        target_lanes=0, target_binding_correction="none; no target module or native bindings",
        scheduling="source direct process; producer n delivered at n+1; new clock route begins at +0; real SDK cables and native compiled routes",
        limits="bounded transport/status wrapper proof; no Rack graph-adoption, UI, device, Listen, Bridge or whole-patch equivalence"))
    fixtures = []
    for name in chosen:
        fixture, commands = output / (name + ".json"), output / (name + ".jsonl")
        common.save(fixture, dict(rate=48000, data=data()))
        commands.write_text("".join(json.dumps(packet, separators=(",", ":")) + "\n" for packet in packets(name)))
        fixtures.extend([fixture, commands])
    build = common.execute(["cargo", "build", "--offline", "--locked", "-p", "openpatch-core", "--lib", "--message-format=json"],
                           output, "cargo", cwd=ROOT, env={**os.environ, "CARGO_TARGET_DIR": str(target)})
    libraries = {}
    for line in build.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact":
            for filename in item["filenames"]:
                if filename.endswith(".rlib"):
                    libraries[item["target"]["name"]] = Path(filename)
    native = output / "native"
    common.execute(["rustc", "--edition=2021", "-O", str(output / "transport.rs"),
                    "-L", "dependency=" + str(libraries["serde_json"].parent),
                    "--extern", "openpatch_core=" + str(libraries["openpatch_core"]),
                    "--extern", "serde_json=" + str(libraries["serde_json"]), "-o", str(native)], output, "rustc")
    mac_sdk = common.execute(["xcrun", "--show-sdk-path"], output, "mac-sdk").stdout.strip()
    cpp = output / "source"
    flags = ["-std=c++17", "-O2", "-ffp-contract=off", "-DRACKMCP_HAVE_JANSSON=1",
             "-isysroot", mac_sdk, "-isystem", mac_sdk + "/usr/include/c++/v1"]
    common.execute(["c++", *flags, "-I" + str(source / "src"), "-I" + str(sdk / "include"),
                    "-I" + str(sdk / "dep/include"), str(output / "transport.cpp"),
                    str(source / "src/rackside/SequencerModule.cpp"), str(source / "src/core/sequencer.cpp"),
                    str(source / "src/core/plan.cpp"), "-L" + str(sdk), "-lRack", "-Wl,-rpath," + str(sdk),
                    "-Wl,-dead_strip", "-o", str(cpp)], output, "cpp")
    manifest = dict(
        source_manifest_sha256=common.digest(output / "source-manifest.json"),
        production_manifest_sha256=common.digest(output / "production-manifest.json"),
        sdk_headers_sha256=common.digest(output / "sdk-headers.json"),
        sdk_library_sha256=common.digest(sdk / "libRack.dylib"),
        drivers={name: common.digest(output / name) for name in FILES},
        declaration_sha256=common.digest(output / "declaration.json"),
        fixtures={path.name: common.digest(path) for path in fixtures},
        rust_library_sha256=common.digest(libraries["openpatch_core"]),
        binaries={path.name: common.digest(path) for path in (cpp, native)},
        toolchain={"rust": common.execute(["rustc", "--version"], output, "rust-version").stdout.strip(),
                   "cpp": common.execute(["c++", "--version"], output, "cpp-version").stdout.splitlines()[0]},
        cpp_flags=flags)
    common.save(output / "manifest.json", manifest)  # Freeze before any driver/sample execution.
    report = dict(manifest_sha256=common.digest(output / "manifest.json"), cases=[])
    for name in chosen:
        traces = []
        for binary in (cpp, native):
            trace = output / (name + "." + binary.name + ".trace")
            command = [str(binary), str(output / (name + ".json")), str(output / (name + ".jsonl"))]
            common.save(output / (name + "." + binary.name + "-command.json"), command)
            with trace.open("w") as stream, (output / (name + "." + binary.name + ".stderr.log")).open("w") as errors:
                try:
                    result = subprocess.run(command, stdout=stream, stderr=errors, timeout=60,
                                            env={**os.environ, "DYLD_LIBRARY_PATH": str(sdk)})
                except subprocess.TimeoutExpired as error:
                    report["failure"] = dict(case=name, binary=binary.name, reason="timeout", seconds=60)
                    common.save(output / "report.json", report)
                    raise RuntimeError(f"{binary.name}/{name} timed out; private partial trace retained") from error
            if result.returncode:
                report["failure"] = dict(case=name, binary=binary.name, returncode=result.returncode)
                common.save(output / "report.json", report)
                raise RuntimeError(f"{binary.name}/{name} failed; see private stderr")
            traces.append(trace)
        result = compare(*traces, CASES[name])
        result["case"] = name
        report["cases"].append(result)
        common.save(output / "report.json", report)
    report["passed"] = all(case["passed"] for case in report["cases"])
    common.save(output / "report.json", report)
    print(json.dumps(dict(passed=report["passed"], cases=len(chosen), frames=sum(CASES[name] for name in chosen), report=str(output / "report.json"))))
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
