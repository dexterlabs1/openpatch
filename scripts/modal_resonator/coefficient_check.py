#!/usr/bin/env python3
"""Check the actual Rust ZOH coefficient against a 65-digit Decimal oracle.

Extracts and compiles the production Coefficient type/implementation in a
temporary directory. The oracle independently evaluates the closed-form forced
oscillator using Decimal.exp and Taylor-series trigonometry. No audio devices
or GUI are opened. Run from any directory; rustc must be available.
"""
import argparse
from decimal import Decimal, localcontext
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/openpatch-core/src/modal_resonator.rs"


def block(source, marker):
    start = source.index(marker)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def sin_decimal(x):
    value = term = x
    for k in range(1, 80):
        term *= -x * x / Decimal((2 * k) * (2 * k + 1))
        value += term
    return value


def cos_decimal(x):
    value = term = Decimal(1)
    for k in range(1, 80):
        term *= -x * x / Decimal((2 * k - 1) * (2 * k))
        value += term
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/benchmarks/modal-coefficients.json")
    args = parser.parse_args()
    source_bytes = SOURCE.read_bytes()
    source = source_bytes.decode()
    extracted = block(source, "struct Coefficient {") + "\n" + block(source, "impl Coefficient {")
    rust = "use std::f64::consts::TAU;\n" + extracted + r'''
fn main() {
    for fs in [8000.0_f64, 44100.0, 48000.0, 96000.0, 192000.0] {
        for f in [20.0, 220.0, 0.45*fs] {
            for rt60 in [0.1_f64, 2.0, 30.0] {
                let sigma=std::f64::consts::LN_10*3.0/rt60;
                let h=1.0/fs;
                let w2=(TAU*f).powi(2);
                let c=Coefficient::new(f,sigma,h);
                println!("{fs},{f},{rt60},{sigma:.17e},{h:.17e},{w2:.17e},{:.17e}", c.force_q);
            }
        }
    }
}
'''
    rustc = shutil.which("rustc")
    if rustc is None:
        raise SystemExit("rustc not found")
    version = subprocess.check_output([rustc, "--version"], text=True).strip()
    with tempfile.TemporaryDirectory(prefix="openpatch-modal-coefficients-") as temporary:
        temporary = Path(temporary)
        probe = temporary / "probe.rs"
        binary = temporary / "probe"
        probe.write_text(rust)
        subprocess.run([rustc, "--edition=2021", "-Awarnings", "-O", str(probe), "-o", str(binary)], check=True)
        rows = subprocess.check_output([str(binary)], text=True).splitlines()
    cases = []
    with localcontext() as context:
        context.prec = 65
        for row in rows:
            rate, frequency, rt60, sigma_text, h_text, w2_text, actual_text = row.split(",")
            # Float conversion reconstructs the exact binary64 parameters used
            # by the compiled implementation, without decimal-input mismatch.
            sigma, h, w2, actual = [Decimal.from_float(float(v)) for v in (sigma_text, h_text, w2_text, actual_text)]
            d = (w2 - sigma * sigma).sqrt()
            x = d * h
            expected = (1 - (-sigma * h).exp() * (cos_decimal(x) + sigma * sin_decimal(x) / d)) / w2
            relative_error = abs(actual - expected) / abs(expected)
            cases.append({"sample_rate": int(float(rate)), "frequency_hz": float(frequency), "rt60_seconds": float(rt60),
                          "production_force_q": float(actual), "reference_force_q_decimal": str(expected),
                          "relative_error": float(relative_error)})
    maximum = max(case["relative_error"] for case in cases)
    result = {
        "description": "Actual extracted production Rust force_q coefficient versus independent high-precision closed-form oracle",
        "source": str(SOURCE.relative_to(ROOT)),
        "source_sha256": hashlib.sha256(source_bytes).hexdigest(),
        "extracted_source_sha256": hashlib.sha256(extracted.encode()).hexdigest(),
        "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "rustc": version,
        "oracle": "65-digit Decimal exp and 80-term independently summed sine/cosine Taylor series; exact binary64 input parameters",
        "scope": "45 static coefficient corners; verifies numerical coefficient evaluation, not full model, output audio, arbitrary modulation, or hardware fidelity",
        "relative_error_limit": 1e-11,
        "maximum_relative_error": maximum,
        "passed": maximum <= 1e-11,
        "cases": cases,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "cases": len(cases), "maximum_relative_error": maximum, "passed": result["passed"]}))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
