#!/usr/bin/env python3
"""Cheap source/syntax checks only; never compile or run DSP."""
import argparse
import ast
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("multistage_prepare", HERE / "prepare.py")
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


def verify():
    baseline = prepare.parent.pinned_files(prepare.PARENT.parents[2])
    manifest = prepare.source_manifest()
    variants = {}
    for matrix, screen in [("engine",False),("highband",False),("dense",False),
                            ("performance",False),("performance",True)]:
        files = prepare.prepared_files(baseline, matrix, screen)
        for name, expected in manifest["frozen_rust_sha256"].items():
            assert prepare.parent.sha256(files["core/src/"+name]) == expected
        assert prepare.parent.sha256(files["core/src/ladder.rs"]) == manifest["frozen_ladder_sha256"]
        for name in ("generate_kernels.py", "screen.py"):
            ast.parse(files[name], filename=name)
        if screen:
            code = files["probe/src/main.rs"]
            for expected in (b"const WARMUP: usize = 256;",b"const MEASURED: usize = 1024;",
                b"for rate in [48000,96000]",b"for modulation in [true]",b"for drive in [0.0,1.0]"):
                assert expected in code
        key = matrix + ("-screen" if screen else "")
        variants[key] = prepare.provenance(baseline, files, matrix, screen)
    for matrix in ("engine", "highband", "dense"):
        try:
            prepare.prepared_files(baseline, matrix, screen=True)
        except ValueError:
            pass
        else:
            raise AssertionError(f"Screen accepted invalid matrix: {matrix}")
    ast.parse((HERE/"prepare.py").read_bytes())
    ast.parse((HERE/"verify_prepare.py").read_bytes())
    # An existing or symlink output must fail before writing any files.
    with tempfile.TemporaryDirectory(prefix="svf-multistage-guards-") as folder:
        occupied=Path(folder)/"existing";occupied.mkdir()
        result=subprocess.run(["python3",str(HERE/"prepare.py"),"--matrix","engine",
            "--output",str(occupied)],capture_output=True,text=True)
        assert result.returncode != 0 and not list(occupied.iterdir())
        linked=Path(folder)/"linked";linked.symlink_to(occupied,target_is_directory=True)
        result=subprocess.run(["python3",str(HERE/"prepare.py"),"--matrix","engine",
            "--output",str(linked)],capture_output=True,text=True)
        assert result.returncode != 0 and not list(occupied.iterdir())
        # The byte-preserved generator must reproduce the frozen Rust exactly.
        candidate=Path(folder)/"generator";(candidate/"core/src").mkdir(parents=True)
        generator=candidate/"generate_kernels.py"
        generator.write_bytes((HERE/"source_artifacts/generate_kernels.py").read_bytes())
        subprocess.run(["python3",str(generator)],check=True,capture_output=True)
        assert (candidate/"core/src/svf_resampler_kernels.rs").read_bytes() == (HERE/"frozen_src/svf_resampler_kernels.rs").read_bytes()
    return {
        "description": "Source-only qualification of all five prepared variants. No DSP or CPU workload executed.",
        "source_and_syntax_checks_passed": True,
        "invalid_screen_and_existing_or_symlink_output_guards_passed": True,
        "original_generator_byte_reproduction_passed": True,
        "variants": variants,
    }


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output",type=Path)
    args=parser.parse_args()
    report=verify()
    if args.output:
        with args.output.open("x") as output:
            json.dump(report,output,indent=2);output.write("\n")
    print("Passed: five byte-preserved variants, syntax, CLI guards, exact original kernel generation.")
