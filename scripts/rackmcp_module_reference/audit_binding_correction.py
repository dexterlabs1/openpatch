#!/usr/bin/env python3
"""Audit the declared native correction without weakening the raw comparison.

The unchanged wrapper comparison must remain failed. Every differing word has
an explicit frame interval and source/native value; all other words must match.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bits(value):
    return struct.unpack("<I", struct.pack("<f", value))[0]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    root = args.reference_dir.resolve()
    report = json.loads((root / "report.json").read_text())
    manifest = json.loads((root / "manifest.json").read_text())
    declaration = json.loads((root / "declaration.json").read_text())
    assert report["passed"] is False, "retain the genuine module mismatch"
    assert report["manifest_sha256"] == digest(root / "manifest.json")
    assert manifest["declaration_sha256"] == digest(root / "declaration.json")
    for name, expected in manifest["fixtures"].items():
        assert digest(root / name) == expected, name
    assert declaration["rate"] == 48000
    assert len(declaration["cases"]) == 9
    # These intervals are fixed by the frozen source fixture, not inferred from
    # whichever native output happens to differ. Equal reload at129 writes at
    # the next32-divider tick159; a manual override at200 ends that difference.
    # Changed data6500 writes6527. Returned target defaults survive until an
    # explicit load256 writes287. SDK self-unbinding prevents those writes.
    # Obtain the normal .75 lane's genuine evaluated value from its initial
    # successful source write, before either implementation reloads data.
    with (root / "poly_internal.source.trace").open() as stream:
        for _ in range(32):
            initial = next(stream).split()
    filter_written = int(initial[307])
    windows = {
        "manual_cache_equal_reload": (159, 200, {306: (bits(.875), bits(.25))}),
        "changed_data": (6527, 8000, {306: (bits(.25), bits(.625))}),
        "written_target_disappear_return": (287, 512, {
            306: (bits(1.0), bits(.25)), 307: (bits(.5), filter_written)}),
    }
    results = []
    output_words = target_words = correction_words = frames = 0
    for case, count in declaration["cases"].items():
        corrected = 0
        with (root / f"{case}.source.trace").open() as source, (root / f"{case}.native.trace").open() as native:
            for frame in range(count):
                a, b = next(source).split(), next(native).split()
                assert len(a) == len(b) == 309, (case, frame)
                window = windows.get(case)
                expected = window[2] if window and window[0] <= frame < window[1] else {}
                for column, (left, right) in enumerate(zip(a, b)):
                    if column in expected:
                        assert (int(left), int(right)) == expected[column], (case, frame, column, left, right)
                        correction_words += 1
                        corrected += 1
                    else:
                        assert left == right, (case, frame, column, left, right)
                        if column < 306:
                            output_words += 1
                        elif column < 308:
                            target_words += 1
                frames += 1
            assert not source.readline() and not native.readline(), case
        results.append({"case": case, "frames": count, "declared_correction_words": corrected})
    assert frames == 25104 and correction_words == 1964
    # Preserve real SDK registry evidence of the defect separately from the
    # healthy native target contract. Neither diagnostic mutates source DSP.
    audits = [json.loads(line) for line in (root / "manual_cache_equal_reload.source.stderr.log").read_text().splitlines()]
    assert len(audits) == 3
    for audit in audits:
        assert audit["module_id_before"] == audit["module_id_after"] == 100
        assert audit["target_registered"] is True
    assert all(h["resolved_module_id"] == 42 for h in audits[0]["handles_after"])
    assert all(h["present"] is False for h in audits[1]["handles_after"])
    assert all(h["module_id"] == -1 for h in audits[1]["previous_handles_after"])
    assert all(h["resolved_module_id"] == 42 for h in audits[2]["handles_after"])
    result = {
        "raw_module_comparison_passed": False,
        "native_binding_correction_contract_passed": True,
        "reference_manifest_sha256": digest(root / "manifest.json"),
        "raw_report_sha256": digest(root / "report.json"),
        "audit_driver_sha256": digest(Path(__file__)),
        "frames": frames, "cases": results,
        "all_18_outputs_exact_words": output_words,
        "unaffected_target_words_exact": target_words,
        "explicit_source_and_native_correction_words": correction_words,
        "source_handle_self_unbind_observed": True,
        "limits": ["No full-module equivalence", "No competing-sequencer ownership qualification", "No source import qualification"],
    }
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(json.dumps({key: result[key] for key in ["raw_module_comparison_passed", "native_binding_correction_contract_passed", "frames", "explicit_source_and_native_correction_words"]}))


if __name__ == "__main__":
    main()
