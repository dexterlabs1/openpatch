#!/usr/bin/env python3
"""Prepare the original, frozen multistage SVF study without running DSP.

Import the parent's pinned scalar 16x preparer, preserve its copied ladder/VCA,
and replace only SVF source plus one private module registration. No production
source is changed. Output must be a new directory. Original source is MIT.
"""
import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent
spec = importlib.util.spec_from_file_location("svf_antialias_parent_prepare", PARENT / "prepare.py")
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)


def source_manifest():
    manifest = json.loads((HERE / "frozen-manifest.json").read_text())
    for name, expected in manifest["frozen_rust_sha256"].items():
        if parent.sha256((HERE / "frozen_src" / name).read_bytes()) != expected:
            raise ValueError(f"Frozen measured source changed: {name}")
    for name, expected in manifest["original_artifacts_sha256"].items():
        if parent.sha256((HERE / "source_artifacts" / name).read_bytes()) != expected:
            raise ValueError(f"Frozen source artifact changed: {name}")
    return manifest


def prepared_files(baseline_files, matrix, screen=False):
    manifest = source_manifest()
    files = parent.prepared_files(baseline_files, 16, matrix, grouped=False, screen=screen)
    if parent.sha256(files["core/src/ladder.rs"]) != manifest["frozen_ladder_sha256"]:
        raise ValueError("Copied scalar-16 ladder differs from the measured candidate")
    voice = parent.replace_exact(files["core/src/voice_tools.rs"].decode(),
        "use crate::ladder::{Oversampler, LATENCY_SAMPLES};",
        "use crate::svf_resampler::{Oversampler, LATENCY_SAMPLES};").encode()
    frozen_voice = (HERE / "frozen_src/voice_tools.rs").read_bytes()
    if voice != frozen_voice:
        raise ValueError("Parent scalar SVF patch no longer matches frozen multistage SVF")
    files["core/src/voice_tools.rs"] = frozen_voice
    for name in ("svf_resampler.rs", "svf_resampler_kernels.rs", "svf_resampler_tests.rs"):
        files["core/src/" + name] = (HERE / "frozen_src" / name).read_bytes()
    files["core/src/lib.rs"] = parent.replace_exact(files["core/src/lib.rs"].decode(),
        "mod voice_tools;", "mod voice_tools;\nmod svf_resampler;").encode()
    extra_module = b"\n#[doc(hidden)] pub mod spectral_probe;\n"
    base_lib = files["core/src/lib.rs"]
    if matrix in ("highband", "dense"):
        if not base_lib.endswith(extra_module):
            raise ValueError("Unexpected direct-component probe registration")
        base_lib = base_lib[:-len(extra_module)]
    if parent.sha256(base_lib) != manifest["frozen_lib_sha256"]:
        raise ValueError("Private module registration differs from measured candidate")
    # Give standalone core tests the pinned probe dependency versions.
    files["core/Cargo.lock"] = parent.replace_exact(
        baseline_files[parent.PROBE_PREFIX + "Cargo.lock"].decode(),
        '\n[[package]]\nname = "openpatch-svf-spectral-probe"\nversion = "0.0.0"\ndependencies = [\n "openpatch-core",\n "serde_json",\n]\n',
        "").encode()
    # Preserve original generator/screen unchanged. At the prepared root their
    # relative paths reproduce core/src/svf_resampler_kernels.rs correctly.
    for name in ("generate_kernels.py", "screen.py", "frequency-screen.json"):
        files[name] = (HERE / "source_artifacts" / name).read_bytes()
    return files


def provenance(baseline_files, files, matrix, screen=False):
    manifest = source_manifest()
    return {
        "baseline_commit": parent.BASELINE,
        "variant": "original-svf-four-stage-16x", "matrix": matrix, "screen": screen,
        "svf_oversampling": 16, "svf_stage_taps": [129, 33, 17, 33],
        "svf_stage_cutoff_host_rate": [.45, 1., 2., 4.],
        "candidate_delay_host_samples": 76,
        "source_delay_host_samples": 3 if matrix == "performance" else 2,
        "copied_ladder_oversampling": 16, "copied_ladder_fir_taps": 1025,
        "copied_ladder_preserved_from_parent_scalar16": True,
        "baseline_files_sha256": {name: parent.sha256(data) for name,data in sorted(baseline_files.items())},
        "prepared_files_sha256": {name: parent.sha256(data) for name,data in sorted(files.items())},
        "parent_study_files_sha256": {name: parent.sha256((PARENT/name).read_bytes())
            for name in ("prepare.py", "highband_probe.rs", "performance.rs", "reference.py")},
        "multistage_study_files_sha256": {str(p.relative_to(HERE)): parent.sha256(p.read_bytes())
            for p in sorted([HERE/"prepare.py", HERE/"frozen-manifest.json",
                *(HERE/"frozen_src").glob("*.rs"), *(HERE/"source_artifacts").iterdir()])},
        "frozen_candidate_source": manifest,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", choices=("engine", "highband", "dense", "performance"), required=True)
    parser.add_argument("--screen", action="store_true", help="Use the measured eight-case performance screen")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.screen and args.matrix != "performance":
        parser.error("--screen requires --matrix performance")
    if args.output.exists() or args.output.is_symlink():
        parser.error("--output must be a new directory; existing paths are never overwritten")
    baseline = parent.pinned_files(PARENT.parents[2])
    files = prepared_files(baseline, args.matrix, args.screen)
    record = provenance(baseline, files, args.matrix, args.screen)
    args.output.mkdir(parents=True, exist_ok=False)
    for name,data in sorted(files.items()):
        destination = args.output/name
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as handle:
            handle.write(data)
    with (args.output/"provenance.json").open("x") as handle:
        json.dump(record, handle, indent=2)
        handle.write("\n")
    print(f"Prepared original multistage 16x {args.matrix} study at {args.output}")


if __name__ == "__main__":
    main()
