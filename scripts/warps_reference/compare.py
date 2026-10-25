#!/usr/bin/env python3
"""Compare the production Rust Warps kernel against an immutable C++ reference run.

Acceptance is exact: every int16 main/aux output sample must be identical inside
the source's defined domain. If the Rust kernel reports a guarded read (a table
index that would leave its array, undefined behaviour in the C++ source), that
block and every later block of the case are outside the defined domain: they
are reported with their own mismatch counts and never counted as matched.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    reference = args.reference.resolve()
    assert json.loads((reference / 'result.json').read_text())['passed']
    for name, expected in json.loads((reference / 'artifact-sha256.json').read_text()).items():
        assert sha(reference / name) == expected, f'changed reference: {name}'
    cases = json.loads((reference / 'cases.json').read_text())['cases']
    base = ROOT / 'native-test-output/warps-reference'
    index = 1
    while (base / f'rust-comparison-{index:03}').exists():
        index += 1
    evidence = base / f'rust-comparison-{index:03}'
    evidence.mkdir(parents=True)
    sources = [HERE / 'rust_probe.rs', HERE / 'compare.py']
    sources += sorted((ROOT / 'crates/openpatch-core/src/cross_matter').rglob('*.rs'))
    source_hashes = {}
    for source in sources:
        relative = source.relative_to(ROOT)
        target = evidence / 'source' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        source_hashes[str(relative)] = sha(source)
    (evidence / 'source-sha256.json').write_text(json.dumps(source_hashes, indent=2) + '\n')
    (evidence / 'rustc.txt').write_text(subprocess.check_output(['rustc', '-Vv'], text=True, timeout=30))
    executable = evidence / 'rust_probe'
    command = ['rustc', '--edition=2021', '-O', str(evidence / 'source/scripts/warps_reference/rust_probe.rs'), '-o', str(executable)]
    (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    report = {'passed': False, 'reference': str(reference), 'reference_manifest_sha256': sha(reference / 'artifact-sha256.json'),
              'scope': 'portable warps::Modulator kernel at 96 kHz / 60-frame blocks, int16 codec frames; not the native host adapter, resampler or hardware',
              'acceptance': 'bit-identical int16 main and aux samples; zero guarded reads', 'cases': {}}
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        (evidence / 'compile.stderr').write_text(result.stderr)
        result.check_returncode()
        executable_hash = sha(executable)
        report['executable_sha256'] = executable_hash
        for case in cases:
            name = case['id']
            target = evidence / name
            target.mkdir()
            with (target / 'output.txt').open('w') as output, (target / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=(reference / name / 'input.txt').read_text(), text=True, stdout=output, stderr=errors, timeout=120)
            result.check_returncode()
            diagnostics = (target / 'stderr.txt').read_text().splitlines()
            guarded = int(diagnostics[-1].split()[1])
            guarded_blocks = [int(value) for value in diagnostics[-2].split()[1:]]
            domain_end = guarded_blocks[0] if guarded_blocks else case['blocks']
            expected = [list(map(int, line.split())) for line in (reference / name / 'output.txt').read_text().splitlines()]
            actual = [list(map(int, line.split())) for line in (target / 'output.txt').read_text().splitlines()]
            assert len(actual) == len(expected) == case['blocks']
            samples = mismatched = max_difference = 0
            outside_samples = outside_mismatched = outside_max = 0
            first = None
            for block, (a, b) in enumerate(zip(expected, actual)):
                assert len(a) == len(b) == 120
                for position, (x, y) in enumerate(zip(a, b)):
                    if block >= domain_end:
                        outside_samples += 1
                        if x != y:
                            outside_mismatched += 1
                            outside_max = max(outside_max, abs(x - y))
                        continue
                    samples += 1
                    if x != y:
                        mismatched += 1
                        max_difference = max(max_difference, abs(x - y))
                        if first is None:
                            first = {'block': block, 'frame': position // 2, 'output': 'aux' if position % 2 else 'main', 'expected': x, 'actual': y}
            report['cases'][name] = {'passed': mismatched == 0, 'samples': samples, 'mismatched_samples': mismatched,
                                     'max_abs_difference': max_difference, 'guarded_reads': guarded}
            if guarded_blocks:
                report['cases'][name]['source_undefined_domain'] = {'first_guarded_block': domain_end, 'guarded_blocks': guarded_blocks,
                                                                    'excluded_samples': outside_samples, 'mismatched_samples': outside_mismatched,
                                                                    'max_abs_difference': outside_max}
            if first:
                report['cases'][name]['first_mismatch'] = first
        report['passed'] = all(item['passed'] for item in report['cases'].values())
    except Exception as error:
        report['error'] = str(error)
    report['compared_samples'] = sum(item.get('samples', 0) for item in report['cases'].values())
    report['mismatched_samples'] = sum(item.get('mismatched_samples', 0) for item in report['cases'].values())
    report['excluded_undefined_samples'] = sum(item.get('source_undefined_domain', {}).get('excluded_samples', 0) for item in report['cases'].values())
    (evidence / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    artifacts = {str(path.relative_to(evidence)): sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases_passed': sum(item['passed'] for item in report['cases'].values()), 'cases': len(cases),
                      'compared_samples': report['compared_samples'], 'mismatched_samples': report['mismatched_samples'], 'excluded_undefined_samples': report['excluded_undefined_samples'], 'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
