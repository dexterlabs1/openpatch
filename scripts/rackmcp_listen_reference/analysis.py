#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Frozen exact-bit Listen analysis / WAV comparison. No tolerance or oracle rewriting."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PIN = "285e84c5ff2d3ecef25890da07f578af6a0c5ca6"
DEFAULT_REFERENCE = ROOT / "native-test-output/rack-control-import/reference/rackmcp"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def command(argv, log=None, timeout=120):
    result = subprocess.run([str(x) for x in argv], cwd=ROOT, capture_output=True,
                            text=True, timeout=timeout, check=False)
    if log:
        log.write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}); see {log or argv[0]}")
    return result.stdout


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


def fixtures(directory):
    """Materialize samples once: neither language generates its own reference inputs."""
    cases = []

    def add(name, rate, seconds, generator, present=(True, True), equivalent_to=None):
        n = round(rate * seconds)
        path = directory / (name + ".f32le")
        data = bytearray()
        for i in range(n):
            left, right = generator(i, rate)
            data.extend(struct.pack("<ff", left, right))
        path.write_bytes(data)
        case = dict(name=name, rate=rate, frames=n, leftPresent=present[0],
                    rightPresent=present[1], input=path.name, sha256=sha(path))
        if equivalent_to:
            case["equivalentTo"] = equivalent_to
        cases.append(case)

    def tone(i, rate):
        return f32(5 * math.sin(2 * math.pi * 440 * i / rate))

    add("silence", 48000, 1, lambda i, sr: (0, 0))
    add("dc", 48000, 1, lambda i, sr: (2.5, -1.25))
    add("no_inputs", 48000, .1, lambda i, sr: (3, -2), (False, False))
    add("right_only", 48000, 1, lambda i, sr: (0, tone(i, sr)), (False, True))
    add("mono_absent_right", 48000, 1, lambda i, sr: (tone(i, sr), 0), (True, False))
    add("stereo_equal", 48000, 1, lambda i, sr: (tone(i, sr), tone(i, sr)))
    add("stereo_opposed", 48000, 1, lambda i, sr: (tone(i, sr), -tone(i, sr)))
    # Raw poly sums are supplied by the host. Pre-sum four exact quarter values,
    # then compare against the equal-bus fixture as a real sum-equivalence check.
    def poly_sum(i, rate):
        v = tone(i, rate)
        q = f32(v * .25)
        s = 0.0
        for _ in range(4):
            s = f32(s + q)
        return s, s
    add("poly_sum_equivalent", 48000, 1, poly_sum, equivalent_to="stereo_equal")
    add("clipped", 48000, 1, lambda i, sr: ([0, 10, -10, 12, -12, 9.5][i % 6],
                                                            [-10, 10, 0, -20, 20, .125][i % 6]))
    add("pulse_edges", 48000, 1, lambda i, sr: (10 if i % 12000 < 48 else 0,
                                                                     5 if i % 8000 < 24 else 0))
    def rhythm(i, rate):
        # 120 BPM, 50 ms gated bursts with a short exponential decay.
        phase = i % (rate // 2)
        v = f32(8 * math.sin(2 * math.pi * 440 * i / rate) * math.exp(-phase / (rate * .015))) if phase < rate // 20 else 0
        return v, v
    add("rhythm_120", 48000, 4, rhythm)
    for rate in (44100, 48000, 96000):
        for duration in (.1, 1):
            add(f"tone_{rate}_{duration:g}s", rate, duration,
                lambda i, sr: (tone(i, sr), f32(tone(i, sr) * .5)))
    write_json(directory / "fixtures.json", cases)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, default=DEFAULT_REFERENCE)
    parser.add_argument("--output", type=Path, required=True,
                        help="New evidence directory; must not exist")
    parser.add_argument("--cpp", default="clang++")
    args = parser.parse_args()
    out = args.output.resolve()
    if out.exists():
        parser.error("output already exists; preserve frozen evidence and choose a new directory")
    out.mkdir(parents=True)
    reference = args.reference.resolve()
    frozen = out / "frozen"
    frozen.mkdir()
    inputs = out / "fixtures"
    inputs.mkdir()
    cases = fixtures(inputs)
    # Snapshot production sources, dependencies/lockfile, and untouched upstream
    # analysis before any compiler is invoked. Nothing writes to the reference.
    files = list((ROOT / "crates/openpatch-core/src").rglob("*.rs"))
    files += [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "crates/openpatch-core/Cargo.toml",
              ROOT / "crates/openpatch-core/LICENSE-THIRD-PARTY"]
    files += list(HERE.glob("*"))
    source_manifest = {}
    for path in files:
        if not path.is_file():
            continue
        rel = path.relative_to(ROOT)
        dest = frozen / "native" / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest)
        source_manifest[str(rel)] = sha(path)
    upstream_manifest = {}
    for name in ("src/core/listen_analysis.cpp", "src/core/listen_analysis.hpp", "LICENSE", "plugin.json"):
        src = reference / name
        dest = frozen / "reference" / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)
        upstream_manifest[name] = sha(src)
    # Source pin is an asserted provenance identity, independently accompanied
    # by every actual input hash. Existing audited reference must supply it.
    mac_sdk = command(["xcrun", "--show-sdk-path"]).strip()
    cpp_flags = ["-std=c++11", "-O2", "-ffp-contract=off", "-isysroot", mac_sdk, "-isystem", str(Path(mac_sdk) / "usr/include/c++/v1")]
    toolchain = {
        "rustc": command(["rustc", "-Vv"]), "cargo": command(["cargo", "-V"]),
        "cpp": command([args.cpp, "--version"]), "platform": platform.platform(),
        "RUSTFLAGS": os.environ.get("RUSTFLAGS", ""),
        "CARGO_ENCODED_RUSTFLAGS": os.environ.get("CARGO_ENCODED_RUSTFLAGS", ""),
        "cppFlags": cpp_flags,
        "rustProbeFlags": ["--edition=2021", "-O", "-C", "lto=thin"],
    }
    manifest = dict(referencePin=PIN, referenceSource=str(reference), upstream=upstream_manifest,
                    nativeAndDrivers=source_manifest, toolchain=toolchain, fixtures=cases)
    write_json(out / "inputs-before-build.json", manifest)
    cpp = out / "analysis-cpp"
    rust = out / "analysis-rust"
    # Build only the frozen standalone reference inputs, never a translated oracle.
    command([args.cpp, *cpp_flags, "-I", frozen / "reference/src",
             frozen / "native/scripts/rackmcp_listen_reference/analysis.cpp",
             frozen / "reference/src/core/listen_analysis.cpp", "-o", cpp], out / "cpp-build.log")
    cargo_output = command(["cargo", "build", "--release", "-p", "openpatch-core", "--message-format=json"],
                           out / "cargo-build.log", timeout=600)
    artifacts = [json.loads(line) for line in cargo_output.splitlines() if line.startswith("{")]
    library = next(Path(name) for item in artifacts if item.get("reason") == "compiler-artifact"
                   and item.get("target", {}).get("name") == "openpatch_core"
                   for name in item.get("filenames", []) if name.endswith(".rlib"))
    command(["rustc", "--edition=2021", "-O", "-C", "lto=thin", frozen / "native/scripts/rackmcp_listen_reference/analysis.rs",
             "--extern", f"openpatch_core={library}", "-L", f"dependency={library.parent / 'deps' if (library.parent / 'deps').is_dir() else library.parent}", "-o", rust],
            out / "rust-build.log")
    # Refuse evidence from a mixed source snapshot if concurrent edits happened.
    changed = [name for name, digest in source_manifest.items() if sha(ROOT / name) != digest]
    if changed:
        write_json(out / "source-changed.json", changed)
        raise RuntimeError("production inputs changed during build; no probes executed")
    binary_hashes = {"cpp": sha(cpp), "rust": sha(rust), "coreRlib": sha(library)}
    write_json(out / "binaries-before-run.json", dict(binary_hashes, coreRlibPath=str(library)))
    # Complete frozen manifest exists before the first sample is analyzed.
    results = out / "results"
    results.mkdir()
    differences = []
    native_results = {}
    for case in cases:
        name = case["name"]
        base_args = [inputs / case["input"], case["rate"], int(case["leftPresent"]), int(case["rightPresent"])]
        outputs = {}
        for language, executable in (("cpp", cpp), ("rust", rust)):
            wav = results / f"{name}.{language}.wav"
            raw = command([executable, *base_args, wav], timeout=90)
            (results / f"{name}.{language}.json").write_text(raw)
            outputs[language] = json.loads(raw)
        native_results[name] = outputs["rust"]
        for field in sorted(outputs["cpp"].keys() | outputs["rust"].keys()):
            a, b = outputs["cpp"].get(field), outputs["rust"].get(field)
            if a != b:
                differences.append(dict(fixture=name, field=field, cpp=a, rust=b))
        for suffix in ("wav", "wav.mono"):
            a = results / f"{name}.cpp.{suffix}"
            b = results / f"{name}.rust.{suffix}"
            if a.read_bytes() != b.read_bytes():
                differences.append(dict(fixture=name, field=suffix, cppSha256=sha(a), rustSha256=sha(b)))
        if "equivalentTo" in case:
            expected = native_results[case["equivalentTo"]]
            if outputs["rust"] != expected:
                differences.append(dict(fixture=name, field="polySumEquivalence", equivalentTo=case["equivalentTo"]))
    write_json(out / "exact-differences.json", differences)
    summary = dict(fixtures=len(cases), exactDifferences=len(differences),
                   passed=not differences, tolerance=None, binaries=binary_hashes)
    write_json(out / "summary.json", summary)
    print(json.dumps(dict(summary, evidence=str(out))))
    return 1 if differences else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.TimeoutExpired, OSError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(2)
