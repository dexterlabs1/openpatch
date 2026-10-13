#!/usr/bin/env python3
"""Compare the Rust evaluator with pinned tinyexpr and reconstructed Rack registration.

Requires clang, rustc, curl, and Python 3. Downloaded third-party sources (including their
license notices) remain unmodified in a temporary directory. No Cargo or Rack build.
Use --reference-dir to read already downloaded sources; their hashes are still checked.
"""
import argparse
import hashlib
import itertools
import json
import math
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PROBES = ROOT / "scripts/expression_reference"
TINYEXPR_COMMIT = "4e8cc0067a1e2378faae23eb2dfdd21e9e9907c2"
RACK_COMMIT = "8ea470f343058ade6f75f16d503a1eb330e1700c"
SOURCES = {
    "tinyexpr.c": (f"https://raw.githubusercontent.com/codeplea/tinyexpr/{TINYEXPR_COMMIT}/tinyexpr.c", "b5b8f549f50fe68c19890ce31562543d9ee0dc6566eb3e6b72426e1e71e6d8c9"),
    "tinyexpr.h": (f"https://raw.githubusercontent.com/codeplea/tinyexpr/{TINYEXPR_COMMIT}/tinyexpr.h", "4d7a0c30f5ec653b3b962d9eee24484d3a5f85a27393754feee6e47014b651c0"),
    "rack-src_Quantity.cpp": (f"https://raw.githubusercontent.com/VCVRack/Rack/{RACK_COMMIT}/src/Quantity.cpp", "bdcc0c812f756d07c4ee1a8a0a02926cc3dbc1a3dde5760d9b1b705abbc9da86"),
    "rack-include_dsp_common.hpp": (f"https://raw.githubusercontent.com/VCVRack/Rack/{RACK_COMMIT}/include/dsp/common.hpp", "73cdb6c6689e79f6d3668160e5041b39beeb8e86b55d28a461675c0049e2665e"),
    "rack-Makefile": (f"https://raw.githubusercontent.com/VCVRack/Rack/{RACK_COMMIT}/Makefile", "3ffa32fb258da31691aa6c70357868d67e69bc8a0836e33a28068514c1c79f38"),
    "rack-.gitmodules": (f"https://raw.githubusercontent.com/VCVRack/Rack/{RACK_COMMIT}/.gitmodules", "37c97511f107c5214b86167cabb66f2bf1b1bd34636ae4a20c9d80611f1146ad"),
}


def command(argv):
    return subprocess.run(argv, cwd=ROOT, capture_output=True, text=True,
                          check=True, timeout=60).stdout


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def matrix():
    cases = [a+b+c+d for a,b,c,d in itertools.product('cdefgab',['','s','b','#'],['']+list('0123456789'),['','v'])]
    for function in ['abs','acos','asin','atan','ceil','cos','cosh','exp','fac','floor','ln','log','log10','log2','sin','sinh','sqrt','tan','tanh','gaintodb','dbtogain','vtof','ftov','vtobpm','bpmtov']:
        for value in [-100,-2,-1,-.1,0,.1,1,2,20,21,100]:
            cases.append(f'{function}({value})')
    for function in ['atan2','pow','ncr','npr']:
        for a,b in itertools.product([-2,0,1,2,4.1,5.9,20,30,67],[0,1,2,4.1,20]):
            cases.append(f'{function}({a},{b})')
    cases += ['sin 2^2','sin -2^2','sin(-2)^2','sin cos -2','pow(2,(1,3))','sin(1,2)','pi()','c()','e()','exp(log(0))']
    main_count = len(cases)
    cases += ['(-2)^2','(-1)^0','2^-2^2','0x0p9999','0x0.1p1024','0x10p-1078','0x'+('1'+'0'*280)+'p-1120']
    intentional = {'(-2)^2': 4.0, '(-1)^0': 1.0, '2^-2^2': 0.0625}
    assert main_count == 1081 and len(cases) == 1088
    return cases, main_count, intentional


def probe_results(executable, cases):
    lines = command([str(executable), *cases]).splitlines()
    if len(lines) != len(cases):
        raise RuntimeError(f"Probe returned {len(lines)} results for {len(cases)} cases")
    results = []
    for index, line in enumerate(lines):
        expression, separator, result = line.partition("\t")
        if not separator or expression != cases[index]:
            raise RuntimeError(f"Probe result {index} has incorrect expression or format")
        results.append(result)
    return results


def equivalent(expected, result):
    if expected.startswith("ERROR"):
        return result.startswith("Err")
    reference = float(expected)
    if result.startswith("Err"):
        return math.isnan(reference)
    if not result.startswith("Ok(") or not result.endswith(")"):
        raise RuntimeError(f"Invalid Rust probe result: {result}")
    actual = float(result[3:-1])
    return actual == reference or math.isclose(actual, reference, rel_tol=1e-12, abs_tol=1e-14)


def verify(work, reference_dir):
    sources = {}
    for name, (url, expected) in SOURCES.items():
        if reference_dir:
            data = (reference_dir / name).read_bytes()
        else:
            data = subprocess.run(
                ["curl", "--fail", "--location", "--silent", "--show-error",
                 "--max-time", "30", url],
                capture_output=True, check=True, timeout=45,
            ).stdout
        digest = sha256(data)
        if digest != expected:
            raise RuntimeError(f"SHA256 mismatch for {name}: expected {expected}, got {digest}")
        (work / name).write_bytes(data)
        sources[name] = {"url": url, "sha256": digest}

    c_source = PROBES / "rack_registration_probe.c"
    template = PROBES / "rust_expression_probe.rs"
    module = ROOT / "crates/openpatch-desktop/src/expression.rs"
    rust_source = work / "rust_expression_probe.rs"
    rust_source.write_text(template.read_text().replace("__EXPRESSION_MODULE_PATH__", json.dumps(str(module))))
    commands = [
        ["clang", "-O2", "-DTE_POW_FROM_RIGHT", "-DTE_NAT_LOG", "-I", str(work),
         str(c_source), str(work / "tinyexpr.c"), "-lm", "-o", str(work / "rack_registration_probe")],
        ["rustc", "--edition=2021", str(rust_source), "-o", str(work / "rust_expression_probe")],
    ]
    for argv in commands:
        command(argv)
    cases, main_count, intentional = matrix()
    reference = probe_results(work / "rack_registration_probe", cases)
    actual = probe_results(work / "rust_expression_probe", cases)
    assert len(cases) == len(reference) == len(actual)
    results = []
    for index, expression in enumerate(cases):
        expected, result = reference[index], actual[index]
        equal = equivalent(expected, result)
        corrected = expression in intentional and equivalent(str(intentional[expression]), result)
        status = "match" if equal else "intentional_difference" if corrected else "unexpected_difference"
        results.append({"expression": expression, "reference": expected, "rust": result, "status": status})

    def public_command(argv):
        return [part.replace(str(work), "<temporary-reference-dir>").replace(str(ROOT), ".") for part in argv]

    hashes = {name: entry["sha256"] for name, entry in sources.items()}
    for path in [c_source, template, Path(__file__).resolve(), module]:
        hashes[path.relative_to(ROOT).as_posix()] = sha256(path.read_bytes())
    return {
        "scope": "Isolated pinned tinyexpr with independently reconstructed Rack note/custom function registration versus original Rust evaluator. Does not execute the Rack application, audio processing, or UI.",
        "tinyexpr_commit": TINYEXPR_COMMIT,
        "rack_commit": RACK_COMMIT,
        "reproduce": "python3 scripts/verify_parameter_expressions.py --output expression-report.json",
        "compiler_commands": [public_command(argv) for argv in commands],
        "compiler_versions": {"clang": command(["clang", "--version"]).splitlines()[0], "rustc": command(["rustc", "--version"]).strip()},
        "reference_sources": sources,
        "source_sha256": hashes,
        "main_matrix_cases": main_count,
        "total_cases": len(cases),
        "tolerance": {"relative": 1e-12, "absolute": 1e-14},
        "comparison_policy": "Reference NaN or syntax failure corresponds to Rust error; matching infinities compare equal. Finite results compare at documented tolerance.",
        "intentional_difference_policy": "Rust preserves explicit negative parentheses and conventional exponent/unary-minus precedence.",
        "unexpected_differences": sum(r["status"] == "unexpected_difference" for r in results),
        "intentional_differences": sum(r["status"] == "intentional_difference" for r in results),
        "cases": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-dir", type=Path, help="Use local pinned sources instead of downloading")
    parser.add_argument("--output", type=Path, required=True, help="Write the full JSON differential report")
    args = parser.parse_args()
    try:
        with tempfile.TemporaryDirectory(prefix="openpatch-expression-reference-") as directory:
            report = verify(Path(directory), args.reference_dir)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(2, f"Verification failed: {error}\n")
    summary = {key: report[key] for key in ["main_matrix_cases", "total_cases", "unexpected_differences", "intentional_differences"]}
    print(json.dumps(summary, indent=2))
    return 1 if report["unexpected_differences"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
