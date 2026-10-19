#!/usr/bin/env python3
"""Load an explicitly selected, existing ARM Bogaudio binary in child processes.

Not an OpenPatch runtime host. No plugin sources are compiled or user Rack
settings loaded. Child process isolation is NOT a filesystem/security sandbox.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent


def write(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, directory, name, env=None, timeout=30):
    write(directory / (name + "-command.json"), [str(s) for s in command])
    with (directory / (name + ".stdout")).open("wb") as out, (directory / (name + ".stderr")).open("wb") as err:
        subprocess.run(command, stdout=out, stderr=err, env=env, check=True, timeout=timeout)


def validate(report):
    if report["protocol"] != 1 or report["rack_runtime"] != "2.6.6":
        raise ValueError("wrong probe protocol/Rack runtime")
    expected = {
        "saved_mono": [4.0625],
        "saved_poly16": [(c - 2.5) * .625 for c in range(16)],
        "saved_unlimited": [64.0625],
        "order_changed": [5.0],
        "limit_enabled": [12.0],
        "gain_changed": [5.0],
    }
    cases = report["cases"]
    if len(cases) != len(expected) or {c["name"] for c in cases} != set(expected):
        raise ValueError("missing/duplicate plugin observation")
    for case in cases:
        values = expected[case["name"]]
        if case["width"] != len(values) or case["voltages"] != values:
            raise ValueError("actual binary output disagrees with pinned source anchor: " + case["name"])
    state = report["state"]
    if state["plugin"] != "Bogaudio" or state["model"] != "Bogaudio-Offset" or state["version"] != "2.6.47":
        raise ValueError("wrong saved model")
    params = {p["id"]: p["value"] for p in state["params"]}
    if params != {0: .25, 1: .25}:
        raise ValueError("parameters not preserved")
    if state["data"]["offset_first"] is not True or state["data"]["disableOutputLimit"] is not True:
        raise ValueError("nondefault plugin custom state not preserved")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--plugin", type=Path, required=True, help="existing Bogaudio 2.6.47 directory")
    parser.add_argument("--output", type=Path, required=True, help="parent for append-only attempt directories")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    evidence = Path(tempfile.mkdtemp(prefix="attempt-", dir=args.output.resolve()))
    report = {"passed": False, "stage": "preflight", "evidence": str(evidence)}
    try:
        if platform.system() != "Darwin" or platform.machine() != "arm64":
            raise ValueError("this feasibility gate is macOS ARM only")
        sdk, plugin = args.sdk.resolve(), args.plugin.resolve()
        manifest = json.loads((plugin / "plugin.json").read_text())
        if manifest["slug"] != "Bogaudio" or manifest["version"] != "2.6.47":
            raise ValueError("requires installed Bogaudio 2.6.47")
        watched = [plugin / "plugin.dylib", plugin / "plugin.json", sdk / "libRack.dylib", HERE / "probe.cpp", HERE / "run.py"]
        watched += sorted(p for p in (sdk / "include").rglob("*") if p.is_file())
        # Include all SDK dependencies actually made available to the compiler.
        watched += sorted(p for p in (sdk / "dep/include").rglob("*") if p.is_file())
        before = {str(p): digest(p) for p in watched}
        write(evidence / "inputs-sha256.json", before)
        write(evidence / "manifest.json", manifest)
        for label, command in {
            "plugin-dependencies": ["otool", "-L", str(plugin / "plugin.dylib")],
            "plugin-signature": ["codesign", "-dv", "--verbose=2", str(plugin / "plugin.dylib")],
            "runtime-dependencies": ["otool", "-L", str(sdk / "libRack.dylib")],
            "architectures": ["file", str(plugin / "plugin.dylib"), str(sdk / "libRack.dylib")],
            "toolchain": ["xcrun", "clang++", "--version"],
        }.items():
            run(command, evidence, label)
        sysroot = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
        binary = evidence / "binary-host-probe"
        report["stage"] = "compile adapter against genuine SDK"
        run(["xcrun", "clang++", "-std=c++17", "-O2", "-ffp-contract=off", "-isysroot", sysroot,
             "-isystem", str(Path(sysroot) / "usr/include/c++/v1"), "-I" + str(sdk / "include"),
             "-I" + str(sdk / "dep/include"), str(HERE / "probe.cpp"), "-L" + str(sdk), "-lRack",
             "-Wl,-rpath," + str(sdk), "-o", str(binary)], evidence, "compile", timeout=60)
        report["probe_sha256"] = digest(binary)
        # The installed plugin requests /tmp/Rack2/libRack.dylib. Child-scoped
        # dyld override resolves genuine libRack without patching the plugin or
        # creating global /tmp symlinks. No inherited DYLD injection settings.
        env = {k: v for k, v in os.environ.items() if not k.startswith("DYLD_")}
        env["DYLD_LIBRARY_PATH"] = str(sdk)
        observations = []
        for mode in ("capture", "restore"):
            report["stage"] = mode + " existing binary in fresh child"
            user = evidence / (mode + "-private-user")
            user.mkdir()
            output = evidence / (mode + ".json")
            command = [str(binary), str(plugin), str(user), str(output)]
            if mode == "restore":
                command.append(str(evidence / "saved-module.json"))
            run(command, evidence, mode, env=env, timeout=15)
            value = json.loads(output.read_text())
            validate(value)
            runtimes = value["loaded_rack_runtimes"]
            if len(runtimes) != 1 or Path(runtimes[0]).resolve() != sdk / "libRack.dylib":
                raise ValueError("probe did not load exactly the selected genuine Rack runtime")
            if Path(value["loaded_plugin"]).resolve() != plugin / "plugin.dylib":
                raise ValueError("init symbol did not come from the selected existing binary")
            observations.append(value)
            if mode == "capture":
                write(evidence / "saved-module.json", value["state"])
        if observations[0] != observations[1]:
            raise ValueError("fresh process restore did not reproduce persisted module state and observations")
        if any(digest(p) != before[str(p)] for p in watched):
            raise ValueError("plugin, manifest, runtime, SDK headers or probe changed during execution")
        report.update(passed=True, stage="complete", fresh_processes=2, observations=12,
                      expected_behavior_source_commit="656eaae458e045602dc974bae82e15a11e104958",
                      identity_scope="actual installed binary hash; official distributed build provenance unverified",
                      host_integration=False, ui_or_audio_device_test=False)
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
    write(evidence / "result.json", report)
    print(json.dumps(report, sort_keys=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
