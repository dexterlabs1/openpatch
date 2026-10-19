#!/usr/bin/env python3
"""Run the actual binary/shared-memory foundation gate, without opening devices.

Serially builds the SDK companion and Rust control driver, then runs bounded
correctness/failure tests. Debug correctness is not realtime qualification.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile

from run import digest, run, write

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gate", choices=("transport", "mixed-graph", "xfade"), default="transport", help="production boundary to exercise; neither mode opens a device")
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--plugin", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True, help="full module snapshot from the feasibility gate")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    sdk, plugin, seed = args.sdk.resolve(), args.plugin.resolve(), args.state.resolve()
    parent_example = {"mixed-graph":"binary_host_graph", "transport":"binary_host_transport", "xfade":"binary_host_xfade"}[args.gate]
    args.output.mkdir(parents=True, exist_ok=True)
    evidence = Path(tempfile.mkdtemp(prefix="transport-", dir=args.output.resolve()))
    os.chdir(ROOT)
    report = {"passed": False, "stage": "preflight", "gate": args.gate, "evidence": str(evidence)}
    try:
        if platform.system() != "Darwin" or platform.machine() != "arm64":
            raise ValueError("macOS ARM64 only")
        watched = [HERE / name for name in ("companion.cpp", "wire.h", "run_transport.py", "run.py")]
        watched += [plugin / "plugin.dylib", plugin / "plugin.json", sdk / "libRack.dylib", seed,
                    ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
        for crate in ("openpatch", "openpatch-core"):
            watched.append(ROOT / "crates" / crate / "Cargo.toml")
            watched += sorted((ROOT / "crates" / crate / "src").rglob("*.rs"))
        watched.append(ROOT / ("crates/openpatch/examples/" + parent_example + ".rs"))
        for directory in (sdk / "include", sdk / "dep/include"):
            watched += sorted(p for p in directory.rglob("*") if p.is_file())
        before = {str(p): digest(p) for p in watched}
        write(evidence / "inputs-sha256.json", before)
        run(["xcrun", "clang++", "--version"], evidence, "cpp-toolchain")
        run(["rustc", "-vV"], evidence, "rust-toolchain")
        run(["otool", "-L", str(plugin / "plugin.dylib")], evidence, "plugin-dependencies")
        run(["otool", "-L", str(sdk / "libRack.dylib")], evidence, "runtime-dependencies")
        run(["codesign", "-dv", "--verbose=2", str(plugin / "plugin.dylib")], evidence, "plugin-signature")
        sysroot = Path(subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip())
        companion = evidence / "companion"
        report["stage"] = "compile companion"
        run(["xcrun", "clang++", "-std=c++17", "-O2", "-ffp-contract=off", "-isysroot", str(sysroot),
             "-isystem", str(sysroot / "usr/include/c++/v1"), "-I" + str(sdk / "include"),
             "-I" + str(sdk / "dep/include"), str(HERE / "companion.cpp"), "-L" + str(sdk), "-lRack",
             "-Wl,-rpath," + str(sdk), "-o", str(companion)], evidence, "compile-companion", timeout=60)
        report["stage"] = "build Rust control driver"
        run(["cargo", "build", "--locked", "-p", "openpatch", "--example", parent_example, "--message-format=json"],
            evidence, "build-parent", timeout=300)
        artifacts = [json.loads(line) for line in (evidence / "build-parent.stdout").read_text().splitlines()]
        parents = [a["executable"] for a in artifacts if a.get("reason") == "compiler-artifact"
                   and a.get("target", {}).get("name") == parent_example and a.get("executable")]
        if len(parents) != 1:
            raise ValueError("Cargo did not identify exactly one built parent example")
        parent = Path(parents[0])
        report["companion_sha256"] = digest(companion)
        report["parent_sha256"] = digest(parent)
        report["stage"] = "actual transport correctness and failure injection"
        run([str(parent), str(companion), str(sdk), str(plugin), str(evidence / "driver"), str(seed)],
            evidence, "driver", timeout=60)
        result = json.loads((evidence / "driver/result.json").read_text())
        if result.get("passed") is not True:
            raise ValueError("parent gate did not pass")
        if any(digest(p) != before[str(p)] for p in watched):
            raise ValueError("source, plugin, SDK or seed changed during execution")
        artifacts = {str(p.relative_to(evidence)): digest(p) for p in sorted(evidence.rglob("*"))
                     if p.is_file() and p.name != "transport"}
        write(evidence / "artifacts-sha256.json", artifacts)
        report.update(passed=True, stage="complete", validation=result,
                      artifact_manifest_sha256=digest(evidence / "artifacts-sha256.json"))
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
    write(evidence / "result.json", report)
    print(json.dumps(report, sort_keys=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
